//! Отрисовка на GPU через wgpu (Metal / DirectX 12 / Vulkan / OpenGL).

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use winit::window::Window;

use crate::decor::{self, DECOR_MAX};
use crate::look::{self, FEATHER_PX, FlakeLook, SHADOW_PX, SHADOW_RGB};
use crate::sim::heap::{self, HEAP_MAX};
use crate::sim::{MAX_PARTICLES, Sim};
use crate::sky::{SKY_MAX, Sky};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    size: [f32; 2],
    scale: f32,
    premul: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Flake {
    center: [f32; 2],
    /// Полуразмеры вдоль и поперёк собственной оси, точки.
    half: [f32; 2],
    rotation: f32,
    /// Вид фигуры и её параметр (см. `FlakeShape::code`).
    shape: f32,
    param: f32,
    color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PileVertex {
    pos: [f32; 2],
    color: [f32; 4],
}

const PILE_ATTRS: [wgpu::VertexAttribute; 2] = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4];

/// Сколько окон небо вырезает за кадр.
const MAX_CUTS: usize = 256;

pub struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    premul: bool,
    globals: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    flake_pipeline: wgpu::RenderPipeline,
    pile_pipeline: wgpu::RenderPipeline,
    /// Вырезает прозрачные дыры по рамкам окон: заменяет цвет, а не смешивает.
    cut_pipeline: wgpu::RenderPipeline,
    cut_buf: wgpu::Buffer,
    cut: Vec<PileVertex>,
    flake_buf: wgpu::Buffer,
    pile_buf: wgpu::Buffer,
    pile_cap: u64,
    flakes: Vec<Flake>,
    looks: Vec<FlakeLook>,
    pile: Vec<PileVertex>,
    /// Подробное описание выбранного адаптера — для журнала и диагностики.
    pub info: String,
    /// Короткое название адаптера — для меню.
    pub adapter: String,
    /// Отрисовка идёт программно, на процессоре.
    pub software: bool,
}

impl Renderer {
    /// Есть ли в системе программный адаптер, на который можно переключиться.
    /// На macOS его нет: Metal работает только на GPU.
    pub fn software_available() -> bool {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all()))
            .iter()
            .any(|a| a.get_info().device_type == wgpu::DeviceType::Cpu)
    }

    /// `prefer_gpu` — сначала пробовать видеокарту (иначе — программный адаптер).
    /// Если предпочтительного варианта нет, берётся другой.
    pub fn new(
        window: Arc<Window>,
        display: winit::event_loop::OwnedDisplayHandle,
        prefer_gpu: bool,
    ) -> Result<Self, String> {
        let size = window.inner_size();

        // На Windows прозрачное окно умеет только DirectX 12 с цепочкой кадров через
        // DirectComposition, поэтому сначала пробуем его, а остальные бэкенды — запасные.
        let attempts: &[wgpu::Backends] =
            if cfg!(target_os = "windows") { &[wgpu::Backends::DX12, wgpu::Backends::all()] } else { &[wgpu::Backends::all()] };

        let mut found = None;
        let mut picked = None;
        let mut last_err = String::from("нет подходящего графического адаптера");
        for &backends in attempts {
            let mut desc = wgpu::InstanceDescriptor::new_with_display_handle(Box::new(display.clone()));
            desc.backends = backends;
            desc.backend_options.dx12.presentation_system = wgpu::Dx12SwapchainKind::DxgiFromVisual;
            let instance = wgpu::Instance::new(desc);
            let surface = match instance.create_surface(window.clone()) {
                Ok(s) => s,
                Err(e) => {
                    last_err = format!("не удалось создать поверхность: {e}");
                    continue;
                }
            };
            // Осадкам хватает встроенной графики; если GPU нет вовсе — программный адаптер.
            for fallback in [!prefer_gpu, prefer_gpu] {
                let opts = wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::LowPower,
                    force_fallback_adapter: fallback,
                    compatible_surface: Some(&surface),
                    ..Default::default()
                };
                match pollster::block_on(instance.request_adapter(&opts)) {
                    Ok(adapter) => {
                        picked = Some(adapter);
                        break;
                    }
                    Err(e) => last_err = format!("адаптер не найден: {e}"),
                }
            }
            if let Some(adapter) = picked.take() {
                found = Some((surface, adapter));
                break;
            }
        }
        let Some((surface, adapter)) = found else { return Err(last_err) };

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("snowdrift"),
            required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
            ..Default::default()
        }))
        .map_err(|e| format!("не удалось открыть устройство: {e}"))?;

        let caps = surface.get_capabilities(&adapter);
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .ok_or("поверхность несовместима с адаптером")?;
        // Цвет без sRGB-кодирования: смешивание белого снега выглядит одинаково везде.
        config.format = caps.formats.iter().copied().find(|f| !f.is_srgb()).unwrap_or(caps.formats[0]);
        config.present_mode = wgpu::PresentMode::Fifo; // кадры по вертикальной синхронизации
        use wgpu::CompositeAlphaMode as A;
        config.alpha_mode = [A::PreMultiplied, A::PostMultiplied, A::Inherit]
            .into_iter()
            .find(|m| caps.alpha_modes.contains(m))
            .unwrap_or(caps.alpha_modes[0]);
        // Непрозрачная поверхность закрыла бы собой весь экран — лучше не запускаться вовсе.
        if config.alpha_mode == A::Opaque && std::env::var_os("SNOWDRIFT_ALLOW_OPAQUE").is_none() {
            return Err(format!(
                "графическая система не даёт прозрачное окно (доступно: {:?}); \
                 в Linux нужен композитор",
                caps.alpha_modes
            ));
        }
        // macOS: wgpu называет режим PostMultiplied, но Core Animation на деле ждёт цвет,
        // уже умноженный на альфу. С «прямой» альфой прозрачный белый фон заливает экран.
        let premul = config.alpha_mode != A::PostMultiplied || cfg!(target_os = "macos");
        surface.configure(&device, &config);

        let ai = adapter.get_info();
        let software = ai.device_type == wgpu::DeviceType::Cpu;
        let short = format!("{} · {:?}", ai.name, ai.backend);
        let info = format!(
            "{} ({:?}, {:?}{}), формат {:?}, альфа {:?}",
            ai.name,
            ai.backend,
            ai.device_type,
            if software { ", программный" } else { "" },
            config.format,
            config.alpha_mode
        );

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("snow"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });

        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globals"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals"),
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: globals.as_entire_binding() }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("snow"),
            bind_group_layouts: &[Some(&bgl)],
            ..Default::default()
        });

        let blend =
            if premul { wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING } else { wgpu::BlendState::ALPHA_BLENDING };
        let targets = [Some(wgpu::ColorTargetState {
            format: config.format,
            blend: Some(blend),
            write_mask: wgpu::ColorWrites::ALL,
        })];
        // вырез: поверх уже нарисованного кладётся прозрачный цвет, без смешивания
        let cut_targets = [Some(wgpu::ColorTargetState {
            format: config.format,
            blend: Some(wgpu::BlendState::REPLACE),
            write_mask: wgpu::ColorWrites::ALL,
        })];
        let pipeline = |label, vs, fs, buffer: wgpu::VertexBufferLayout, targets: &[Option<wgpu::ColorTargetState>]| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vs),
                    compilation_options: Default::default(),
                    buffers: &[Some(buffer)],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets,
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let flake_pipeline = pipeline(
            "flakes",
            "vs_flake",
            "fs_flake",
            wgpu::VertexBufferLayout {
                array_stride: size_of::<Flake>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x3, 3 => Float32x4],
            },
            &targets,
        );
        let pile_layout = || wgpu::VertexBufferLayout {
            array_stride: size_of::<PileVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &PILE_ATTRS,
        };
        let pile_pipeline = pipeline("piles", "vs_pile", "fs_pile", pile_layout(), &targets);
        let cut_pipeline = pipeline("cuts", "vs_pile", "fs_pile", pile_layout(), &cut_targets);
        let cut_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cuts"),
            size: (MAX_CUTS * 6 * size_of::<PileVertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let flake_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("flakes"),
            size: ((MAX_PARTICLES + SKY_MAX + DECOR_MAX + HEAP_MAX) * size_of::<Flake>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let pile_cap = 64 * 1024;
        let pile_buf = Self::pile_buffer(&device, pile_cap);

        Ok(Self {
            surface,
            device,
            queue,
            config,
            premul,
            globals,
            bind_group,
            flake_pipeline,
            pile_pipeline,
            cut_pipeline,
            cut_buf,
            cut: Vec::new(),
            flake_buf,
            pile_buf,
            pile_cap,
            flakes: Vec::with_capacity(MAX_PARTICLES + SKY_MAX + DECOR_MAX + HEAP_MAX),
            looks: Vec::with_capacity(MAX_PARTICLES + SKY_MAX + DECOR_MAX + HEAP_MAX),
            pile: Vec::new(),
            info,
            adapter: short,
            software,
        })
    }

    fn pile_buffer(device: &wgpu::Device, size: u64) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("piles"),
            size,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
    }

    /// Строит сетку куч: тень над кромкой, сглаживающая полоска, тело.
    fn build_piles(&mut self, sim: &Sim, scale: f32) {
        self.pile.clear();
        let px = 1.0 / scale;
        let clear = |c: [f32; 4]| [c[0], c[1], c[2], 0.0];
        let pile = &mut self.pile;
        type V = ([f32; 2], [f32; 4]);
        let mut quad = |a: V, b: V, c: V, d: V| {
            for (pos, color) in [a, b, c, c, b, d] {
                pile.push(PileVertex { pos, color });
            }
        };
        look::pile_runs(sim, |run, base, shadow_alpha| {
            let shadow = [SHADOW_RGB[0], SHADOW_RGB[1], SHADOW_RGB[2], shadow_alpha];
            for pair in run.windows(2) {
                let (p, q) = (pair[0], pair[1]);
                if shadow_alpha > 0.0 {
                    // мягкая тень над кучей — чтобы она читалась на светлом фоне
                    quad(
                        ([p.x, p.top - SHADOW_PX * px], clear(shadow)),
                        ([q.x, q.top - SHADOW_PX * px], clear(shadow)),
                        ([p.x, p.top], shadow),
                        ([q.x, q.top], shadow),
                    );
                }
                // сглаживание верхней кромки
                quad(
                    ([p.x, p.top - FEATHER_PX * px], clear(p.top_color)),
                    ([q.x, q.top - FEATHER_PX * px], clear(q.top_color)),
                    ([p.x, p.top], p.top_color),
                    ([q.x, q.top], q.top_color),
                );
                // тело кучи
                quad(([p.x, p.top], p.top_color), ([q.x, q.top], q.top_color), ([p.x, base], p.base_color), ([q.x, base], q.base_color));
            }
        });
    }

    /// Рисует кадр. Возвращает `false`, если кадр получить не удалось (окно скрыто и т. п.).
    pub fn render(&mut self, sim: &Sim, sky: &Sky, scale: f32) -> bool {
        use wgpu::CurrentSurfaceTexture as T;
        let frame = match self.surface.get_current_texture() {
            T::Success(f) | T::Suboptimal(f) => f,
            T::Outdated | T::Lost => {
                self.surface.configure(&self.device, &self.config);
                return false;
            }
            _ => return false,
        };

        // небо рисуется под частицами и за окнами
        let haze = sim.haze();
        self.looks.clear();
        sky.looks(sim.w, sim.h, &mut self.looks);
        let sky_n = self.looks.len();
        self.cut.clear();
        // вырезы нужны и небу, и дымке: за окнами их не должно быть
        if sky_n > 0 || haze[3] > 0.0 {
            for r in sim.window_rects().iter().take(MAX_CUTS) {
                let (x0, y0, x1, y1) = (r.min_x(), r.min_y(), r.max_x(), r.max_y());
                let v = |x, y| PileVertex { pos: [x, y], color: [0.0; 4] };
                self.cut.extend([v(x0, y0), v(x1, y0), v(x0, y1), v(x0, y1), v(x1, y0), v(x1, y1)]);
            }
        }
        // украшения окон — перед частицами, после выреза по окнам
        decor::looks(sim, &mut self.looks);
        // шары и кучи, собранные курсором
        heap::looks(sim, &mut self.looks);
        self.looks.extend(sim.particles.iter().take(MAX_PARTICLES).map(look::flake));
        self.flakes.clear();
        self.flakes.extend(self.looks.iter().map(|f| {
            let (shape, param) = f.shape.code();
            Flake { center: f.center, half: f.half, rotation: f.rotation, shape, param, color: f.color }
        }));
        self.build_piles(sim, scale);

        let globals =
            Globals { size: [sim.w.max(1.0), sim.h.max(1.0)], scale, premul: if self.premul { 1.0 } else { 0.0 } };
        self.queue.write_buffer(&self.globals, 0, bytemuck::bytes_of(&globals));
        if !self.flakes.is_empty() {
            self.queue.write_buffer(&self.flake_buf, 0, bytemuck::cast_slice(&self.flakes));
        }
        if !self.cut.is_empty() {
            self.queue.write_buffer(&self.cut_buf, 0, bytemuck::cast_slice(&self.cut));
        }
        let pile_bytes = (self.pile.len() * size_of::<PileVertex>()) as u64;
        if pile_bytes > self.pile_cap {
            self.pile_cap = pile_bytes.next_power_of_two();
            self.pile_buf = Self::pile_buffer(&self.device, self.pile_cap);
        }
        if pile_bytes > 0 {
            self.queue.write_buffer(&self.pile_buf, 0, bytemuck::cast_slice(&self.pile));
        }

        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            // Фон — дымка (у большинства материалов её нет, и он прозрачный). Для «прямой»
            // альфы цвет фона белый, чтобы края не темнели.
            let k = if self.premul { haze[3] } else { 1.0 };
            let c = if haze[3] > 0.0 {
                wgpu::Color { r: (haze[0] * k) as f64, g: (haze[1] * k) as f64, b: (haze[2] * k) as f64, a: haze[3] as f64 }
            } else {
                let w = if self.premul { 0.0 } else { 1.0 };
                wgpu::Color { r: w, g: w, b: w, a: 0.0 }
            };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("snow"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(c),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_bind_group(0, &self.bind_group, &[]);
            // порядок: небо, вырезы по окнам, частицы
            pass.set_pipeline(&self.flake_pipeline);
            pass.set_vertex_buffer(0, self.flake_buf.slice(..));
            if sky_n > 0 {
                pass.draw(0..6, 0..sky_n as u32);
            }
            if !self.cut.is_empty() {
                pass.set_pipeline(&self.cut_pipeline);
                pass.set_vertex_buffer(0, self.cut_buf.slice(0..(self.cut.len() * size_of::<PileVertex>()) as u64));
                pass.draw(0..self.cut.len() as u32, 0..1);
                pass.set_pipeline(&self.flake_pipeline);
                pass.set_vertex_buffer(0, self.flake_buf.slice(..));
            }
            if self.flakes.len() > sky_n {
                pass.draw(0..6, sky_n as u32..self.flakes.len() as u32);
            }
            if pile_bytes > 0 {
                pass.set_pipeline(&self.pile_pipeline);
                pass.set_vertex_buffer(0, self.pile_buf.slice(0..pile_bytes));
                pass.draw(0..self.pile.len() as u32, 0..1);
            }
        }
        self.queue.submit([encoder.finish()]);
        self.queue.present(frame);
        true
    }
}
