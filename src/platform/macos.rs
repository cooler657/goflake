//! macOS: CoreGraphics для окон и курсора, Accessibility для Dock.
//!
//! Системные функции вызываются напрямую через FFI — их немного, и это избавляет
//! от лишних зависимостей.

use std::ffi::{CStr, c_char, c_void};
use std::ptr::null;
use std::time::{Duration, Instant};

use objc2::encode::{Encode, Encoding};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};
use winit::event_loop::EventLoopBuilder;
use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS, WindowAttributesExtMacOS};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::{Window, WindowAttributes};

use super::RawWindow;
use crate::sim::Sim;
use crate::sky::Sky;
use crate::soft::Canvas;

pub const PHYSICAL: bool = false;
pub const DOCK_LIFT: f32 = 5.0;

type CFRef = *const c_void;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CGSize {
    w: f64,
    h: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}

// SAFETY: раскладка и кодировки совпадают с системными CGPoint, CGSize и CGRect.
unsafe impl Encode for CGPoint {
    const ENCODING: Encoding = Encoding::Struct("CGPoint", &[f64::ENCODING, f64::ENCODING]);
}
unsafe impl Encode for CGSize {
    const ENCODING: Encoding = Encoding::Struct("CGSize", &[f64::ENCODING, f64::ENCODING]);
}
unsafe impl Encode for CGRect {
    const ENCODING: Encoding = Encoding::Struct("CGRect", &[CGPoint::ENCODING, CGSize::ENCODING]);
}

#[repr(C)]
struct Opaque {
    _private: [u8; 0],
}

const UTF8: u32 = 0x0800_0100;
const NUM_I64: isize = 4;
const NUM_F64: isize = 6;
const AX_POINT: u32 = 1;
const AX_SIZE: u32 = 2;
const LIST_ON_SCREEN: u32 = 1 << 0;
const LIST_NO_DESKTOP: u32 = 1 << 4;

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFTypeDictionaryKeyCallBacks: Opaque;
    static kCFTypeDictionaryValueCallBacks: Opaque;
    static kCFBooleanTrue: CFRef;

    fn CFRelease(obj: CFRef);
    fn CFGetTypeID(obj: CFRef) -> usize;
    fn CFArrayGetCount(array: CFRef) -> isize;
    fn CFArrayGetValueAtIndex(array: CFRef, index: isize) -> CFRef;
    fn CFDictionaryGetValue(dict: CFRef, key: CFRef) -> CFRef;
    fn CFDictionaryCreate(
        alloc: CFRef,
        keys: *const CFRef,
        values: *const CFRef,
        count: isize,
        key_callbacks: *const Opaque,
        value_callbacks: *const Opaque,
    ) -> CFRef;
    fn CFNumberGetValue(number: CFRef, kind: isize, out: *mut c_void) -> u8;
    fn CFStringCreateWithCString(alloc: CFRef, s: *const c_char, encoding: u32) -> CFRef;
    fn CFStringGetCString(s: CFRef, buf: *mut c_char, size: isize, encoding: u32) -> u8;
    fn CFDataCreate(alloc: CFRef, bytes: *const u8, length: isize) -> CFRef;
}

// CALayer и CATransaction живут в QuartzCore.
#[link(name = "QuartzCore", kind = "framework")]
unsafe extern "C" {}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    static kCGWindowNumber: CFRef;
    static kCGWindowLayer: CFRef;
    static kCGWindowBounds: CFRef;
    static kCGWindowAlpha: CFRef;
    static kCGWindowOwnerPID: CFRef;
    static kCGWindowOwnerName: CFRef;

    fn CGWindowListCopyWindowInfo(option: u32, relative_to: u32) -> CFRef;
    fn CGRectMakeWithDictionaryRepresentation(dict: CFRef, rect: *mut CGRect) -> u8;
    fn CGEventCreate(source: CFRef) -> CFRef;
    fn CGEventGetLocation(event: CFRef) -> CGPoint;
    fn CGColorSpaceCreateDeviceRGB() -> CFRef;
    fn CGDataProviderCreateWithCFData(data: CFRef) -> CFRef;
    #[allow(clippy::too_many_arguments)]
    fn CGImageCreate(
        width: usize,
        height: usize,
        bits_per_component: usize,
        bits_per_pixel: usize,
        bytes_per_row: usize,
        space: CFRef,
        bitmap_info: u32,
        provider: CFRef,
        decode: *const f64,
        should_interpolate: u8,
        intent: i32,
    ) -> CFRef;
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    static kAXTrustedCheckOptionPrompt: CFRef;

    fn AXIsProcessTrusted() -> u8;
    fn AXIsProcessTrustedWithOptions(options: CFRef) -> u8;
    fn AXUIElementCreateApplication(pid: i32) -> CFRef;
    fn AXUIElementCopyAttributeValue(element: CFRef, attribute: CFRef, value: *mut CFRef) -> i32;
    fn AXValueGetTypeID() -> usize;
    fn AXValueGetValue(value: CFRef, kind: u32, out: *mut c_void) -> u8;
}

unsafe fn dict_i64(dict: CFRef, key: CFRef) -> Option<i64> {
    unsafe {
        let v = CFDictionaryGetValue(dict, key);
        let mut out = 0i64;
        (!v.is_null() && CFNumberGetValue(v, NUM_I64, (&raw mut out).cast()) != 0).then_some(out)
    }
}

unsafe fn dict_f64(dict: CFRef, key: CFRef) -> Option<f64> {
    unsafe {
        let v = CFDictionaryGetValue(dict, key);
        let mut out = 0f64;
        (!v.is_null() && CFNumberGetValue(v, NUM_F64, (&raw mut out).cast()) != 0).then_some(out)
    }
}

/// Читает атрибут Accessibility. Вызывающий обязан освободить результат.
unsafe fn ax_attr(element: CFRef, name: &CStr) -> Option<CFRef> {
    unsafe {
        let key = CFStringCreateWithCString(null(), name.as_ptr(), UTF8);
        if key.is_null() {
            return None;
        }
        let mut value: CFRef = null();
        let err = AXUIElementCopyAttributeValue(element, key, &mut value);
        CFRelease(key);
        (err == 0 && !value.is_null()).then_some(value)
    }
}

pub struct Platform {
    pid: i64,
    dock_pid: Option<i32>,
}

impl Platform {
    pub fn new() -> Result<Self, String> {
        Ok(Self { pid: std::process::id() as i64, dock_pid: None })
    }

    pub fn windows(&mut self) -> Vec<RawWindow> {
        let mut out = Vec::new();
        // SAFETY: обычные вызовы CoreGraphics / CoreFoundation; массив освобождается ниже,
        // значения из него живут, пока жив массив.
        unsafe {
            let list = CGWindowListCopyWindowInfo(LIST_ON_SCREEN | LIST_NO_DESKTOP, 0);
            if list.is_null() {
                return out;
            }
            for i in 0..CFArrayGetCount(list) {
                let w = CFArrayGetValueAtIndex(list, i);
                let pid = dict_i64(w, kCGWindowOwnerPID).unwrap_or(0);
                if self.dock_pid.is_none() {
                    let name = CFDictionaryGetValue(w, kCGWindowOwnerName);
                    let mut buf = [0 as c_char; 64];
                    if !name.is_null()
                        && CFStringGetCString(name, buf.as_mut_ptr(), buf.len() as isize, UTF8) != 0
                        && CStr::from_ptr(buf.as_ptr()).to_bytes() == b"Dock"
                    {
                        self.dock_pid = Some(pid as i32);
                    }
                }
                // слой 0 — обычные окна приложений
                if dict_i64(w, kCGWindowLayer) != Some(0)
                    || pid == self.pid
                    || dict_f64(w, kCGWindowAlpha).unwrap_or(1.0) <= 0.05
                {
                    continue;
                }
                let bounds = CFDictionaryGetValue(w, kCGWindowBounds);
                let mut r = CGRect::default();
                if bounds.is_null() || CGRectMakeWithDictionaryRepresentation(bounds, &mut r) == 0 {
                    continue;
                }
                if let Some(id) = dict_i64(w, kCGWindowNumber) {
                    out.push(RawWindow {
                        id,
                        rect: [r.origin.x as f32, r.origin.y as f32, r.size.w as f32, r.size.h as f32],
                    });
                }
            }
            CFRelease(list);
        }
        out
    }

    pub fn cursor(&mut self) -> Option<(f32, f32)> {
        // SAFETY: событие создаётся и освобождается здесь же.
        unsafe {
            let ev = CGEventCreate(null());
            if ev.is_null() {
                return None;
            }
            let p = CGEventGetLocation(ev);
            CFRelease(ev);
            Some((p.x as f32, p.y as f32))
        }
    }

    /// Рамка Dock через Accessibility — только если доступ выдан.
    pub fn dock(&mut self) -> Option<[f32; 4]> {
        let pid = self.dock_pid?;
        // SAFETY: все полученные объекты освобождаются; типы значений проверяются.
        unsafe {
            if AXIsProcessTrusted() == 0 {
                return None;
            }
            let app = AXUIElementCreateApplication(pid);
            if app.is_null() {
                return None;
            }
            let mut result = None;
            if let Some(kids) = ax_attr(app, c"AXChildren") {
                if CFArrayGetCount(kids) > 0 {
                    let list = CFArrayGetValueAtIndex(kids, 0);
                    let pos = ax_attr(list, c"AXPosition");
                    let size = ax_attr(list, c"AXSize");
                    if let (Some(pv), Some(sv)) = (pos, size) {
                        let mut p = CGPoint::default();
                        let mut s = CGSize::default();
                        if CFGetTypeID(pv) == AXValueGetTypeID()
                            && CFGetTypeID(sv) == AXValueGetTypeID()
                            && AXValueGetValue(pv, AX_POINT, (&raw mut p).cast()) != 0
                            && AXValueGetValue(sv, AX_SIZE, (&raw mut s).cast()) != 0
                        {
                            result = Some([p.x as f32, p.y as f32, s.w as f32, s.h as f32]);
                        }
                    }
                    for v in [pos, size].into_iter().flatten() {
                        CFRelease(v);
                    }
                }
                CFRelease(kids);
            }
            CFRelease(app);
            result
        }
    }

    pub fn dock_access(&self) -> Option<bool> {
        // SAFETY: функция без аргументов и побочных эффектов.
        Some(unsafe { AXIsProcessTrusted() } != 0)
    }

    /// Показывает системный запрос на «Универсальный доступ».
    pub fn request_dock_access(&self) {
        // SAFETY: словарь создаётся из статических ключей и освобождается здесь же.
        unsafe {
            let keys = [kAXTrustedCheckOptionPrompt];
            let values = [kCFBooleanTrue];
            let opts = CFDictionaryCreate(
                null(),
                keys.as_ptr(),
                values.as_ptr(),
                1,
                &raw const kCFTypeDictionaryKeyCallBacks,
                &raw const kCFTypeDictionaryValueCallBacks,
            );
            if !opts.is_null() {
                AXIsProcessTrustedWithOptions(opts);
                CFRelease(opts);
            }
        }
    }
}

/// Приложение без значка в Dock и без собственного меню.
pub fn configure_event_loop<T: 'static>(builder: &mut EventLoopBuilder<T>) {
    builder.with_activation_policy(ActivationPolicy::Accessory);
}

pub fn window_attributes(attrs: WindowAttributes) -> WindowAttributes {
    attrs.with_has_shadow(false)
}

/// Поднимает окно на уровень строки меню и показывает его на всех рабочих столах.
pub fn configure_window(window: &Window) {
    let Ok(handle) = window.window_handle() else { return };
    let RawWindowHandle::AppKit(h) = handle.as_raw() else { return };
    let view: *mut AnyObject = h.ns_view.as_ptr().cast();
    // SAFETY: `view` — живой NSView этого окна; вызовы идут из главного потока,
    // селекторы и типы аргументов соответствуют NSWindow.
    unsafe {
        let ns_window: *mut AnyObject = msg_send![view, window];
        if ns_window.is_null() {
            return;
        }
        const STATUS_BAR_LEVEL: isize = 25;
        // canJoinAllSpaces | stationary | ignoresCycle | fullScreenAuxiliary
        const BEHAVIOR: usize = (1 << 0) | (1 << 4) | (1 << 6) | (1 << 8);
        let _: () = msg_send![ns_window, setLevel: STATUS_BAR_LEVEL];
        let _: () = msg_send![ns_window, setCollectionBehavior: BEHAVIOR];
        let _: () = msg_send![ns_window, setIgnoresMouseEvents: true];
    }
}

/// NSView окна winit.
fn ns_view(window: &Window) -> Option<*mut AnyObject> {
    let handle = window.window_handle().ok()?;
    let RawWindowHandle::AppKit(h) = handle.as_raw() else { return None };
    Some(h.ns_view.as_ptr().cast())
}

/// Программная отрисовка на macOS: кадр рисуется на процессоре (`soft::Canvas`)
/// и отдаётся системе готовой картинкой в слое окна. Metal не используется.
///
/// Растр считается в логических точках, без удвоения под Retina: так кадр
/// вчетверо легче, а мягкости осадкам это не вредит.
pub struct SoftSurface {
    view: *mut AnyObject,
    layer: Retained<AnyObject>,
    color_space: CFRef,
    canvas: Canvas,
    bytes: Vec<u8>,
    last_frame: Instant,
}

/// Сколько длится кадр программной отрисовки (60 кадров в секунду).
const SOFT_FRAME: Duration = Duration::from_micros(16_667);
/// kCGImageAlphaPremultipliedFirst | kCGBitmapByteOrder32Little — байты B, G, R, A.
const BGRA_PREMULTIPLIED: u32 = 2 | (2 << 12);

impl SoftSurface {
    pub fn new(window: &Window) -> Result<Self, String> {
        let view = ns_view(window).ok_or("у окна нет NSView")?;
        // SAFETY: `view` — живой NSView этого окна, вызовы идут из главного потока;
        // селекторы и типы аргументов соответствуют NSView и CALayer.
        unsafe {
            let _: () = msg_send![view, setWantsLayer: true];
            let root: *mut AnyObject = msg_send![view, layer];
            if root.is_null() {
                return Err("у окна нет слоя для отрисовки".into());
            }
            // убираем слой, оставшийся от отрисовки на видеокарте
            let _: () = msg_send![root, setSublayers: null::<AnyObject>()];
            let layer: Retained<AnyObject> = msg_send![class!(CALayer), new];
            let _: () = msg_send![root, addSublayer: &*layer];
            Ok(Self {
                view,
                layer,
                color_space: CGColorSpaceCreateDeviceRGB(),
                canvas: Canvas::new(),
                bytes: Vec::new(),
                last_frame: Instant::now(),
            })
        }
    }

    /// Рисует и показывает кадр, затем выдерживает паузу до следующего.
    pub fn render(&mut self, sim: &Sim, sky: &Sky) -> bool {
        self.canvas.draw(sim, sky, 1.0);
        let (w, h) = (self.canvas.width, self.canvas.height);
        // SAFETY: слой и вид живы, пока жив `self`; все созданные объекты
        // CoreFoundation освобождаются; данные картинки копируются в CFData.
        unsafe {
            // Строки идут сверху вниз, как есть. Слой сообщает contentsAreFlipped,
            // но картинку-содержимое система показывает правильно сама: переворачивать
            // строки нельзя — осадки пойдут снизу вверх.
            // На Mac порядок байтов младший-первый: u32 0xAARRGGBB — это уже B, G, R, A.
            self.bytes.clear();
            self.bytes.extend_from_slice(bytemuck::cast_slice(&self.canvas.pixels[..w * h]));
            let data = CFDataCreate(null(), self.bytes.as_ptr(), self.bytes.len() as isize);
            let provider = CGDataProviderCreateWithCFData(data);
            let image =
                CGImageCreate(w, h, 8, 32, w * 4, self.color_space, BGRA_PREMULTIPLIED, provider, null(), 0, 0);
            let root: *mut AnyObject = msg_send![self.view, layer];
            let bounds: CGRect = msg_send![root, bounds];
            // без неявных анимаций: иначе кадры плавно «перетекали» бы друг в друга
            let _: () = msg_send![class!(CATransaction), begin];
            let _: () = msg_send![class!(CATransaction), setDisableActions: true];
            let _: () = msg_send![&*self.layer, setFrame: bounds];
            let _: () = msg_send![&*self.layer, setContents: image as *mut AnyObject];
            let _: () = msg_send![class!(CATransaction), commit];
            for obj in [image, provider, data] {
                if !obj.is_null() {
                    CFRelease(obj);
                }
            }
        }
        // вертикальной синхронизации здесь нет — выдерживаем темп сами
        let elapsed = self.last_frame.elapsed();
        if elapsed < SOFT_FRAME {
            std::thread::sleep(SOFT_FRAME - elapsed);
        }
        self.last_frame = Instant::now();
        true
    }
}

impl Drop for SoftSurface {
    fn drop(&mut self) {
        // SAFETY: слой жив; цветовое пространство создано в `new` и больше не нужно.
        unsafe {
            let _: () = msg_send![&*self.layer, removeFromSuperlayer];
            if !self.color_space.is_null() {
                CFRelease(self.color_space);
            }
        }
    }
}
