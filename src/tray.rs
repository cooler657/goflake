//! Значок и меню в трее / строке меню.
//!
//! macOS и Windows — через `tray-icon`, Linux — через `ksni` (протокол
//! StatusNotifierItem, без GTK). Снаружи обе реализации выглядят одинаково:
//! `Tray::new(proxy, state)` и `Tray::sync(state)`.

use winit::event_loop::EventLoopProxy;

use crate::settings::{PLACE_COUNT, RATES, SCENES, SKIES, place_label, scene_label, sky_label};

/// Команда из меню. Она же — пользовательское событие цикла событий.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Cmd {
    Toggle,
    /// Что падает: 0 — по времени года, дальше материалы.
    Scene(usize),
    /// Что на небе: номер в `sky::MODES`.
    Sky(usize),
    /// Где находится небо: номер в `sky::PLACES`.
    SkyPlace(usize),
    Rate(usize),
    Solid,
    Shake,
    /// Переключить аппаратное ускорение.
    Gpu,
    /// Запросить доступ к Dock (пункт есть только на macOS).
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    Dock,
    Quit,
}

/// То, что меню должно показывать галочками.
#[derive(Clone, Debug, PartialEq)]
pub struct MenuState {
    pub running: bool,
    pub scene: usize,
    pub sky: usize,
    pub sky_place: usize,
    pub rate: usize,
    pub solid: bool,
    /// `None` — пункта про Dock нет (разрешение не требуется).
    pub dock: Option<bool>,
    /// Отрисовка идёт на видеокарте.
    pub gpu: bool,
    /// Можно ли переключаться между видеокартой и программной отрисовкой.
    pub gpu_switchable: bool,
    /// Название используемого графического адаптера.
    pub adapter: String,
}

const L_RUNNING: &str = "Включено";
const L_SOLID: &str = "Не засыпать содержимое окон";
const L_SHAKE: &str = "Стряхнуть всё";
const L_DOCK: &str = "Ложиться на Dock (нужен Универсальный доступ)";
const L_GPU: &str = "Аппаратное ускорение (GPU)";
const L_QUIT: &str = "Выйти";

/// Рисует снежинку: шесть лучей с веточками. Возвращает RGBA, `size`×`size`.
fn snowflake_rgba(size: u32, color: [u8; 3]) -> Vec<u8> {
    let c = size as f32 / 2.0;
    let radius = c * 0.88;
    let thick = (size as f32 / 22.0).max(0.9);
    let mut segments = Vec::new();
    for k in 0..6 {
        let a = std::f32::consts::FRAC_PI_3 * k as f32 + std::f32::consts::FRAC_PI_2;
        let (dx, dy) = (a.cos(), a.sin());
        segments.push(((c, c), (c + dx * radius, c + dy * radius)));
        let (bx, by) = (c + dx * radius * 0.55, c + dy * radius * 0.55);
        for side in [-1.0f32, 1.0] {
            let b = a + side * std::f32::consts::FRAC_PI_3;
            segments.push(((bx, by), (bx + b.cos() * radius * 0.3, by + b.sin() * radius * 0.3)));
        }
    }
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let mut dist = f32::MAX;
            for &((ax, ay), (bx, by)) in &segments {
                let (vx, vy) = (bx - ax, by - ay);
                let t = (((px - ax) * vx + (py - ay) * vy) / (vx * vx + vy * vy)).clamp(0.0, 1.0);
                let (qx, qy) = (ax + vx * t - px, ay + vy * t - py);
                dist = dist.min((qx * qx + qy * qy).sqrt());
            }
            let alpha = (thick - dist + 0.5).clamp(0.0, 1.0);
            out.extend_from_slice(&[color[0], color[1], color[2], (alpha * 255.0) as u8]);
        }
    }
    out
}

// ---- macOS и Windows -------------------------------------------------------

#[cfg(any(target_os = "macos", target_os = "windows"))]
mod imp {
    use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

    use super::*;

    pub struct Tray {
        _icon: TrayIcon,
        running: CheckMenuItem,
        scenes: Vec<CheckMenuItem>,
        skies: Vec<CheckMenuItem>,
        places: Vec<CheckMenuItem>,
        rates: Vec<CheckMenuItem>,
        solid: CheckMenuItem,
        dock: Option<CheckMenuItem>,
        gpu: CheckMenuItem,
        adapter: MenuItem,
    }

    fn command(id: &str) -> Option<Cmd> {
        Some(match id {
            "toggle" => Cmd::Toggle,
            "solid" => Cmd::Solid,
            "shake" => Cmd::Shake,
            "gpu" => Cmd::Gpu,
            "dock" => Cmd::Dock,
            "quit" => Cmd::Quit,
            _ => {
                if let Some(i) = id.strip_prefix("scene") {
                    Cmd::Scene(i.parse().ok()?)
                } else if let Some(i) = id.strip_prefix("place") {
                    Cmd::SkyPlace(i.parse().ok()?)
                } else if let Some(i) = id.strip_prefix("sky") {
                    Cmd::Sky(i.parse().ok()?)
                } else {
                    Cmd::Rate(id.strip_prefix("rate")?.parse().ok()?)
                }
            }
        })
    }

    impl Tray {
        /// Создавать нужно в главном потоке после запуска цикла событий.
        pub fn new(proxy: EventLoopProxy<Cmd>, state: &MenuState) -> Result<Self, String> {
            let err = |e: tray_icon::menu::Error| e.to_string();
            let menu = Menu::new();
            let sep = PredefinedMenuItem::separator;

            let running = CheckMenuItem::with_id("toggle", L_RUNNING, true, state.running, None);
            menu.append(&running).map_err(err)?;
            menu.append(&sep()).map_err(err)?;
            let mut scenes = Vec::new();
            for i in 0..SCENES {
                let item = CheckMenuItem::with_id(format!("scene{i}"), scene_label(i), true, state.scene == i, None);
                menu.append(&item).map_err(err)?;
                scenes.push(item);
            }
            menu.append(&sep()).map_err(err)?;
            let mut skies = Vec::new();
            for i in 0..SKIES {
                let item = CheckMenuItem::with_id(format!("sky{i}"), sky_label(i), true, state.sky == i, None);
                menu.append(&item).map_err(err)?;
                skies.push(item);
            }
            menu.append(&sep()).map_err(err)?;
            let mut places = Vec::new();
            for i in 0..PLACE_COUNT {
                let item = CheckMenuItem::with_id(format!("place{i}"), place_label(i), true, state.sky_place == i, None);
                menu.append(&item).map_err(err)?;
                places.push(item);
            }
            menu.append(&sep()).map_err(err)?;
            let mut rates = Vec::new();
            for (i, (label, _)) in RATES.iter().enumerate() {
                let item = CheckMenuItem::with_id(format!("rate{i}"), label, true, state.rate == i, None);
                menu.append(&item).map_err(err)?;
                rates.push(item);
            }
            menu.append(&sep()).map_err(err)?;
            let solid = CheckMenuItem::with_id("solid", L_SOLID, true, state.solid, None);
            menu.append(&solid).map_err(err)?;
            menu.append(&MenuItem::with_id("shake", L_SHAKE, true, None)).map_err(err)?;
            let dock = match state.dock {
                Some(ok) => {
                    let item = CheckMenuItem::with_id("dock", L_DOCK, true, ok, None);
                    menu.append(&item).map_err(err)?;
                    Some(item)
                }
                None => None,
            };
            menu.append(&sep()).map_err(err)?;
            let gpu = CheckMenuItem::with_id("gpu", L_GPU, state.gpu_switchable, state.gpu, None);
            menu.append(&gpu).map_err(err)?;
            // строка только для сведения: какой адаптер сейчас рисует
            let adapter = MenuItem::with_id("adapter", &state.adapter, false, None);
            menu.append(&adapter).map_err(err)?;
            menu.append(&sep()).map_err(err)?;
            menu.append(&MenuItem::with_id("quit", L_QUIT, true, None)).map_err(err)?;

            MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
                if let Some(cmd) = command(e.id.0.as_str()) {
                    let _ = proxy.send_event(cmd);
                }
            }));

            let size = 44;
            let icon = |color| Icon::from_rgba(snowflake_rgba(size, color), size, size).map_err(|e| e.to_string());
            let builder = TrayIconBuilder::new().with_menu(Box::new(menu)).with_tooltip("Сугроб");
            // На macOS значок-шаблон: система сама красит его под светлую и тёмную строку меню.
            #[cfg(target_os = "macos")]
            let builder = builder.with_icon_templated(icon([0, 0, 0])?);
            #[cfg(not(target_os = "macos"))]
            let builder = builder.with_icon(icon([205, 230, 255])?);
            let tray = builder.build().map_err(|e| e.to_string())?;

            Ok(Self { _icon: tray, running, scenes, skies, places, rates, solid, dock, gpu, adapter })
        }

        /// Приводит галочки в соответствие с состоянием приложения.
        pub fn sync(&self, state: &MenuState) {
            self.gpu.set_checked(state.gpu);
            self.gpu.set_enabled(state.gpu_switchable);
            self.adapter.set_text(&state.adapter);
            self.running.set_checked(state.running);
            for (i, item) in self.scenes.iter().enumerate() {
                item.set_checked(state.scene == i);
            }
            for (i, item) in self.skies.iter().enumerate() {
                item.set_checked(state.sky == i);
            }
            for (i, item) in self.places.iter().enumerate() {
                item.set_checked(state.sky_place == i);
            }
            for (i, item) in self.rates.iter().enumerate() {
                item.set_checked(state.rate == i);
            }
            self.solid.set_checked(state.solid);
            if let (Some(item), Some(ok)) = (&self.dock, state.dock) {
                item.set_checked(ok);
            }
        }
    }
}

// ---- Linux -----------------------------------------------------------------

#[cfg(target_os = "linux")]
mod imp {
    use ksni::blocking::{Handle, TrayMethods};
    use ksni::menu::{CheckmarkItem, MenuItem, RadioGroup, RadioItem, StandardItem};

    use super::*;

    struct Model {
        proxy: EventLoopProxy<Cmd>,
        state: MenuState,
    }

    impl Model {
        fn send(&self, cmd: Cmd) {
            let _ = self.proxy.send_event(cmd);
        }
    }

    impl ksni::Tray for Model {
        fn id(&self) -> String {
            "snowdrift".into()
        }

        fn title(&self) -> String {
            "Сугроб".into()
        }

        fn icon_pixmap(&self) -> Vec<ksni::Icon> {
            let size = 32;
            let mut data = snowflake_rgba(size as u32, [205, 230, 255]);
            // RGBA -> ARGB (сетевой порядок байтов)
            for px in data.chunks_exact_mut(4) {
                px.rotate_right(1);
            }
            vec![ksni::Icon { width: size, height: size, data }]
        }

        fn menu(&self) -> Vec<MenuItem<Self>> {
            vec![
                CheckmarkItem {
                    label: L_RUNNING.into(),
                    checked: self.state.running,
                    activate: Box::new(|m: &mut Self| m.send(Cmd::Toggle)),
                    ..Default::default()
                }
                .into(),
                MenuItem::Separator,
                RadioGroup {
                    selected: self.state.scene,
                    select: Box::new(|m: &mut Self, i| m.send(Cmd::Scene(i))),
                    options: (0..SCENES)
                        .map(|i| RadioItem { label: scene_label(i).into(), ..Default::default() })
                        .collect(),
                }
                .into(),
                MenuItem::Separator,
                RadioGroup {
                    selected: self.state.sky,
                    select: Box::new(|m: &mut Self, i| m.send(Cmd::Sky(i))),
                    options: (0..SKIES)
                        .map(|i| RadioItem { label: sky_label(i).into(), ..Default::default() })
                        .collect(),
                }
                .into(),
                MenuItem::Separator,
                RadioGroup {
                    selected: self.state.sky_place,
                    select: Box::new(|m: &mut Self, i| m.send(Cmd::SkyPlace(i))),
                    options: (0..PLACE_COUNT)
                        .map(|i| RadioItem { label: place_label(i).into(), ..Default::default() })
                        .collect(),
                }
                .into(),
                MenuItem::Separator,
                RadioGroup {
                    selected: self.state.rate,
                    select: Box::new(|m: &mut Self, i| m.send(Cmd::Rate(i))),
                    options: RATES
                        .iter()
                        .map(|(label, _)| RadioItem { label: (*label).into(), ..Default::default() })
                        .collect(),
                }
                .into(),
                MenuItem::Separator,
                CheckmarkItem {
                    label: L_SOLID.into(),
                    checked: self.state.solid,
                    activate: Box::new(|m: &mut Self| m.send(Cmd::Solid)),
                    ..Default::default()
                }
                .into(),
                StandardItem {
                    label: L_SHAKE.into(),
                    activate: Box::new(|m: &mut Self| m.send(Cmd::Shake)),
                    ..Default::default()
                }
                .into(),
                MenuItem::Separator,
                CheckmarkItem {
                    label: L_GPU.into(),
                    checked: self.state.gpu,
                    enabled: self.state.gpu_switchable,
                    activate: Box::new(|m: &mut Self| m.send(Cmd::Gpu)),
                    ..Default::default()
                }
                .into(),
                StandardItem { label: self.state.adapter.clone(), enabled: false, ..Default::default() }.into(),
                MenuItem::Separator,
                StandardItem {
                    label: L_QUIT.into(),
                    activate: Box::new(|m: &mut Self| m.send(Cmd::Quit)),
                    ..Default::default()
                }
                .into(),
            ]
        }
    }

    pub struct Tray {
        handle: Handle<Model>,
    }

    impl Tray {
        pub fn new(proxy: EventLoopProxy<Cmd>, state: &MenuState) -> Result<Self, String> {
            let state = state.clone();
            let _ = L_DOCK; // на Linux панель видна без разрешений, пункт не нужен
            let handle = Model { proxy, state }.spawn().map_err(|e| e.to_string())?;
            Ok(Self { handle })
        }

        pub fn sync(&self, state: &MenuState) {
            let state = state.clone();
            self.handle.update(move |m: &mut Model| m.state = state);
        }
    }
}

pub use imp::Tray;
