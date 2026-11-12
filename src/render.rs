//! Отрисовка на GPU через wgpu (Metal / DirectX 12 / Vulkan / OpenGL).

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use winit::window::Window;

use crate::sim::{COL_W, MAX_PARTICLES, Sim};

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
    radius: f32,
    alpha: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PileVertex {
    pos: [f32; 2],
    color: [f32; 4],
}

/// Прозрачность слоёв снежинок: дальний, средний, ближний.
const LAYER_ALPHA: [f32; 3] = [0.45, 0.7, 0.95];
const PILE_TOP: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
const PILE_BASE: [f32; 4] = [0.84, 0.90, 1.0, 1.0];
const SHADOW: [f32; 4] = [0.1, 0.2, 0.4, 0.28];

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
    flake_buf: wgpu::Buffer,
    pile_buf: wgpu::Buffer,
    pile_cap: u64,
    flakes: Vec<Flake>,
    pile: Vec<PileVertex>,
    /// Описание выбранного GPU — для журнала и диагностики.
    pub info: String,
}

impl Renderer {
    pub fn new(window: Arc<Window>, display: winit::event_loop::OwnedDisplayHandle) -> Result<Self, String> {
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
            // Снегу хватает встроенной графики; если GPU нет вовсе — программный адаптер.
            for fallback in [false, true] {
                let opts = wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::LowPower,
                    force_fallback_adapter: fallback,
                    compatible_surface: Some(&surface),
                    ..Default::default()
                };
                match pollster::block_on(instance.request_adapter(&opts)) {
                    Ok(adapter) => {
                        picked = Some((adapter, fallback));
                        break;
                    }
                    Err(e) => last_err = format!("адаптер не найден: {e}"),
                }
            }
            if let Some((adapter, fallback)) = picked.take() {
                found = Some((surface, adapter, fallback));
                break;
            }
        }
        let Some((surface, adapter, fallback)) = found else { return Err(last_err) };

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
        let info = format!(
            "{} ({:?}, {:?}{}), формат {:?}, альфа {:?}",
            ai.name,
            ai.backend,
            ai.device_type,
            if fallback { ", программный" } else { "" },
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
        let pipeline = |label, vs, fs, buffer: wgpu::VertexBufferLayout| {
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
                    targets: &targets,
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
                attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2],
            },
        );
        let pile_pipeline = pipeline(
            "piles",
            "vs_pile",
            "fs_pile",
            wgpu::VertexBufferLayout {
                array_stride: size_of::<PileVertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4],
            },
        );

        let flake_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("flakes"),
            size: (MAX_PARTICLES * size_of::<Flake>()) as u64,
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
            flake_buf,
            pile_buf,
            pile_cap,
            flakes: Vec::with_capacity(MAX_PARTICLES),
            pile: Vec::new(),
            info,
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

    /// Строит сетку сугробов: тень над кромкой, тело, сглаживающая полоска.
    fn build_piles(&mut self, sim: &Sim, scale: f32) {
        self.pile.clear();
        let px = 1.0 / scale;
        let clear = |c: [f32; 4]| [c[0], c[1], c[2], 0.0];
        let mut quad = |a: ([f32; 2], [f32; 4]), b: ([f32; 2], [f32; 4]), c: ([f32; 2], [f32; 4]), d: ([f32; 2], [f32; 4])| {
            for (pos, color) in [a, b, c, c, b, d] {
                self.pile.push(PileVertex { pos, color });
            }
        };
        for s in sim.surfaces.values() {
            let n = s.h.len();
            let base = s.y + 0.5;
            let mut i = 0;
            while i < n {
                if !(s.vis[i] && s.h[i] > 0.15) {
                    i += 1;
                    continue;
                }
                let mut j = i;
                while j < n && s.vis[j] && s.h[j] > 0.15 {
                    j += 1;
                }
                // контур участка: (x, верх сугроба); на концах высота сходит к нулю
                let mut prev = (s.x0 + i as f32 * COL_W, s.y);
                for k in i..=j {
                    let cur = if k == j {
                        (s.x0 + j as f32 * COL_W, s.y)
                    } else {
                        let l = if k > i { s.h[k - 1] } else { 0.0 };
                        let r = if k + 1 < j { s.h[k + 1] } else { 0.0 };
                        (s.x0 + (k as f32 + 0.5) * COL_W, s.y - (l + 2.0 * s.h[k] + r) / 4.0)
                    };
                    let (x0, t0) = prev;
                    let (x1, t1) = cur;
                    // мягкая тень над снегом — чтобы он читался на светлом фоне
                    quad(
                        ([x0, t0 - 3.5 * px], clear(SHADOW)),
                        ([x1, t1 - 3.5 * px], clear(SHADOW)),
                        ([x0, t0], SHADOW),
                        ([x1, t1], SHADOW),
                    );
                    // сглаживание верхней кромки
                    quad(
                        ([x0, t0 - 1.5 * px], clear(PILE_TOP)),
                        ([x1, t1 - 1.5 * px], clear(PILE_TOP)),
                        ([x0, t0], PILE_TOP),
                        ([x1, t1], PILE_TOP),
                    );
                    // тело сугроба
                    quad(([x0, t0], PILE_TOP), ([x1, t1], PILE_TOP), ([x0, base], PILE_BASE), ([x1, base], PILE_BASE));
                    prev = cur;
                }
                i = j;
            }
        }
    }

    /// Рисует кадр. Возвращает `false`, если кадр получить не удалось (окно скрыто и т. п.).
    pub fn render(&mut self, sim: &Sim, scale: f32) -> bool {
        use wgpu::CurrentSurfaceTexture as T;
        let frame = match self.surface.get_current_texture() {
            T::Success(f) | T::Suboptimal(f) => f,
            T::Outdated | T::Lost => {
                self.surface.configure(&self.device, &self.config);
                return false;
            }
            _ => return false,
        };

        self.flakes.clear();
        self.flakes.extend(sim.particles.iter().take(MAX_PARTICLES).map(|p| Flake {
            center: [p.x, p.y],
            radius: p.r,
            alpha: LAYER_ALPHA[p.layer.min(2) as usize],
        }));
        self.build_piles(sim, scale);

        let globals =
            Globals { size: [sim.w.max(1.0), sim.h.max(1.0)], scale, premul: if self.premul { 1.0 } else { 0.0 } };
        self.queue.write_buffer(&self.globals, 0, bytemuck::bytes_of(&globals));
        if !self.flakes.is_empty() {
            self.queue.write_buffer(&self.flake_buf, 0, bytemuck::cast_slice(&self.flakes));
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
            // Для «прямой» альфы фон — прозрачный белый, чтобы края снега не темнели.
            let c = if self.premul { 0.0 } else { 1.0 };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("snow"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: c, g: c, b: c, a: 0.0 }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_bind_group(0, &self.bind_group, &[]);
            if !self.flakes.is_empty() {
                pass.set_pipeline(&self.flake_pipeline);
                pass.set_vertex_buffer(0, self.flake_buf.slice(..));
                pass.draw(0..6, 0..self.flakes.len() as u32);
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
