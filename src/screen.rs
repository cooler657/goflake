//! Экран: прозрачное окно на одном мониторе со своей отрисовкой, симуляцией и небом.
//!
//! Мониторов может быть несколько — на каждом свой экран, у каждого своя
//! симуляция в его собственных координатах. Общее (список окон, курсор, меню,
//! настройки) хранит `App` в `main.rs`.

use std::sync::Arc;
use std::time::Instant;

use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowLevel};

use crate::gfx::Gfx;
use crate::platform;
use crate::sim::{Rect, Sim};
use crate::sky::{Sky, SkyMode};

/// Монитор: положение и размер в физических пикселях рабочего стола и масштаб.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MonitorInfo {
    pub pos: (i32, i32),
    pub size: (u32, u32),
    pub scale: f64,
}

impl MonitorInfo {
    /// Рамка монитора в единицах, в которых ОС отдаёт рамки окон: физические
    /// пиксели (Windows, X11) или точки (macOS). Нужна, чтобы узнать окно «на весь экран».
    pub fn os_bounds(&self) -> [f32; 4] {
        let k = if platform::PHYSICAL { 1.0 } else { 1.0 / self.scale as f32 };
        [self.pos.0 as f32 * k, self.pos.1 as f32 * k, self.size.0 as f32 * k, self.size.1 as f32 * k]
    }
}

/// Все подключённые мониторы. Если система ничего не сообщила — один условный.
pub fn monitors(el: &ActiveEventLoop) -> Vec<MonitorInfo> {
    let list: Vec<MonitorInfo> = el
        .available_monitors()
        .map(|m| MonitorInfo { pos: (m.position().x, m.position().y), size: (m.size().width, m.size().height), scale: m.scale_factor() })
        .collect();
    if list.is_empty() {
        vec![MonitorInfo { pos: (0, 0), size: (1280, 720), scale: 1.0 }]
    } else {
        list
    }
}

/// Сколько экран может не рисовать, прежде чем цепочку перерисовок сочтут оборвавшейся.
pub const STALL: std::time::Duration = std::time::Duration::from_millis(300);

/// Оборвалась ли цепочка перерисовок: экран не на паузе, следующий кадр не отложен
/// на таймер, а последний был давно (или ещё не было). Так бывает, пока открыто
/// меню в трее: система крутит свой цикл сообщений, и очередной запрос перерисовки
/// может потеряться. Тогда его нужно сделать заново.
pub fn needs_kick(paused: bool, next_frame: Option<Instant>, last_frame: Option<Instant>, now: Instant) -> bool {
    if paused || next_frame.is_some() {
        return false;
    }
    last_frame.is_none_or(|t| now.duration_since(t) > STALL)
}

/// Окно размером с монитор — полноэкранное приложение, игра или видео.
pub fn is_fullscreen(window: [f32; 4], monitor: [f32; 4]) -> bool {
    window.iter().zip(monitor.iter()).all(|(a, b)| (a - b).abs() <= 3.0)
}

/// Переводит точку из глобальных координат ОС в координаты симуляции экрана.
/// `origin` — левый верхний угол монитора в физических пикселях, `scale` — его масштаб.
pub fn to_local(physical: bool, origin: (f32, f32), scale: f32, x: f32, y: f32) -> (f32, f32) {
    if physical {
        ((x - origin.0) / scale, (y - origin.1) / scale)
    } else {
        (x - origin.0 / scale, y - origin.1 / scale)
    }
}

pub struct Screen {
    pub window: Arc<Window>,
    pub gfx: Gfx,
    pub sim: Sim,
    pub sky: Sky,
    pub monitor: MonitorInfo,
    /// Левый верхний угол окна в физических пикселях рабочего стола.
    pub origin: (f32, f32),
    pub scale: f32,
    /// Над экраном полноэкранное приложение: он рисует пустой кадр и стоит на паузе.
    pub paused: bool,
    pub last_frame: Option<Instant>,
    /// Когда рисовать следующий кадр (при экономии энергии кадры идут реже).
    pub next_frame: Option<Instant>,
    pub last_mouse: Option<(f32, f32)>,
    pub mouse_vel: (f32, f32),
    /// Номер списка окон, который симуляция уже видела.
    pub seen_gen: u64,
    pub last_refresh: Instant,
    pub dock: Option<Rect>,
    /// Небо одно на весь рабочий стол: солнце и луна рисуются только на основном мониторе.
    pub has_sky: bool,
}

impl Screen {
    pub fn create(
        el: &ActiveEventLoop,
        monitor: MonitorInfo,
        gpu: bool,
        sim: Sim,
        sky: Sky,
    ) -> Result<Self, String> {
        let mut size = PhysicalSize::new(monitor.size.0, monitor.size.1);
        // Окно ровно во весь экран Windows считает полноэкранным приложением
        // (прячет панель задач, глушит уведомления) — делаем его на пиксель ниже.
        if cfg!(target_os = "windows") {
            size.height = size.height.saturating_sub(1).max(1);
        }
        let pos = PhysicalPosition::new(monitor.pos.0, monitor.pos.1);
        let attrs = Window::default_attributes()
            .with_title("Сугроб")
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(false)
            .with_active(false)
            .with_window_level(WindowLevel::AlwaysOnTop);
        // На macOS положение и размер задаются в точках: так окно попадает на нужный
        // монитор и при разном масштабе у разных мониторов.
        #[cfg(target_os = "macos")]
        let attrs = {
            use winit::dpi::{LogicalPosition, LogicalSize};
            let s = monitor.scale;
            attrs
                .with_position(LogicalPosition::new(pos.x as f64 / s, pos.y as f64 / s))
                .with_inner_size(LogicalSize::new(size.width as f64 / s, size.height as f64 / s))
        };
        #[cfg(not(target_os = "macos"))]
        let attrs = attrs.with_position(pos).with_inner_size(size);

        let window = el
            .create_window(platform::window_attributes(attrs))
            .map_err(|e| format!("не удалось создать окно: {e}"))?;
        let window = Arc::new(window);
        // клики и перетаскивания проходят сквозь снег
        if let Err(e) = window.set_cursor_hittest(false) {
            eprintln!("сугроб: окно не удалось сделать прозрачным для мыши: {e}");
        }
        platform::configure_window(&window);

        let gfx = Gfx::new(window.clone(), el.owned_display_handle(), gpu)?;
        let mut screen = Screen {
            window,
            gfx,
            sim,
            sky,
            monitor,
            origin: (pos.x as f32, pos.y as f32),
            scale: 1.0,
            paused: false,
            last_frame: None,
            next_frame: None,
            last_mouse: None,
            mouse_vel: (0.0, 0.0),
            seen_gen: u64::MAX,
            last_refresh: Instant::now(),
            dock: None,
            has_sky: true,
        };
        screen.apply_size();
        Ok(screen)
    }

    /// Обновляет масштаб, размер поверхности и размеры мира симуляции.
    pub fn apply_size(&mut self) {
        let size = self.window.inner_size();
        self.scale = self.window.scale_factor() as f32;
        self.gfx.resize(size.width, size.height);
        self.sim.w = size.width as f32 / self.scale;
        self.sim.h = size.height as f32 / self.scale;
    }

    /// Режим неба для этого экрана: на неосновных мониторах неба нет.
    pub fn sky_mode(&self, mode: SkyMode) -> SkyMode {
        if self.has_sky { mode } else { SkyMode::Off }
    }

    pub fn point(&self, x: f32, y: f32) -> (f32, f32) {
        to_local(platform::PHYSICAL, self.origin, self.scale, x, y)
    }

    pub fn rect(&self, r: [f32; 4]) -> Rect {
        let (x, y) = self.point(r[0], r[1]);
        let k = if platform::PHYSICAL { 1.0 / self.scale } else { 1.0 };
        Rect::new(x, y, r[2] * k, r[3] * k)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stalled_redraw_chain_is_noticed_and_a_healthy_one_is_left_alone() {
        let now = Instant::now();
        let ago = |ms: u64| Some(now - std::time::Duration::from_millis(ms));
        // кадры идут — ничего не трогаем
        assert!(!needs_kick(false, None, ago(16), now));
        // кадров давно нет, а экран не на паузе — перезапускаем
        assert!(needs_kick(false, None, ago(600), now));
        // кадров ещё не было совсем (после включения) — запускаем
        assert!(needs_kick(false, None, None, now));
        // на паузе и при отложенном кадре (энергосбережение) цепочку не трогаем
        assert!(!needs_kick(true, None, ago(5000), now));
        assert!(!needs_kick(false, Some(now + std::time::Duration::from_millis(20)), ago(5000), now));
    }

    #[test]
    fn a_window_the_size_of_the_monitor_is_fullscreen() {
        let m = [0.0, 0.0, 1470.0, 956.0];
        assert!(is_fullscreen([0.0, 0.0, 1470.0, 956.0], m));
        assert!(is_fullscreen([1.0, -2.0, 1472.0, 957.0], m), "допуск в пару точек");
        // обычное и развёрнутое окно (меню и Dock остаются) — нет
        assert!(!is_fullscreen([100.0, 80.0, 800.0, 600.0], m));
        assert!(!is_fullscreen([0.0, 38.0, 1470.0, 880.0], m));
        // монитор слева от основного: окно на нём не полноэкранное для основного
        assert!(!is_fullscreen([-1920.0, 0.0, 1920.0, 1080.0], m));
        assert!(is_fullscreen([-1920.0, 0.0, 1920.0, 1080.0], [-1920.0, 0.0, 1920.0, 1080.0]));
    }

    #[test]
    fn monitor_bounds_use_the_units_the_os_reports_windows_in() {
        let m = MonitorInfo { pos: (200, 100), size: (3000, 2000), scale: 2.0 };
        let b = m.os_bounds();
        if platform::PHYSICAL {
            assert_eq!(b, [200.0, 100.0, 3000.0, 2000.0]);
        } else {
            assert_eq!(b, [100.0, 50.0, 1500.0, 1000.0]);
        }
    }

    #[test]
    fn global_points_become_local_to_each_monitor() {
        // физические пиксели (Windows, X11): монитор 4K справа от основного, масштаб 2
        assert_eq!(to_local(true, (1920.0, 0.0), 2.0, 2920.0, 400.0), (500.0, 200.0));
        // точки (macOS): монитор начинается в (1470, 0) точек = 2940 пикселей при масштабе 2
        assert_eq!(to_local(false, (2940.0, 0.0), 2.0, 1970.0, 200.0), (500.0, 200.0));
        // монитор слева от основного: отрицательное начало
        assert_eq!(to_local(false, (-3840.0, 0.0), 2.0, -1000.0, 50.0), (920.0, 50.0));
        // курсор на соседнем мониторе оказывается за границей мира этого экрана
        let (x, _) = to_local(false, (2940.0, 0.0), 2.0, 300.0, 100.0);
        assert!(x < 0.0);
    }
}
