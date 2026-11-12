//! Значок и меню в трее / строке меню.
//!
//! macOS и Windows — через `tray-icon`, Linux — через `ksni` (протокол
//! StatusNotifierItem, без GTK). Снаружи обе реализации выглядят одинаково:
//! `Tray::new(proxy, state)` и `Tray::sync(state)`.

use winit::event_loop::EventLoopProxy;

use crate::settings::{PLACE_COUNT, RATES, SCENES, SKIES, WIND_DIRS, WIND_FORCES, place_label, scene_label, sky_label};

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
    /// Направление ветра: номер в `settings::WIND_DIRS`.
    Wind(usize),
    /// Сила ветра: номер в `settings::WIND_FORCES`.
    WindForce(usize),
    Rate(usize),
    Solid,
    /// Показать или спрятать украшения на окнах.
    Decor,
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
    pub wind: usize,
    pub wind_force: usize,
    pub rate: usize,
    pub solid: bool,
    pub decor: bool,
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
const L_SCENES: &str = "Осадки";
const L_SKY: &str = "Небо";
const L_WIND: &str = "Ветер";
const L_RATE: &str = "Интенсивность";
const L_SOLID: &str = "Не засыпать содержимое окон";
const L_DECOR: &str = "Украшения на окнах (деревья, кактусы)";
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
    use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
    use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

    use super::*;

    type MenuError = tray_icon::menu::Error;

    pub struct Tray {
        _icon: TrayIcon,
        running: CheckMenuItem,
        scenes: Vec<CheckMenuItem>,
        skies: Vec<CheckMenuItem>,
        places: Vec<CheckMenuItem>,
        winds: Vec<CheckMenuItem>,
        forces: Vec<CheckMenuItem>,
        rates: Vec<CheckMenuItem>,
        solid: CheckMenuItem,
        decor: CheckMenuItem,
        dock: Option<CheckMenuItem>,
        gpu: CheckMenuItem,
        adapter: MenuItem,
    }

    fn command(id: &str) -> Option<Cmd> {
        Some(match id {
            "toggle" => Cmd::Toggle,
            "solid" => Cmd::Solid,
            "decor" => Cmd::Decor,
            "shake" => Cmd::Shake,
            "gpu" => Cmd::Gpu,
            "dock" => Cmd::Dock,
            "quit" => Cmd::Quit,
            _ => {
                let num = |prefix: &str| id.strip_prefix(prefix)?.parse::<usize>().ok();
                if let Some(i) = num("scene") {
                    Cmd::Scene(i)
                } else if let Some(i) = num("place") {
                    Cmd::SkyPlace(i)
                } else if let Some(i) = num("sky") {
                    Cmd::Sky(i)
                } else if let Some(i) = num("wdir") {
                    Cmd::Wind(i)
                } else if let Some(i) = num("wforce") {
                    Cmd::WindForce(i)
                } else {
                    Cmd::Rate(num("rate")?)
                }
            }
        })
    }

    /// Добавляет в подменю группу взаимоисключающих пунктов с галочкой.
    fn radio(sub: &Submenu, prefix: &str, labels: &[&str], selected: usize) -> Result<Vec<CheckMenuItem>, MenuError> {
        labels
            .iter()
            .enumerate()
            .map(|(i, label)| {
                let item = CheckMenuItem::with_id(format!("{prefix}{i}"), label, true, selected == i, None);
                sub.append(&item)?;
                Ok(item)
            })
            .collect()
    }

    fn check(items: &[CheckMenuItem], selected: usize) {
        for (i, item) in items.iter().enumerate() {
            item.set_checked(selected == i);
        }
    }

    impl Tray {
        /// Создавать нужно в главном потоке после запуска цикла событий.
        pub fn new(proxy: EventLoopProxy<Cmd>, state: &MenuState) -> Result<Self, String> {
            let build = || -> Result<_, MenuError> {
                let menu = Menu::new();
                let sep = PredefinedMenuItem::separator;

                let running = CheckMenuItem::with_id("toggle", L_RUNNING, true, state.running, None);
                menu.append(&running)?;
                menu.append(&sep())?;

                let sub = Submenu::new(L_SCENES, true);
                let scene_labels: Vec<&str> = (0..SCENES).map(scene_label).collect();
                let scenes = radio(&sub, "scene", &scene_labels, state.scene)?;
                menu.append(&sub)?;

                let sub = Submenu::new(L_SKY, true);
                let sky_labels: Vec<&str> = (0..SKIES).map(sky_label).collect();
                let skies = radio(&sub, "sky", &sky_labels, state.sky)?;
                sub.append(&sep())?;
                let place_labels: Vec<&str> = (0..PLACE_COUNT).map(place_label).collect();
                let places = radio(&sub, "place", &place_labels, state.sky_place)?;
                menu.append(&sub)?;

                let sub = Submenu::new(L_WIND, true);
                let dir_labels: Vec<&str> = WIND_DIRS.iter().map(|d| d.0).collect();
                let winds = radio(&sub, "wdir", &dir_labels, state.wind)?;
                sub.append(&sep())?;
                let force_labels: Vec<&str> = WIND_FORCES.iter().map(|f| f.0).collect();
                let forces = radio(&sub, "wforce", &force_labels, state.wind_force)?;
                menu.append(&sub)?;

                let sub = Submenu::new(L_RATE, true);
                let rate_labels: Vec<&str> = RATES.iter().map(|r| r.0).collect();
                let rates = radio(&sub, "rate", &rate_labels, state.rate)?;
                menu.append(&sub)?;

                menu.append(&sep())?;
                let solid = CheckMenuItem::with_id("solid", L_SOLID, true, state.solid, None);
                menu.append(&solid)?;
                let decor = CheckMenuItem::with_id("decor", L_DECOR, true, state.decor, None);
                menu.append(&decor)?;
                menu.append(&MenuItem::with_id("shake", L_SHAKE, true, None))?;
                let dock = match state.dock {
                    Some(ok) => {
                        let item = CheckMenuItem::with_id("dock", L_DOCK, true, ok, None);
                        menu.append(&item)?;
                        Some(item)
                    }
                    None => None,
                };
                menu.append(&sep())?;
                let gpu = CheckMenuItem::with_id("gpu", L_GPU, state.gpu_switchable, state.gpu, None);
                menu.append(&gpu)?;
                // строка только для сведения: какой адаптер сейчас рисует
                let adapter = MenuItem::with_id("adapter", &state.adapter, false, None);
                menu.append(&adapter)?;
                menu.append(&sep())?;
                menu.append(&MenuItem::with_id("quit", L_QUIT, true, None))?;
                Ok((menu, running, scenes, skies, places, winds, forces, rates, solid, decor, dock, gpu, adapter))
            };
            let (menu, running, scenes, skies, places, winds, forces, rates, solid, decor, dock, gpu, adapter) =
                build().map_err(|e| e.to_string())?;

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

            Ok(Self { _icon: tray, running, scenes, skies, places, winds, forces, rates, solid, decor, dock, gpu, adapter })
        }

        /// Приводит галочки в соответствие с состоянием приложения.
        pub fn sync(&self, state: &MenuState) {
            self.gpu.set_checked(state.gpu);
            self.gpu.set_enabled(state.gpu_switchable);
            self.adapter.set_text(&state.adapter);
            self.running.set_checked(state.running);
            check(&self.scenes, state.scene);
            check(&self.skies, state.sky);
            check(&self.places, state.sky_place);
            check(&self.winds, state.wind);
            check(&self.forces, state.wind_force);
            check(&self.rates, state.rate);
            self.solid.set_checked(state.solid);
            self.decor.set_checked(state.decor);
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
    use ksni::menu::{CheckmarkItem, MenuItem, RadioGroup, RadioItem, StandardItem, SubMenu};

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

    /// Группа взаимоисключающих пунктов.
    fn radio(selected: usize, labels: Vec<&str>, cmd: fn(usize) -> Cmd) -> MenuItem<Model> {
        RadioGroup {
            selected,
            select: Box::new(move |m: &mut Model, i| m.send(cmd(i))),
            options: labels.into_iter().map(|l| RadioItem { label: l.into(), ..Default::default() }).collect(),
        }
        .into()
    }

    fn submenu(label: &str, items: Vec<MenuItem<Model>>) -> MenuItem<Model> {
        SubMenu { label: label.into(), submenu: items, ..Default::default() }.into()
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
            let s = &self.state;
            vec![
                CheckmarkItem {
                    label: L_RUNNING.into(),
                    checked: s.running,
                    activate: Box::new(|m: &mut Self| m.send(Cmd::Toggle)),
                    ..Default::default()
                }
                .into(),
                MenuItem::Separator,
                submenu(L_SCENES, vec![radio(s.scene, (0..SCENES).map(scene_label).collect(), Cmd::Scene)]),
                submenu(
                    L_SKY,
                    vec![
                        radio(s.sky, (0..SKIES).map(sky_label).collect(), Cmd::Sky),
                        MenuItem::Separator,
                        radio(s.sky_place, (0..PLACE_COUNT).map(place_label).collect(), Cmd::SkyPlace),
                    ],
                ),
                submenu(
                    L_WIND,
                    vec![
                        radio(s.wind, WIND_DIRS.iter().map(|d| d.0).collect(), Cmd::Wind),
                        MenuItem::Separator,
                        radio(s.wind_force, WIND_FORCES.iter().map(|f| f.0).collect(), Cmd::WindForce),
                    ],
                ),
                submenu(L_RATE, vec![radio(s.rate, RATES.iter().map(|r| r.0).collect(), Cmd::Rate)]),
                MenuItem::Separator,
                CheckmarkItem {
                    label: L_SOLID.into(),
                    checked: s.solid,
                    activate: Box::new(|m: &mut Self| m.send(Cmd::Solid)),
                    ..Default::default()
                }
                .into(),
                CheckmarkItem {
                    label: L_DECOR.into(),
                    checked: s.decor,
                    activate: Box::new(|m: &mut Self| m.send(Cmd::Decor)),
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
                    checked: s.gpu,
                    enabled: s.gpu_switchable,
                    activate: Box::new(|m: &mut Self| m.send(Cmd::Gpu)),
                    ..Default::default()
                }
                .into(),
                StandardItem { label: s.adapter.clone(), enabled: false, ..Default::default() }.into(),
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
