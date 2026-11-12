//! Linux: X11 через x11rb. Нужен оконный менеджер с поддержкой EWMH и композитор.
//!
//! В сеансе Wayland приложение работает через XWayland и видит только окна
//! X11-приложений: протокол Wayland не позволяет узнать положение чужих окон.

use winit::event_loop::EventLoopBuilder;
use winit::platform::x11::{EventLoopBuilderExtX11, WindowAttributesExtX11};
use winit::window::{Window, WindowAttributes};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{Atom, AtomEnum, ConnectionExt, Window as XWindow};
use x11rb::rust_connection::RustConnection;

use super::RawWindow;

pub const PHYSICAL: bool = true;
pub const DOCK_LIFT: f32 = 0.0;

/// «На всех рабочих столах» в `_NET_WM_DESKTOP`.
const ALL_DESKTOPS: u32 = 0xFFFF_FFFF;

struct Atoms {
    client_list_stacking: Atom,
    current_desktop: Atom,
    wm_desktop: Atom,
    wm_pid: Atom,
    wm_state: Atom,
    state_hidden: Atom,
    window_type: Atom,
    type_dock: Atom,
    type_desktop: Atom,
    frame_extents: Atom,
    gtk_frame_extents: Atom,
}

pub struct Platform {
    conn: RustConnection,
    root: XWindow,
    pid: u32,
    atoms: Atoms,
    /// Рамка нижней панели, найденная при последнем обходе окон.
    panel: Option<[f32; 4]>,
}

impl Platform {
    pub fn new() -> Result<Self, String> {
        let (conn, screen) =
            x11rb::connect(None).map_err(|e| format!("не удалось подключиться к X-серверу: {e}"))?;
        let root = conn.setup().roots[screen].root;
        let atom = |name: &str| -> Result<Atom, String> {
            conn.intern_atom(false, name.as_bytes())
                .map_err(|e| e.to_string())?
                .reply()
                .map(|r| r.atom)
                .map_err(|e| e.to_string())
        };
        let atoms = Atoms {
            client_list_stacking: atom("_NET_CLIENT_LIST_STACKING")?,
            current_desktop: atom("_NET_CURRENT_DESKTOP")?,
            wm_desktop: atom("_NET_WM_DESKTOP")?,
            wm_pid: atom("_NET_WM_PID")?,
            wm_state: atom("_NET_WM_STATE")?,
            state_hidden: atom("_NET_WM_STATE_HIDDEN")?,
            window_type: atom("_NET_WM_WINDOW_TYPE")?,
            type_dock: atom("_NET_WM_WINDOW_TYPE_DOCK")?,
            type_desktop: atom("_NET_WM_WINDOW_TYPE_DESKTOP")?,
            frame_extents: atom("_NET_FRAME_EXTENTS")?,
            gtk_frame_extents: atom("_GTK_FRAME_EXTENTS")?,
        };
        Ok(Self { conn, root, pid: std::process::id(), atoms, panel: None })
    }

    /// Читает свойство окна как список 32-битных значений.
    fn prop(&self, win: XWindow, prop: Atom) -> Vec<u32> {
        self.conn
            .get_property(false, win, prop, AtomEnum::ANY, 0, 4096)
            .ok()
            .and_then(|c| c.reply().ok())
            .and_then(|r| r.value32().map(|v| v.collect()))
            .unwrap_or_default()
    }

    /// Рамка окна в координатах корневого окна: `[x, y, ширина, высота]`.
    fn frame(&self, win: XWindow) -> Option<[f32; 4]> {
        let geo = self.conn.get_geometry(win).ok()?.reply().ok()?;
        let pos = self.conn.translate_coordinates(win, self.root, 0, 0).ok()?.reply().ok()?;
        let (mut x, mut y) = (pos.dst_x as f32, pos.dst_y as f32);
        let (mut w, mut h) = (geo.width as f32, geo.height as f32);
        // рамка оконного менеджера (заголовок) — снаружи клиентской области
        if let [l, r, t, b] = self.prop(win, self.atoms.frame_extents)[..] {
            x -= l as f32;
            y -= t as f32;
            w += (l + r) as f32;
            h += (t + b) as f32;
        }
        // у окон с собственным оформлением (GTK) в размер входит невидимая тень
        if let [l, r, t, b] = self.prop(win, self.atoms.gtk_frame_extents)[..] {
            x += l as f32;
            y += t as f32;
            w -= (l + r) as f32;
            h -= (t + b) as f32;
        }
        (w > 0.0 && h > 0.0).then_some([x, y, w, h])
    }

    pub fn windows(&mut self) -> Vec<RawWindow> {
        let mut out = Vec::new();
        self.panel = None;
        let current = self.prop(self.root, self.atoms.current_desktop).first().copied();
        // список идёт от заднего окна к переднему — разворачиваем
        let stack = self.prop(self.root, self.atoms.client_list_stacking);
        for &win in stack.iter().rev() {
            if self.prop(win, self.atoms.wm_pid).first() == Some(&self.pid) {
                continue;
            }
            if self.prop(win, self.atoms.wm_state).contains(&self.atoms.state_hidden) {
                continue;
            }
            let types = self.prop(win, self.atoms.window_type);
            if types.contains(&self.atoms.type_desktop) {
                continue;
            }
            if types.contains(&self.atoms.type_dock) {
                // панель: запоминаем самую нижнюю из широких
                if let Some(r) = self.frame(win)
                    && r[2] > r[3]
                    && self.panel.is_none_or(|p| r[1] > p[1])
                {
                    self.panel = Some(r);
                }
                continue;
            }
            if let (Some(cur), Some(&desk)) = (current, self.prop(win, self.atoms.wm_desktop).first())
                && desk != cur
                && desk != ALL_DESKTOPS
            {
                continue;
            }
            if let Some(rect) = self.frame(win) {
                out.push(RawWindow { id: win as i64, rect });
            }
        }
        out
    }

    pub fn cursor(&mut self) -> Option<(f32, f32)> {
        let p = self.conn.query_pointer(self.root).ok()?.reply().ok()?;
        Some((p.root_x as f32, p.root_y as f32))
    }

    /// Рамка нижней панели рабочего стола. Разрешений не требует.
    pub fn dock(&mut self) -> Option<[f32; 4]> {
        self.panel
    }

    /// Профиль питания «экономия» (power-saver), если ядро его сообщает.
    pub fn low_power(&mut self) -> bool {
        std::fs::read_to_string("/sys/firmware/acpi/platform_profile").is_ok_and(|s| s.trim() == "low-power")
    }

    /// В X11 полноэкранное окно находится по списку окон.
    pub fn fullscreen_app(&mut self) -> bool {
        false
    }

    pub fn dock_access(&self) -> Option<bool> {
        None
    }

    pub fn request_dock_access(&self) {}
}

pub fn configure_event_loop<T: 'static>(builder: &mut EventLoopBuilder<T>) {
    builder.with_x11();
}

pub fn window_attributes(attrs: WindowAttributes) -> WindowAttributes {
    // Окно в обход оконного менеджера: без рамки, фокуса и записи в панели задач.
    attrs.with_override_redirect(true)
}

pub fn configure_window(_window: &Window) {}
