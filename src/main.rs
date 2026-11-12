//! Сугроб — снег, дождь, песок и листья поверх рабочего стола.
//!
//! На каждом мониторе — прозрачное окно на весь экран, пропускающее клики (`screen`),
//! со своей симуляцией. Физика — в `sim`, отрисовка на GPU — в `render`, всё
//! зависящее от ОС — в `platform` и `tray`.

// В релизной сборке под Windows не открываем окно консоли.
#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]

mod clock;
mod decor;
mod gfx;
mod look;
mod materials;
mod platform;
mod render;
mod screen;
mod settings;
mod sim;
mod sky;
mod soft;
mod tray;

use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::window::WindowId;

use gfx::Gfx;
use platform::{Platform, RawWindow};
use screen::{MonitorInfo, Screen};
use settings::{RATES, Settings};
use sim::{Rect, Sim, WindowInfo};
use sky::Sky;
use tray::{Cmd, MenuState, Tray};

/// Как часто опрашивается список окон, секунды.
const POLL_WINDOWS: f32 = 0.05;
/// Как часто проверяются Dock, мониторы, энергосбережение и разрешения, секунды.
const POLL_SLOW: f32 = 2.0;
/// Как часто смотреть, не ушло ли полноэкранное приложение, пока всё на паузе.
const POLL_PAUSED: f32 = 0.5;
/// Во сколько раз меньше осадков в режиме энергосбережения.
const POWER_SAVING_RATE: f32 = 0.4;
/// Пауза между кадрами в режиме энергосбережения (около 30 кадров в секунду).
const POWER_SAVING_FRAME: Duration = Duration::from_millis(33);

/// Пауза между кадрами: в обычном режиме кадры идут сразу один за другим по
/// вертикальной синхронизации, при энергосбережении — реже.
fn frame_interval(low_power: bool) -> Option<Duration> {
    low_power.then_some(POWER_SAVING_FRAME)
}

/// Новая симуляция с текущими настройками.
fn new_sim(settings: &Settings, seed: u64, low_power: bool) -> Sim {
    let mut sim = Sim::new(seed);
    sim.rate = RATES[settings.rate].1;
    sim.rate_scale = if low_power { POWER_SAVING_RATE } else { 1.0 };
    sim.solid = settings.solid;
    sim.decor = settings.decor;
    sim.gather = settings.gather;
    sim.wind_dir = settings.wind_dir();
    sim.wind_force = settings.wind_force_value();
    sim.set_kind(settings.kind());
    sim
}

fn new_sky(settings: &Settings) -> Sky {
    let mut sky = Sky::new(settings.sky_mode(), clock::now());
    sky.set_place(settings.sky_place_value());
    sky
}

struct App {
    proxy: EventLoopProxy<Cmd>,
    settings: Settings,
    platform: Platform,
    /// По одному экрану на монитор.
    screens: Vec<Screen>,
    /// Мониторы, под которые созданы экраны: изменились — экраны пересоздаются.
    monitors: Vec<MonitorInfo>,
    tray: Option<Tray>,
    menu: MenuState,
    /// Есть ли программный адаптер, на который можно переключиться.
    software_available: bool,
    running: bool,
    /// Включён режим энергосбережения: осадков меньше, кадры реже.
    low_power: bool,
    /// Последний список окон; один на все экраны.
    windows: Vec<RawWindow>,
    /// Номер списка окон: растёт, когда он обновлён.
    win_gen: u64,
    last_poll: Instant,
    last_slow: Instant,
    seed: u64,
}

impl App {
    fn menu_state(&self) -> MenuState {
        let first = self.screens.first();
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
            gather: self.settings.gather,
            dock: self.platform.dock_access(),
            gpu: first.map_or(self.settings.gpu, |s| !s.gfx.software()),
            gpu_switchable: self.software_available,
            adapter: first.map_or_else(String::new, |s| s.gfx.adapter()),
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

    /// Создаёт по экрану на каждый монитор.
    fn build_screens(&mut self, el: &ActiveEventLoop) -> Result<(), String> {
        let monitors = screen::monitors(el);
        let mut screens = Vec::with_capacity(monitors.len());
        for (i, m) in monitors.iter().enumerate() {
            let sim = new_sim(&self.settings, self.seed.wrapping_add(i as u64 * 7919), self.low_power);
            let screen = Screen::create(el, *m, self.settings.gpu, sim, new_sky(&self.settings))?;
            eprintln!("сугроб: экран {} — отрисовка: {}", i + 1, screen.gfx.info());
            screens.push(screen);
        }
        if !self.software_available {
            eprintln!("сугроб: программной отрисовки в системе нет — переключатель в меню отключён");
        }
        self.screens = screens;
        self.monitors = monitors;
        for s in &self.screens {
            s.window.request_redraw();
        }
        Ok(())
    }

    /// Пересоздаёт отрисовку на всех экранах после переключения аппаратного ускорения.
    fn rebuild_renderers(&mut self, el: &ActiveEventLoop) {
        let old = std::mem::take(&mut self.screens);
        for s in old {
            let window = s.window.clone();
            // старую поверхность нужно освободить до создания новой на том же окне
            drop(s.gfx);
            let display = el.owned_display_handle();
            let made = Gfx::new(window.clone(), display.clone(), self.settings.gpu).or_else(|e| {
                eprintln!("сугроб: не удалось переключить отрисовку ({e}), возвращаю прежнюю");
                self.settings.gpu = !self.settings.gpu;
                Gfx::new(window.clone(), display, self.settings.gpu)
            });
            match made {
                Ok(gfx) => {
                    eprintln!("сугроб: отрисовка — {}", gfx.info());
                    let mut s = Screen { gfx, ..s };
                    s.apply_size();
                    s.window.request_redraw();
                    self.screens.push(s);
                }
                Err(e) => {
                    eprintln!("сугроб: {e}");
                    el.exit();
                    return;
                }
            }
        }
    }

    /// Обновляет список окон и следит, не встало ли над экраном полноэкранное
    /// приложение: тогда экран рисует пустой кадр и замирает, пока оно не уйдёт.
    fn poll_windows(&mut self) {
        let all_paused = !self.screens.is_empty() && self.screens.iter().all(|s| s.paused);
        let every = if all_paused { POLL_PAUSED } else { POLL_WINDOWS };
        if self.last_poll.elapsed().as_secs_f32() < every {
            return;
        }
        self.last_poll = Instant::now();
        self.windows = self.platform.windows();
        self.win_gen += 1;
        let system_fullscreen = self.platform.fullscreen_app();
        for (i, s) in self.screens.iter_mut().enumerate() {
            let bounds = s.monitor.os_bounds();
            let fullscreen = system_fullscreen || self.windows.iter().any(|w| screen::is_fullscreen(w.rect, bounds));
            if fullscreen && !s.paused {
                eprintln!("сугроб: над экраном {} полноэкранное приложение — пауза", i + 1);
                s.paused = true;
                s.next_frame = None;
                s.gfx.clear();
            } else if !fullscreen && s.paused {
                eprintln!("сугроб: экран {} свободен — продолжаю", i + 1);
                s.paused = false;
                s.last_frame = None;
                s.last_mouse = None;
                s.window.request_redraw();
            }
        }
    }

    /// Редкие проверки: мониторы, Dock, время, энергосбережение, галочки в меню.
    fn maintain(&mut self, el: &ActiveEventLoop) {
        if self.last_slow.elapsed().as_secs_f32() < POLL_SLOW {
            return;
        }
        self.last_slow = Instant::now();
        // подключили или отключили монитор, сменили разрешение — экраны пересоздаются
        if screen::monitors(el) != self.monitors {
            self.screens.clear();
            if let Err(e) = self.build_screens(el) {
                eprintln!("сугроб: {e}");
                el.exit();
            }
            return;
        }
        let clk = clock::now();
        let kind = self.settings.kind();
        // SNOWDRIFT_LOW_POWER=1 включает облегчённый режим принудительно (для проверки)
        let low_power = self.platform.low_power() || std::env::var_os("SNOWDRIFT_LOW_POWER").is_some();
        if low_power != self.low_power {
            eprintln!("сугроб: энергосбережение {}", if low_power { "включено — осадков меньше, кадры реже" } else { "выключено" });
            self.low_power = low_power;
            for s in &mut self.screens {
                s.sim.rate_scale = if low_power { POWER_SAVING_RATE } else { 1.0 };
                // вышли из энергосбережения — не ждём отложенного кадра
                if !low_power && s.next_frame.take().is_some() {
                    s.window.request_redraw();
                }
            }
        }
        let dock = self.platform.dock();
        for s in &mut self.screens {
            s.sky.set_clock(clk);
            // в режиме «по времени года» материал меняется вместе с календарём
            s.sim.set_kind(kind);
            // Dock лежит на одном мониторе: на том, где его середина; снизу экрана
            s.dock = dock.and_then(|raw| {
                let b = s.monitor.os_bounds();
                let (cx, cy) = (raw[0] + raw[2] / 2.0, raw[1] + raw[3] / 2.0);
                if cx < b[0] || cx > b[0] + b[2] || cy < b[1] || cy > b[1] + b[3] {
                    return None;
                }
                let r = s.rect(raw);
                let bottom = r.w > r.h && r.min_y() < s.sim.h - 10.0 && r.max_y() > s.sim.h - 150.0;
                bottom.then(|| Rect::new(r.x, r.y - platform::DOCK_LIFT, r.w, r.h + platform::DOCK_LIFT))
            });
        }
        self.sync_menu();
    }

    fn frame(&mut self, i: usize) {
        if !self.running {
            return;
        }
        let Some(s) = self.screens.get_mut(i) else { return };
        if s.paused {
            return;
        }
        let now = Instant::now();
        let dt = s.last_frame.map_or(1.0 / 60.0, |t| (now - t).as_secs_f32()).clamp(0.001, 0.05);
        s.last_frame = Some(now);
        s.sky.step(dt);

        // курсор и его сглаженная скорость
        let cursor = self.platform.cursor().map(|(x, y)| s.point(x, y)).or(s.last_mouse);
        let c = cursor.unwrap_or((-1000.0, -1000.0));
        if let Some(prev) = s.last_mouse {
            let v = (((c.0 - prev.0) / dt).clamp(-1500.0, 1500.0), ((c.1 - prev.1) / dt).clamp(-1500.0, 1500.0));
            s.mouse_vel.0 += (v.0 - s.mouse_vel.0) * 0.4;
            s.mouse_vel.1 += (v.1 - s.mouse_vel.1) * 0.4;
        }
        s.last_mouse = cursor;

        // окна: один список на все экраны, каждый переводит его в свои координаты
        if s.seen_gen != self.win_gen {
            let list: Vec<WindowInfo> =
                self.windows.iter().map(|w| WindowInfo { id: w.id, rect: s.rect(w.rect) }).collect();
            let poll_dt = s.last_refresh.elapsed().as_secs_f32().max(0.001);
            s.sim.refresh(&list, s.dock, poll_dt);
            s.seen_gen = self.win_gen;
            s.last_refresh = now;
        }

        s.sim.step(dt, c, s.mouse_vel);
        if !s.gfx.render(&s.sim, &s.sky, s.scale) {
            // кадр недоступен (окно скрыто системой) — не крутим цикл вхолостую
            std::thread::sleep(Duration::from_millis(16));
        }
        match frame_interval(self.low_power) {
            Some(gap) => s.next_frame = Some(now + gap),
            None => s.window.request_redraw(),
        }
    }

    fn command(&mut self, el: &ActiveEventLoop, cmd: Cmd) {
        match cmd {
            Cmd::Toggle => {
                self.running = !self.running;
                for s in &mut self.screens {
                    s.window.set_visible(self.running);
                    if self.running {
                        s.last_frame = None;
                        s.last_mouse = None;
                        s.window.request_redraw();
                    }
                }
            }
            Cmd::Scene(i) => {
                self.settings.scene = i.min(settings::SCENES - 1);
                let kind = self.settings.kind();
                self.screens.iter_mut().for_each(|s| s.sim.set_kind(kind));
                self.settings.save();
            }
            Cmd::Sky(i) => {
                self.settings.sky = i.min(settings::SKIES - 1);
                let mode = self.settings.sky_mode();
                self.screens.iter_mut().for_each(|s| s.sky.set_mode(mode));
                self.settings.save();
            }
            Cmd::SkyPlace(i) => {
                self.settings.sky_place = i.min(settings::PLACE_COUNT - 1);
                let place = self.settings.sky_place_value();
                self.screens.iter_mut().for_each(|s| s.sky.set_place(place));
                self.settings.save();
            }
            Cmd::Wind(i) => {
                self.settings.wind = i.min(settings::WIND_DIRS.len() - 1);
                let dir = self.settings.wind_dir();
                self.screens.iter_mut().for_each(|s| s.sim.wind_dir = dir);
                self.settings.save();
            }
            Cmd::WindForce(i) => {
                self.settings.wind_force = i.min(settings::WIND_FORCES.len() - 1);
                let force = self.settings.wind_force_value();
                self.screens.iter_mut().for_each(|s| s.sim.wind_force = force);
                self.settings.save();
            }
            Cmd::Rate(i) => {
                self.settings.rate = i.min(RATES.len() - 1);
                let rate = RATES[self.settings.rate].1;
                self.screens.iter_mut().for_each(|s| s.sim.rate = rate);
                self.settings.save();
            }
            Cmd::Solid => {
                self.settings.solid = !self.settings.solid;
                let on = self.settings.solid;
                self.screens.iter_mut().for_each(|s| s.sim.solid = on);
                self.settings.save();
            }
            Cmd::Decor => {
                self.settings.decor = !self.settings.decor;
                let on = self.settings.decor;
                self.screens.iter_mut().for_each(|s| s.sim.decor = on);
                self.settings.save();
            }
            Cmd::Cursor(i) => {
                self.settings.gather = i == 1;
                let on = self.settings.gather;
                self.screens.iter_mut().for_each(|s| s.sim.gather = on);
                self.settings.save();
            }
            Cmd::Shake => self.screens.iter_mut().for_each(|s| s.sim.dump_all()),
            Cmd::Gpu => {
                if self.software_available {
                    self.settings.gpu = !self.settings.gpu;
                    self.rebuild_renderers(el);
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
        if !self.screens.is_empty() {
            return;
        }
        if let Err(e) = self.build_screens(el) {
            eprintln!("сугроб: {e}");
            el.exit();
            return;
        }
        self.menu = self.menu_state();
        match Tray::new(self.proxy.clone(), &self.menu) {
            Ok(tray) => self.tray = Some(tray),
            Err(e) => eprintln!("сугроб: значок в трее недоступен ({e}); завершить приложение можно через диспетчер задач"),
        }
        self.last_poll = Instant::now() - Duration::from_secs(1);
        self.last_slow = Instant::now() - Duration::from_secs(10);
        self.poll_windows();
        self.maintain(el);
    }

    fn user_event(&mut self, el: &ActiveEventLoop, cmd: Cmd) {
        self.command(el, cmd);
    }

    fn window_event(&mut self, el: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let Some(i) = self.screens.iter().position(|s| s.window.id() == id) else { return };
        match event {
            WindowEvent::RedrawRequested => self.frame(i),
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => self.screens[i].apply_size(),
            WindowEvent::CloseRequested => el.exit(),
            _ => {}
        }
    }

    /// Вызывается после каждой пачки событий: обновляет окна, следит за паузой и
    /// будит цикл вовремя, когда кадры идут реже обычного или всё на паузе.
    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        if !self.running || self.screens.is_empty() {
            el.set_control_flow(ControlFlow::Wait);
            return;
        }
        self.poll_windows();
        self.maintain(el);
        let now = Instant::now();
        let mut wake: Option<Instant> = None;
        let mut soonest = |t: Instant| wake = Some(wake.map_or(t, |w| w.min(t)));
        for s in &mut self.screens {
            if let Some(t) = s.next_frame {
                if now >= t {
                    s.next_frame = None;
                    s.window.request_redraw();
                } else {
                    soonest(t);
                }
            }
        }
        if self.screens.iter().all(|s| s.paused) {
            soonest(now + Duration::from_secs_f32(POLL_PAUSED));
        }
        el.set_control_flow(wake.map_or(ControlFlow::Wait, ControlFlow::WaitUntil));
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
        platform,
        screens: Vec::new(),
        monitors: Vec::new(),
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
            gather: settings.gather,
            dock: None,
            gpu: settings.gpu,
            gpu_switchable: false,
            adapter: String::new(),
        },
        software_available: Gfx::software_available(),
        running: true,
        low_power: false,
        windows: Vec::new(),
        win_gen: 0,
        last_poll: Instant::now(),
        last_slow: Instant::now(),
        seed,
    };
    if let Err(e) = event_loop.run_app(&mut app) {
        eprintln!("сугроб: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn power_saving_slows_the_frame_rate_only_when_asked() {
        assert_eq!(frame_interval(false), None);
        let gap = frame_interval(true).unwrap();
        assert!(gap >= Duration::from_millis(30) && gap <= Duration::from_millis(40));
    }

    #[test]
    fn a_new_sim_carries_the_settings_and_the_power_mode() {
        let st = Settings { rate: 2, wind: 1, wind_force: 3, solid: true, decor: false, gather: false, ..Settings::default() };
        let sim = new_sim(&st, 1, true);
        assert_eq!(sim.rate, RATES[2].1);
        assert_eq!(sim.rate_scale, POWER_SAVING_RATE);
        assert!(sim.solid && !sim.decor && !sim.gather);
        assert_eq!(new_sim(&st, 1, false).rate_scale, 1.0);
    }
}
