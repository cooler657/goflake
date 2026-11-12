//! Windows: Win32 для окон, курсора и панели задач.

use std::ffi::c_void;

use windows_sys::Win32::Foundation::{HWND, LPARAM, POINT, RECT};
use windows_sys::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute};
use windows_sys::Win32::System::Threading::GetCurrentProcessId;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, FindWindowW, GWL_EXSTYLE, GetClassNameW, GetCursorPos, GetWindowLongW, GetWindowRect,
    GetWindowThreadProcessId, IsIconic, IsWindowVisible, WS_EX_TOOLWINDOW,
};
use windows_sys::core::BOOL;
use winit::event_loop::EventLoopBuilder;
use winit::platform::windows::WindowAttributesExtWindows;
use winit::window::{Window, WindowAttributes};

use super::RawWindow;

pub const PHYSICAL: bool = true;
pub const DOCK_LIFT: f32 = 0.0;

/// Служебные окна оболочки: рабочий стол и панели задач.
const SHELL_CLASSES: [&str; 4] = ["Progman", "WorkerW", "Shell_TrayWnd", "Shell_SecondaryTrayWnd"];

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

struct EnumState {
    pid: u32,
    out: Vec<RawWindow>,
}

/// Вызывается для каждого окна верхнего уровня, от переднего к заднему.
unsafe extern "system" fn on_window(hwnd: HWND, lparam: LPARAM) -> BOOL {
    // SAFETY: `lparam` — указатель на `EnumState`, живущий на время `EnumWindows`.
    let state = unsafe { &mut *(lparam as *mut EnumState) };
    // SAFETY: `hwnd` получен от системы и действителен на время вызова.
    unsafe {
        if IsWindowVisible(hwnd) == 0 || IsIconic(hwnd) != 0 {
            return 1;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == state.pid || GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW != 0 {
            return 1;
        }
        // «спрятанные» окна (другой виртуальный стол, приостановленные UWP) формально видимы
        let mut cloaked = 0u32;
        DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED as u32, (&raw mut cloaked).cast::<c_void>(), 4);
        if cloaked != 0 {
            return 1;
        }
        let mut class = [0u16; 64];
        let len = GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32).max(0) as usize;
        let class = String::from_utf16_lossy(&class[..len]);
        if SHELL_CLASSES.contains(&class.as_str()) {
            return 1;
        }
        // рамка без невидимых полей для изменения размера и без тени
        let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        let hr = DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS as u32,
            (&raw mut r).cast::<c_void>(),
            size_of::<RECT>() as u32,
        );
        if hr != 0 && GetWindowRect(hwnd, &mut r) == 0 {
            return 1;
        }
        state.out.push(RawWindow {
            id: hwnd as isize as i64,
            rect: [r.left as f32, r.top as f32, (r.right - r.left) as f32, (r.bottom - r.top) as f32],
        });
    }
    1
}

pub struct Platform {
    pid: u32,
}

impl Platform {
    pub fn new() -> Result<Self, String> {
        // SAFETY: функция без аргументов.
        Ok(Self { pid: unsafe { GetCurrentProcessId() } })
    }

    pub fn windows(&mut self) -> Vec<RawWindow> {
        let mut state = EnumState { pid: self.pid, out: Vec::new() };
        // SAFETY: `state` живёт дольше вызова; обратный вызов синхронный.
        unsafe { EnumWindows(Some(on_window), (&raw mut state) as LPARAM) };
        state.out
    }

    pub fn cursor(&mut self) -> Option<(f32, f32)> {
        let mut p = POINT { x: 0, y: 0 };
        // SAFETY: передаётся указатель на локальную структуру.
        (unsafe { GetCursorPos(&mut p) } != 0).then_some((p.x as f32, p.y as f32))
    }

    /// Рамка панели задач. Разрешений не требует.
    pub fn dock(&mut self) -> Option<[f32; 4]> {
        let class = wide("Shell_TrayWnd");
        let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        // SAFETY: строка класса завершается нулём и живёт на время вызова.
        unsafe {
            let hwnd = FindWindowW(class.as_ptr(), std::ptr::null());
            if hwnd.is_null() || IsWindowVisible(hwnd) == 0 || GetWindowRect(hwnd, &mut r) == 0 {
                return None;
            }
        }
        Some([r.left as f32, r.top as f32, (r.right - r.left) as f32, (r.bottom - r.top) as f32])
    }

    /// Включена ли экономия заряда батареи.
    pub fn low_power(&mut self) -> bool {
        use windows_sys::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
        // SAFETY: функция пишет только в переданную структуру.
        unsafe {
            let mut st: SYSTEM_POWER_STATUS = std::mem::zeroed();
            GetSystemPowerStatus(&mut st) != 0 && st.SystemStatusFlag == 1
        }
    }

    /// Идёт ли полноэкранное приложение или презентация (системное определение).
    pub fn fullscreen_app(&mut self) -> bool {
        use windows_sys::Win32::UI::Shell::SHQueryUserNotificationState;
        use windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
        // SAFETY: функции только читают состояние; `pid` — локальная переменная.
        unsafe {
            // Если на переднем плане наше приложение (например, открыто меню в трее),
            // система может принять наше же окно на весь экран за полноэкранное
            // приложение и поставить всё на паузу. Такой ответ не учитываем.
            let fg = GetForegroundWindow();
            if !fg.is_null() {
                let mut pid = 0u32;
                GetWindowThreadProcessId(fg, &mut pid);
                if pid == self.pid {
                    return false;
                }
            }
        }
        let mut state = 0;
        // SAFETY: функция пишет только в переданную переменную.
        let hr = unsafe { SHQueryUserNotificationState(&mut state) };
        // 2 — занят полноэкранным приложением, 3 — полноэкранный Direct3D, 4 — презентация
        hr == 0 && matches!(state, 2..=4)
    }

    pub fn dock_access(&self) -> Option<bool> {
        None
    }

    pub fn request_dock_access(&self) {}
}

pub fn configure_event_loop<T: 'static>(_builder: &mut EventLoopBuilder<T>) {}

pub fn window_attributes(attrs: WindowAttributes) -> WindowAttributes {
    // Без «перенаправляющего» буфера окно рисуется только через DirectComposition —
    // это и даёт настоящую прозрачность с DirectX 12.
    attrs.with_skip_taskbar(true).with_no_redirection_bitmap(true)
}

pub fn configure_window(_window: &Window) {}
