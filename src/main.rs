//! Сугроб — снег, дождь, песок и листья поверх рабочего стола.
//!
//! Прозрачное окно на весь основной экран, пропускающее клики. Физика — в `sim`,
//! отрисовка на GPU — в `render`, всё зависящее от ОС — в `platform` и `tray`.

// В релизной сборке под Windows не открываем окно консоли.
#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]

mod clock;
mod decor;
mod gfx;
mod look;
mod materials;
mod platform;
mod render;
mod settings;
mod sim;
mod sky;
mod soft;
mod tray;

use std::sync::Arc;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::window::{Window, WindowId, WindowLevel};

use platform::Platform;
use gfx::Gfx;
use settings::{RATES, Settings};
use sim::{Rect, Sim, WindowInfo};
use sky::Sky;
use tray::{Cmd, MenuState, Tray};

/// Как часто опрашивается список окон, секунды.
const POLL_WINDOWS: f32 = 0.05;
/// Как часто проверяются Dock, размер экрана и разрешения, секунды.
const POLL_SLOW: f32 = 2.0;

struct Overlay {
    window: Arc<Window>,
    gfx: Gfx,
    /// Левый верхний угол окна в физических пикселях рабочего стола.
    origin: (f32, f32),
    scale: f32,
}

impl Overlay {
    /// Переводит точку из глобальных координат ОС в координаты симуляции.
    fn point(&self, x: f32, y: f32) -> (f32, f32) {
        if platform::PHYSICAL {
            ((x - self.origin.0) / self.scale, (y - self.origin.1) / self.scale)
        } else {
            (x - self.origin.0 / self.scale, y - self.origin.1 / self.scale)
        }
    }

    fn rect(&self, r: [f32; 4]) -> Rect {
        let (x, y) = self.point(r[0], r[1]);
        let k = if platform::PHYSICAL { 1.0 / self.scale } else { 1.0 };
        Rect::new(x, y, r[2] * k, r[3] * k)
    }
}

struct App {
    proxy: EventLoopProxy<Cmd>,
    settings: Settings,
    sim: Sim,
    sky: Sky,
    platform: Platform,
    overlay: Option<Overlay>,
    tray: Option<Tray>,
    menu: MenuState,
    /// Есть ли программный адаптер, на который можно переключиться.
    software_available: bool,
    running: bool,
    last_frame: Option<Instant>,
    last_mouse: Option<(f32, f32)>,
    mouse_vel: (f32, f32),
    poll_acc: f32,
    slow_acc: f32,
    dock: Option<Rect>,
}

/// Положение и размер окна-оверлея: весь основной монитор.
fn overlay_geometry(el: &ActiveEventLoop) -> (PhysicalPosition<i32>, PhysicalSize<u32>) {
    let monitor = el.primary_monitor().or_else(|| el.available_monitors().next());
    let (pos, mut size) = match monitor {
        Some(m) => (m.position(), m.size()),
        None => (PhysicalPosition::new(0, 0), PhysicalSize::new(1280, 720)),
    };
    // Окно ровно во весь экран Windows считает полноэкранным приложением
    // (прячет панель задач, глушит уведомления) — делаем его на пиксель ниже.
    if cfg!(target_os = "windows") {
        size.height = size.height.saturating_sub(1).max(1);
    }
    (pos, size)
}

impl App {
    fn menu_state(&self) -> MenuState {
        MenuState {
            running: self.running,
            scene: self.settings.scene,
            sky: self.settings.sky,
            sky_place: self.settings.sky_place,
            wind: self.settings.wind,
            wind_force: self.settings.wind_force,
            rate: self.settings.rate,
            solid: self.settings.solid,
            decor: self.settings.decor,
            dock: self.platform.dock_access(),
            gpu: self.overlay.as_ref().map_or(self.settings.gpu, |o| !o.gfx.software()),
            gpu_switchable: self.software_available,
            adapter: self.overlay.as_ref().map_or_else(String::new, |o| o.gfx.adapter()),
        }
    }

    fn sync_menu(&mut self) {
        let state = self.menu_state();
        if state != self.menu {
            self.menu = state;
            if let Some(tray) = &self.tray {
                tray.sync(&self.menu);
            }
        }
    }

    fn create_overlay(&mut self, el: &ActiveEventLoop) -> Result<Overlay, String> {
        let (pos, size) = overlay_geometry(el);
        let attrs = Window::default_attributes()
            .with_title("Сугроб")
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(false)
            .with_active(false)
            .with_window_level(WindowLevel::AlwaysOnTop)
            .with_position(pos)
            .with_inner_size(size);
        let window = el
            .create_window(platform::window_attributes(attrs))
            .map_err(|e| format!("не удалось создать окно: {e}"))?;
        let window = Arc::new(window);
        // клики и перетаскивания проходят сквозь снег
        if let Err(e) = window.set_cursor_hittest(false) {
            eprintln!("сугроб: окно не удалось сделать прозрачным для мыши: {e}");
        }
        platform::configure_window(&window);

        let gfx = Gfx::new(window.clone(), el.owned_display_handle(), self.settings.gpu)?;
        eprintln!("сугроб: отрисовка — {}", gfx.info());
        if !self.software_available {
            eprintln!("сугроб: программной отрисовки в системе нет — переключатель в меню отключён");
        }
        let mut overlay = Overlay { window, gfx, origin: (pos.x as f32, pos.y as f32), scale: 1.0 };
        self.apply_size(&mut overlay);
        Ok(overlay)
    }

    /// Обновляет масштаб, размер поверхности и размеры мира симуляции.
    fn apply_size(&mut self, overlay: &mut Overlay) {
        let size = overlay.window.inner_size();
        overlay.scale = overlay.window.scale_factor() as f32;
        overlay.gfx.resize(size.width, size.height);
        self.sim.w = size.width as f32 / overlay.scale;
        self.sim.h = size.height as f32 / overlay.scale;
    }

    /// Пересоздаёт отрисовку после переключения аппаратного ускорения.
    fn rebuild_renderer(&mut self, el: &ActiveEventLoop) {
        let Some(Overlay { window, gfx, origin, scale }) = self.overlay.take() else { return };
        // старую поверхность нужно освободить до создания новой на том же окне
        drop(gfx);
        let display = el.owned_display_handle();
        let gfx = Gfx::new(window.clone(), display.clone(), self.settings.gpu).or_else(|e| {
            eprintln!("сугроб: не удалось переключить отрисовку ({e}), возвращаю прежнюю");
            self.settings.gpu = !self.settings.gpu;
            Gfx::new(window.clone(), display, self.settings.gpu)
        });
        match gfx {
            Ok(gfx) => {
                eprintln!("сугроб: отрисовка — {}", gfx.info());
                let mut overlay = Overlay { window, gfx, origin, scale };
                self.apply_size(&mut overlay);
                overlay.window.request_redraw();
                self.overlay = Some(overlay);
            }
            Err(e) => {
                eprintln!("сугроб: {e}");
                el.exit();
            }
        }
    }

    /// Редкие проверки: Dock, смена разрешения экрана, галочки в меню.
    fn slow_poll(&mut self, el: &ActiveEventLoop) {
        self.sky.set_clock(clock::now());
        let Some(overlay) = &mut self.overlay else { return };
        let (pos, size) = overlay_geometry(el);
        if overlay.window.inner_size() != size || overlay.origin != (pos.x as f32, pos.y as f32) {
            overlay.origin = (pos.x as f32, pos.y as f32);
            overlay.window.set_outer_position(pos);
            let _ = overlay.window.request_inner_size(size);
        }
        // Dock или панель задач годятся, только если лежат вдоль нижнего края экрана
        self.dock = self.platform.dock().map(|r| overlay.rect(r)).and_then(|r| {
            let bottom = r.w > r.h && r.min_y() < self.sim.h - 10.0 && r.max_y() > self.sim.h - 150.0;
            bottom.then(|| Rect::new(r.x, r.y - platform::DOCK_LIFT, r.w, r.h + platform::DOCK_LIFT))
        });
        // в режиме «по времени года» материал меняется вместе с календарём
        self.sim.set_kind(self.settings.kind());
        self.sync_menu();
    }

    fn frame(&mut self, el: &ActiveEventLoop) {
        let now = Instant::now();
        let dt = self.last_frame.map_or(1.0 / 60.0, |t| (now - t).as_secs_f32()).clamp(0.001, 0.05);
        self.last_frame = Some(now);

        self.slow_acc += dt;
        if self.slow_acc >= POLL_SLOW {
            self.slow_acc = 0.0;
            self.slow_poll(el);
        }
        self.sky.step(dt);
        let Some(overlay) = &mut self.overlay else { return };

        // курсор и его сглаженная скорость
        let cursor = self.platform.cursor().map(|(x, y)| overlay.point(x, y)).or(self.last_mouse);
        let c = cursor.unwrap_or((-1000.0, -1000.0));
        if let Some(prev) = self.last_mouse {
            let v = (((c.0 - prev.0) / dt).clamp(-1500.0, 1500.0), ((c.1 - prev.1) / dt).clamp(-1500.0, 1500.0));
            self.mouse_vel.0 += (v.0 - self.mouse_vel.0) * 0.4;
            self.mouse_vel.1 += (v.1 - self.mouse_vel.1) * 0.4;
        }
        self.last_mouse = cursor;

        self.poll_acc += dt;
        if self.poll_acc >= POLL_WINDOWS {
            let windows: Vec<WindowInfo> = self
                .platform
                .windows()
                .into_iter()
                .map(|w| WindowInfo { id: w.id, rect: overlay.rect(w.rect) })
                .collect();
            self.sim.refresh(&windows, self.dock, self.poll_acc);
            self.poll_acc = 0.0;
        }

        self.sim.step(dt, c, self.mouse_vel);
        if !overlay.gfx.render(&self.sim, &self.sky, overlay.scale) {
            // кадр недоступен (окно скрыто системой) — не крутим цикл вхолостую
            std::thread::sleep(Duration::from_millis(16));
        }
        if self.running {
            overlay.window.request_redraw();
        }
    }

    fn command(&mut self, el: &ActiveEventLoop, cmd: Cmd) {
        match cmd {
            Cmd::Toggle => {
                self.running = !self.running;
                if let Some(o) = &self.overlay {
                    o.window.set_visible(self.running);
                    if self.running {
                        self.last_frame = None;
                        self.last_mouse = None;
                        o.window.request_redraw();
                    }
                }
            }
            Cmd::Scene(i) => {
                self.settings.scene = i.min(settings::SCENES - 1);
                self.sim.set_kind(self.settings.kind());
                self.settings.save();
            }
            Cmd::Sky(i) => {
                self.settings.sky = i.min(settings::SKIES - 1);
                self.sky.set_mode(self.settings.sky_mode());
                self.settings.save();
            }
            Cmd::SkyPlace(i) => {
                self.settings.sky_place = i.min(settings::PLACE_COUNT - 1);
                self.sky.set_place(self.settings.sky_place_value());
                self.settings.save();
            }
            Cmd::Wind(i) => {
                self.settings.wind = i.min(settings::WIND_DIRS.len() - 1);
                self.sim.wind_dir = self.settings.wind_dir();
                self.settings.save();
            }
            Cmd::WindForce(i) => {
                self.settings.wind_force = i.min(settings::WIND_FORCES.len() - 1);
                self.sim.wind_force = self.settings.wind_force_value();
                self.settings.save();
            }
            Cmd::Rate(i) => {
                self.settings.rate = i.min(RATES.len() - 1);
                self.sim.rate = RATES[self.settings.rate].1;
                self.settings.save();
            }
            Cmd::Solid => {
                self.settings.solid = !self.settings.solid;
                self.sim.solid = self.settings.solid;
                self.settings.save();
            }
            Cmd::Decor => {
                self.settings.decor = !self.settings.decor;
                self.sim.decor = self.settings.decor;
                self.settings.save();
            }
            Cmd::Shake => self.sim.dump_all(),
            Cmd::Gpu => {
                if self.software_available {
                    self.settings.gpu = !self.settings.gpu;
                    self.rebuild_renderer(el);
                    self.settings.save();
                }
            }
            Cmd::Dock => self.platform.request_dock_access(),
            Cmd::Quit => el.exit(),
        }
        // галочки ставим сами: на разных ОС меню ведёт себя по-разному
        self.menu = self.menu_state();
        if let Some(tray) = &self.tray {
            tray.sync(&self.menu);
        }
    }
}

impl ApplicationHandler<Cmd> for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.overlay.is_some() {
            return;
        }
        match self.create_overlay(el) {
            Ok(overlay) => {
                overlay.window.request_redraw();
                self.overlay = Some(overlay);
            }
            Err(e) => {
                eprintln!("сугроб: {e}");
                el.exit();
                return;
            }
        }
        self.menu = self.menu_state();
        match Tray::new(self.proxy.clone(), &self.menu) {
            Ok(tray) => self.tray = Some(tray),
            Err(e) => eprintln!("сугроб: значок в трее недоступен ({e}); завершить приложение можно через диспетчер задач"),
        }
        self.slow_poll(el);
    }

    fn user_event(&mut self, el: &ActiveEventLoop, cmd: Cmd) {
        self.command(el, cmd);
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::RedrawRequested => {
                if self.running {
                    self.frame(el);
                }
            }
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                if let Some(mut overlay) = self.overlay.take() {
                    self.apply_size(&mut overlay);
                    self.overlay = Some(overlay);
                }
            }
            WindowEvent::CloseRequested => el.exit(),
            _ => {}
        }
    }
}

fn main() {
    let platform = match Platform::new() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("сугроб: {e}");
            std::process::exit(1);
        }
    };
    let settings = Settings::load();
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0x5EED, |d| d.as_nanos() as u64);
    let mut sim = Sim::new(seed);
    sim.rate = RATES[settings.rate].1;
    sim.solid = settings.solid;
    sim.decor = settings.decor;
    sim.wind_dir = settings.wind_dir();
    sim.wind_force = settings.wind_force_value();
    sim.set_kind(settings.kind());

    let mut builder = EventLoop::<Cmd>::with_user_event();
    platform::configure_event_loop(&mut builder);
    let event_loop = match builder.build() {
        Ok(el) => el,
        Err(e) => {
            eprintln!("сугроб: не удалось запустить цикл событий: {e}");
            std::process::exit(1);
        }
    };

    let mut app = App {
        proxy: event_loop.create_proxy(),
        settings,
        sim,
        sky: {
            let mut sky = Sky::new(settings.sky_mode(), clock::now());
            sky.set_place(settings.sky_place_value());
            sky
        },
        platform,
        overlay: None,
        tray: None,
        menu: MenuState {
            running: true,
            scene: settings.scene,
            sky: settings.sky,
            sky_place: settings.sky_place,
            wind: settings.wind,
            wind_force: settings.wind_force,
            rate: settings.rate,
            solid: settings.solid,
            decor: settings.decor,
            dock: None,
            gpu: settings.gpu,
            gpu_switchable: false,
            adapter: String::new(),
        },
        software_available: Gfx::software_available(),
        running: true,
        last_frame: None,
        last_mouse: None,
        mouse_vel: (0.0, 0.0),
        poll_acc: 1.0,
        slow_acc: 0.0,
        dock: None,
    };
    if let Err(e) = event_loop.run_app(&mut app) {
        eprintln!("сугроб: {e}");
        std::process::exit(1);
    }
}
