//! Всё, что зависит от ОС: список окон, курсор, Dock / панель задач, настройка оверлея.
//!
//! Каждая платформа предоставляет тип `Platform` с одинаковым набором методов:
//!
//! - `new() -> Result<Platform, String>`
//! - `windows(&mut self) -> Vec<RawWindow>` — обычные окна от переднего к заднему, без нашего;
//! - `cursor(&mut self) -> Option<(f32, f32)>` — глобальное положение курсора;
//! - `dock(&mut self) -> Option<[f32; 4]>` — рамка Dock или панели задач;
//! - `dock_access(&self) -> Option<bool>` — нужно ли и выдано ли разрешение для Dock
//!   (`None` — разрешение не требуется);
//! - `request_dock_access(&self)` — запросить это разрешение.
//!
//! Константы:
//!
//! - `PHYSICAL` — в каких единицах ОС отдаёт координаты: физические пиксели (`true`)
//!   или логические точки (`false`);
//! - `DOCK_LIFT` — на сколько точек кромка кучи выше рамки Dock (минус — ниже).

/// Окно в глобальных координатах ОС: `[x, y, ширина, высота]`, y вниз.
pub struct RawWindow {
    pub id: i64,
    pub rect: [f32; 4],
}

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::*;

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use windows::*;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::*;

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
compile_error!("Сугроб поддерживает только macOS, Windows и Linux (X11)");
