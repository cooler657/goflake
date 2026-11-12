//! Выбор способа отрисовки: видеокарта или процессор.
//!
//! - **Видеокарта** — `render::Renderer` на wgpu.
//! - **Программно** — в Windows и Linux тот же `Renderer`, но на программном
//!   адаптере системы (WARP, llvmpipe). В macOS такого адаптера нет, поэтому
//!   там кадр рисует собственный растеризатор `soft::Canvas`.

use std::sync::Arc;

use winit::event_loop::OwnedDisplayHandle;
use winit::window::Window;

use crate::render::Renderer;
use crate::sim::Sim;
use crate::sky::Sky;

pub enum Gfx {
    Gpu(Renderer),
    #[cfg(target_os = "macos")]
    Soft(crate::platform::SoftSurface),
}

impl Gfx {
    /// Можно ли переключаться между видеокартой и программной отрисовкой.
    pub fn software_available() -> bool {
        cfg!(target_os = "macos") || Renderer::software_available()
    }

    /// `prefer_gpu` — рисовать на видеокарте; иначе программно.
    pub fn new(window: Arc<Window>, display: OwnedDisplayHandle, prefer_gpu: bool) -> Result<Self, String> {
        #[cfg(target_os = "macos")]
        if !prefer_gpu {
            let _ = display;
            return crate::platform::SoftSurface::new(&window).map(Gfx::Soft);
        }
        Renderer::new(window, display, prefer_gpu).map(Gfx::Gpu)
    }

    /// Рисует кадр. Возвращает `false`, если кадр получить не удалось.
    pub fn render(&mut self, sim: &Sim, sky: &Sky, scale: f32) -> bool {
        match self {
            Gfx::Gpu(r) => r.render(sim, sky, scale),
            #[cfg(target_os = "macos")]
            Gfx::Soft(s) => s.render(sim, sky),
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        match self {
            Gfx::Gpu(r) => r.resize(width, height),
            // программный растр подстраивается под размер мира на каждом кадре
            #[cfg(target_os = "macos")]
            Gfx::Soft(_) => {}
        }
    }

    /// Идёт ли отрисовка на процессоре.
    pub fn software(&self) -> bool {
        match self {
            Gfx::Gpu(r) => r.software,
            #[cfg(target_os = "macos")]
            Gfx::Soft(_) => true,
        }
    }

    /// Короткое название того, что рисует, — для меню.
    pub fn adapter(&self) -> String {
        match self {
            Gfx::Gpu(r) => r.adapter.clone(),
            #[cfg(target_os = "macos")]
            Gfx::Soft(_) => "Процессор · программно".into(),
        }
    }

    /// Подробное описание — для журнала.
    pub fn info(&self) -> String {
        match self {
            Gfx::Gpu(r) => r.info.clone(),
            #[cfg(target_os = "macos")]
            Gfx::Soft(_) => "программная отрисовка на процессоре, разрешение 1x".into(),
        }
    }
}
