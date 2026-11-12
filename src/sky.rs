//! Небо: днём солнце с лучами, ночью луна со звёздами.
//!
//! Не зависит ни от ОС, ни от отрисовки. По времени суток и дате считает, что
//! и где нарисовать, и отдаёт список фигур `FlakeLook` — как и частицы.
//!
//! Солнце и луна идут по дуге вдоль верха экрана: у горизонта они ниже и
//! краснее, в зените — выше. Время восхода и заката зависит от времени года.

use std::f32::consts::{PI, TAU};

use crate::clock::Clock;
use crate::look::{FlakeLook, FlakeShape, hash01};

/// Радиус диска солнца, точки.
pub const SUN_R: f32 = 22.0;
/// Сколько лучей у солнца.
const BEAMS: usize = 24;

/// Сколько фигур небо может отдать за кадр (под него запасён буфер на GPU).
pub const SKY_MAX: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkyMode {
    /// Неба нет.
    Off,
    /// Днём солнце, ночью луна со звёздами.
    Auto,
    /// Всегда солнце.
    Sun,
    /// Всегда луна со звёздами.
    Moon,
}

pub const MODES: [SkyMode; 4] = [SkyMode::Off, SkyMode::Auto, SkyMode::Sun, SkyMode::Moon];

impl SkyMode {
    /// Название в меню.
    pub fn label(self) -> &'static str {
        match self {
            SkyMode::Off => "Без неба",
            SkyMode::Auto => "Небо по времени суток",
            SkyMode::Sun => "Всегда солнце",
            SkyMode::Moon => "Всегда луна со звёздами",
        }
    }
}

/// Где на экране находится солнце или луна.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkyPlace {
    /// Идёт по дуге вдоль верха экрана вслед за временем суток.
    Arc,
    /// Стоит в левом верхнем углу.
    Left,
    /// Стоит в правом верхнем углу.
    Right,
}

pub const PLACES: [SkyPlace; 3] = [SkyPlace::Arc, SkyPlace::Left, SkyPlace::Right];

impl SkyPlace {
    /// Название в меню.
    pub fn label(self) -> &'static str {
        match self {
            SkyPlace::Arc => "По дуге вдоль верха экрана",
            SkyPlace::Left => "В левом верхнем углу",
            SkyPlace::Right => "В правом верхнем углу",
        }
    }
}

/// Отступ тела от бокового края экрана в углу, точки.
const CORNER_X: f32 = 90.0;
/// Высота центра луны над экраном: ниже строки меню вместе со свечением.
const TOP_Y: f32 = 96.0;
/// Высота центра солнца: выше луны, у самого верха экрана, диск — под строкой меню.
const SUN_TOP_Y: f32 = 58.0;

/// Время восхода и заката, часы. Дни длиннее летом и короче зимой
/// (северное полушарие); полдень — в 12:30.
pub fn sun_hours(season: f32) -> (f32, f32) {
    // +1 в середине июня, −1 в середине декабря
    let s = (TAU * (season - 5.5) / 12.0).cos();
    let half_day = 6.25 + 2.25 * s;
    (12.5 - half_day, 12.5 + half_day)
}

/// Фаза луны: 0 — новолуние, 0.5 — полнолуние. Средняя синодическая длина
/// месяца; точность — около суток.
pub fn moon_phase(unix: f64) -> f32 {
    const NEW_MOON: f64 = 947_182_440.0; // 6 января 2000, 18:14 UTC
    const MONTH_DAYS: f64 = 29.530_588_853;
    (((unix - NEW_MOON) / 86_400.0 / MONTH_DAYS).rem_euclid(1.0)) as f32
}

/// Что на небе в данный момент.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Celestial {
    pub moon: bool,
    /// Положение на дуге: 0 — у левого края экрана, 1 — у правого.
    pub t: f32,
    /// Видимость: плавно нарастает на восходе и закате.
    pub alpha: f32,
}

/// Плавный переход через `edge` часов у восхода и заката.
const FADE_HOURS: f32 = 0.75;

fn smooth(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

pub fn celestial(mode: SkyMode, clock: Clock) -> Option<Celestial> {
    let (rise, set) = sun_hours(clock.season);
    let h = clock.hour;
    let day = h >= rise && h <= set;
    let night_len = 24.0 - (set - rise);
    let after_set = (h - set).rem_euclid(24.0);

    let sun = |forced: bool| {
        let (t, alpha) = if day {
            ((h - rise) / (set - rise), smooth((h - rise).min(set - h) / FADE_HOURS))
        } else {
            (0.5, 1.0)
        };
        Celestial { moon: false, t, alpha: if forced { 1.0 } else { alpha } }
    };
    let moon = |forced: bool| {
        let (t, alpha) = if !day {
            (after_set / night_len, smooth(after_set.min(night_len - after_set) / FADE_HOURS))
        } else {
            (0.5, 1.0)
        };
        Celestial { moon: true, t, alpha: if forced { 1.0 } else { alpha } }
    };
    match mode {
        SkyMode::Off => None,
        SkyMode::Auto => Some(if day { sun(false) } else { moon(false) }),
        SkyMode::Sun => Some(sun(true)),
        SkyMode::Moon => Some(moon(true)),
    }
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

fn shape(center: [f32; 2], half: [f32; 2], rotation: f32, shape: FlakeShape, rgb: [f32; 3], alpha: f32) -> FlakeLook {
    FlakeLook { center, half, rotation, shape, color: [rgb[0], rgb[1], rgb[2], alpha] }
}

pub struct Sky {
    mode: SkyMode,
    place: SkyPlace,
    clock: Clock,
    phase: f32,
    /// Время для анимации (вращение лучей, мерцание), секунды.
    anim: f32,
}

impl Sky {
    pub fn new(mode: SkyMode, clock: Clock) -> Self {
        Self { mode, place: SkyPlace::Arc, clock, phase: moon_phase(clock.unix), anim: 0.0 }
    }

    pub fn set_mode(&mut self, mode: SkyMode) {
        self.mode = mode;
    }

    pub fn set_place(&mut self, place: SkyPlace) {
        self.place = place;
    }

    pub fn set_clock(&mut self, clock: Clock) {
        self.clock = clock;
        self.phase = moon_phase(clock.unix);
    }

    pub fn step(&mut self, dt: f32) {
        self.anim = (self.anim + dt) % 3600.0;
    }

    /// Дописывает в `out` фигуры неба, от дальних к ближним.
    pub fn looks(&self, w: f32, h: f32, out: &mut Vec<FlakeLook>) {
        let Some(c) = celestial(self.mode, self.clock) else { return };
        let start = out.len();
        // насколько тело близко к горизонту: от этого оно ниже на дуге и краснее
        let horizon = 1.0 - (PI * c.t).sin();
        let top = if c.moon { TOP_Y } else { SUN_TOP_Y };
        let (cx, cy) = match self.place {
            // дуга: у горизонта ниже, в зените выше; под строкой меню
            SkyPlace::Arc => (w * (0.08 + 0.84 * c.t), top + horizon * (h * 0.16).clamp(80.0, 180.0)),
            SkyPlace::Left => (CORNER_X, top),
            SkyPlace::Right => (w - CORNER_X, top),
        };
        if c.moon {
            self.stars(w, h, c.alpha, out);
            self.moon([cx, cy], c, out);
        } else {
            self.sun([cx, cy], horizon, c.alpha, w, h, out);
        }
        out.truncate(start + SKY_MAX);
    }

    /// Солнце: мягкое свечение, раскалённое ядро и лучи во все стороны — длинные
    /// мягкие клинья света, как лучи сквозь дымку.
    fn sun(&self, c: [f32; 2], horizon: f32, a: f32, w: f32, h: f32, out: &mut Vec<FlakeLook>) {
        // у горизонта свет краснее
        let red = horizon.powf(1.6);
        let col = mix([1.0, 0.86, 0.40], [1.0, 0.45, 0.15], red);
        let ray = mix([1.0, 0.80, 0.40], [1.0, 0.48, 0.18], red);
        let hot = mix([1.0, 0.97, 0.80], [1.0, 0.70, 0.40], red);

        out.push(shape(c, [SUN_R * 5.0; 2], 0.0, FlakeShape::Glow, col, 0.45 * a));
        let diag = (w * w + h * h).sqrt();
        for i in 0..BEAMS {
            let k = 1000 + i as u32 * 8;
            // лучи расходятся по кругу с неровным шагом; весь веер очень медленно
            // поворачивается, а каждый луч ещё качается и мерцает по-своему
            let ang = (i as f32 + hash01(k) * 0.6 - 0.3) * TAU / BEAMS as f32
                + self.anim * 0.02
                + (self.anim * 0.18 + hash01(k + 1) * TAU).sin() * 0.025;
            let pulse = 0.75 + 0.25 * (self.anim * 0.6 + hash01(k + 2) * TAU).sin();
            // каждый третий луч — тонкий, яркий и длиннее, остальные — широкие и мягкие
            let thin = i % 3 == 1;
            let len = diag * if thin { 0.22 + 0.30 * hash01(k + 3) } else { 0.14 + 0.26 * hash01(k + 3) };
            let width = if thin { 3.0 + 4.0 * hash01(k + 4) } else { 20.0 + 45.0 * hash01(k + 4).powi(2) };
            let alpha = (0.22 + 0.25 * hash01(k + 5)) * if thin { 1.1 } else { 1.0 } * pulse * a;
            let (s, co) = ang.sin_cos();
            let mid = SUN_R * 0.5 + len * 0.5;
            out.push(shape([c[0] + co * mid, c[1] + s * mid], [len * 0.5, width], ang, FlakeShape::Beam, ray, alpha));
        }
        out.push(shape(c, [SUN_R * 2.8; 2], 0.0, FlakeShape::Glow, col, 0.85 * a));
        out.push(shape(c, [SUN_R * 1.5; 2], 0.0, FlakeShape::Glow, hot, 0.9 * a));
        out.push(shape(c, [SUN_R * 0.85; 2], 0.0, FlakeShape::Capsule, hot, a));
    }

    fn moon(&self, c: [f32; 2], cel: Celestial, out: &mut Vec<FlakeLook>) {
        const R: f32 = 20.0;
        let a = cel.alpha;
        // освещённая доля: на новолуние свечение почти гаснет
        let lit = (1.0 - (TAU * self.phase).cos()) / 2.0;
        out.push(shape(c, [R * 3.0; 2], 0.0, FlakeShape::Glow, [0.65, 0.75, 1.0], 0.28 * a * (0.3 + 0.7 * lit)));
        // тёмная часть диска едва заметна — «пепельный свет»
        out.push(shape(c, [R, R], 0.0, FlakeShape::Capsule, [0.7, 0.76, 0.92], 0.12 * a));
        // чем ближе к краю экрана, тем сильнее наклон серпа
        let tilt = (cel.t - 0.5) * 0.9;
        out.push(shape(c, [R, R], tilt, FlakeShape::Moon(self.phase), [0.95, 0.96, 1.0], 0.95 * a));
    }

    fn stars(&self, w: f32, h: f32, a: f32, out: &mut Vec<FlakeLook>) {
        let n = ((w / 18.0) as usize).clamp(40, 120);
        for i in 0..n {
            let k = i as u32 * 8;
            let x = hash01(k) * w;
            let y = 24.0 + hash01(k + 1).powf(1.6) * h * 0.45;
            let size = hash01(k + 2);
            let r = 0.5 + size.powi(3) * 1.5;
            let speed = 0.6 + hash01(k + 3) * 1.8;
            let twinkle = 0.6 + 0.4 * (self.anim * speed + hash01(k + 5) * TAU).sin();
            let tint = hash01(k + 6);
            let rgb = if tint < 0.25 {
                [1.0, 0.92, 0.78]
            } else if tint < 0.5 {
                [0.8, 0.88, 1.0]
            } else {
                [1.0, 1.0, 1.0]
            };
            let alpha = (0.35 + 0.6 * size) * twinkle * a;
            if r > 1.4 {
                out.push(shape([x, y], [r * 4.0; 2], 0.0, FlakeShape::Glow, rgb, 0.25 * twinkle * a));
            }
            out.push(shape([x, y], [r, r], 0.0, FlakeShape::Capsule, rgb, alpha));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(hour: f32, season: f32) -> Clock {
        Clock { hour, season, unix: 0.0 }
    }

    #[test]
    fn summer_days_are_longer_than_winter_days() {
        let (sr, ss) = sun_hours(5.5);
        let (wr, ws) = sun_hours(11.5);
        assert!(ss - sr > ws - wr + 6.0, "лето {} ч, зима {} ч", ss - sr, ws - wr);
        assert!(sr < 6.0 && ss > 19.0, "летом солнце встаёт рано и садится поздно");
        assert!(wr > 8.0 && ws < 17.0, "зимой — наоборот");
    }

    #[test]
    fn auto_picks_sun_by_day_and_moon_by_night() {
        let noon = celestial(SkyMode::Auto, at(12.5, 5.5)).unwrap();
        assert!(!noon.moon && noon.alpha > 0.99 && (noon.t - 0.5).abs() < 0.02);
        let night = celestial(SkyMode::Auto, at(1.0, 5.5)).unwrap();
        assert!(night.moon && night.alpha > 0.99);
        // та же ночная пора зимой — ещё ночь, а 8 утра зимой — тоже ночь
        assert!(celestial(SkyMode::Auto, at(8.0, 11.5)).unwrap().moon);
        assert!(!celestial(SkyMode::Auto, at(8.0, 5.5)).unwrap().moon);
    }

    #[test]
    fn body_fades_at_the_horizon_and_crosses_left_to_right() {
        let (rise, set) = sun_hours(5.5);
        let dawn = celestial(SkyMode::Auto, at(rise + 0.05, 5.5)).unwrap();
        assert!(dawn.alpha < 0.1 && dawn.t < 0.02);
        let dusk = celestial(SkyMode::Auto, at(set - 0.05, 5.5)).unwrap();
        assert!(dusk.alpha < 0.1 && dusk.t > 0.98);
        let mut last = -1.0;
        for hour in [8.0, 10.0, 12.0, 14.0, 16.0, 18.0] {
            let t = celestial(SkyMode::Auto, at(hour, 5.5)).unwrap().t;
            assert!(t > last);
            last = t;
        }
        // ночью луна идёт тем же путём, даже через полночь
        let late = celestial(SkyMode::Auto, at(23.0, 5.5)).unwrap().t;
        let early = celestial(SkyMode::Auto, at(2.0, 5.5)).unwrap().t;
        assert!(early > late && (0.0..=1.0).contains(&late) && (0.0..=1.0).contains(&early));
    }

    #[test]
    fn forced_modes_ignore_the_clock() {
        for hour in [0.0, 6.0, 12.5, 23.9] {
            let sun = celestial(SkyMode::Sun, at(hour, 5.5)).unwrap();
            assert!(!sun.moon && sun.alpha == 1.0);
            let moon = celestial(SkyMode::Moon, at(hour, 5.5)).unwrap();
            assert!(moon.moon && moon.alpha == 1.0);
            assert!(celestial(SkyMode::Off, at(hour, 5.5)).is_none());
        }
    }

    #[test]
    fn moon_phase_matches_known_dates() {
        let p = |unix: f64| moon_phase(unix);
        assert!(p(947_182_440.0) < 0.01, "новолуние 6.01.2000");
        assert!((p(947_182_440.0 + 14.4375 * 86_400.0) - 0.5).abs() < 0.03, "полнолуние 21.01.2000");
        assert!((0.0..1.0).contains(&p(1.78e9)));
    }

    #[test]
    fn sky_shapes_fit_the_budget() {
        for (mode, hour) in [(SkyMode::Sun, 12.0), (SkyMode::Moon, 1.0), (SkyMode::Auto, 12.0), (SkyMode::Auto, 1.0)] {
            let mut out = Vec::new();
            Sky::new(mode, at(hour, 5.5)).looks(3840.0, 2160.0, &mut out);
            assert!(!out.is_empty() && out.len() <= SKY_MAX, "{mode:?}: {} фигур", out.len());
            assert!(out.iter().all(|f| f.center.iter().all(|v| v.is_finite()) && f.color[3] <= 1.0));
        }
        let mut none = Vec::new();
        Sky::new(SkyMode::Off, at(12.0, 5.5)).looks(1000.0, 600.0, &mut none);
        assert!(none.is_empty());
    }

    #[test]
    fn day_has_rays_and_night_has_moon_and_stars() {
        let mut day = Vec::new();
        Sky::new(SkyMode::Sun, at(12.5, 5.5)).looks(1440.0, 900.0, &mut day);
        let beams = day.iter().filter(|f| f.shape == FlakeShape::Beam).count();
        assert_eq!(beams, BEAMS);
        assert!(day.iter().any(|f| f.shape == FlakeShape::Glow));
        let mut night = Vec::new();
        Sky::new(SkyMode::Moon, at(1.0, 5.5)).looks(1440.0, 900.0, &mut night);
        assert!(night.iter().any(|f| matches!(f.shape, FlakeShape::Moon(_))));
        assert!(night.iter().filter(|f| f.shape == FlakeShape::Capsule).count() >= 40);
    }

    #[test]
    fn corner_places_pin_the_body_regardless_of_time() {
        let center = |place: SkyPlace, mode: SkyMode, hour: f32| {
            let mut sky = Sky::new(mode, at(hour, 5.5));
            sky.set_place(place);
            let mut out = Vec::new();
            sky.looks(1440.0, 900.0, &mut out);
            // тело — самая крупная заливка из последних фигур: берём диск-ядро
            let body = out.iter().rev().find(|f| matches!(f.shape, FlakeShape::Moon(_)) || (f.shape == FlakeShape::Capsule && f.half == [SUN_R * 0.85, SUN_R * 0.85]));
            body.map(|f| f.center).unwrap()
        };
        for hour in [9.0, 12.5, 17.0] {
            assert_eq!(center(SkyPlace::Right, SkyMode::Sun, hour), [1440.0 - CORNER_X, SUN_TOP_Y]);
            assert_eq!(center(SkyPlace::Left, SkyMode::Sun, hour), [CORNER_X, SUN_TOP_Y]);
        }
        for hour in [22.0, 1.0, 4.0] {
            assert_eq!(center(SkyPlace::Right, SkyMode::Moon, hour), [1440.0 - CORNER_X, TOP_Y]);
        }
        // по дуге положение зависит от времени
        assert_ne!(center(SkyPlace::Arc, SkyMode::Sun, 9.0), center(SkyPlace::Arc, SkyMode::Sun, 12.5));
    }

    #[test]
    fn sun_beams_radiate_in_all_directions_from_the_sun() {
        let mut sky = Sky::new(SkyMode::Sun, at(12.5, 5.5));
        sky.set_place(SkyPlace::Right);
        let mut out = Vec::new();
        sky.looks(1440.0, 900.0, &mut out);
        let sun = [1440.0 - CORNER_X, SUN_TOP_Y];
        let beams: Vec<_> = out.iter().filter(|f| f.shape == FlakeShape::Beam).collect();
        assert_eq!(beams.len(), BEAMS);
        let mut octants = [0; 8];
        for b in &beams {
            // каждый луч выходит из солнца
            let (s, c) = b.rotation.sin_cos();
            let start = [b.center[0] - c * b.half[0], b.center[1] - s * b.half[0]];
            assert!((start[0] - sun[0]).abs() < 15.0 && (start[1] - sun[1]).abs() < 15.0);
            let octant = ((b.rotation.rem_euclid(TAU) / (TAU / 8.0)) as usize).min(7);
            octants[octant] += 1;
        }
        // и лучи есть в каждой из восьми сторон света, а не в узком секторе
        assert!(octants.iter().all(|n| *n >= 1), "лучи только в части сторон: {octants:?}");
        // лучи разной ширины: и тонкие, и широкие
        let widths: Vec<f32> = beams.iter().map(|b| b.half[1]).collect();
        assert!(widths.iter().any(|w| *w < 8.0) && widths.iter().any(|w| *w > 25.0));
    }

    #[test]
    fn beam_coverage_fades_along_and_across() {
        let c = |x: f32, y: f32| crate::look::coverage(FlakeShape::Beam, x, y, 300.0, 40.0);
        // вдоль оси ярче у источника, чем вдали; за концами пусто
        assert!(c(-250.0, 0.0) > c(100.0, 0.0) && c(100.0, 0.0) > c(280.0, 0.0));
        assert_eq!(c(-310.0, 0.0), 0.0);
        assert!(c(299.0, 0.0) < 0.02);
        // поперёк — мягкий край, за краем клина пусто
        assert!(c(0.0, 0.0) > c(0.0, 10.0) && c(0.0, 10.0) > c(0.0, 18.0));
        assert_eq!(c(0.0, 30.0), 0.0);
        // клин расширяется: на одном и том же смещении вдали ещё светло, а у источника уже пусто
        assert!(c(250.0, 8.0) > 0.0 && c(-250.0, 8.0) == 0.0);
    }

    #[test]
    fn moon_terminator_follows_the_phase() {
        let cover = |phase: f32, x: f32| crate::look::coverage(FlakeShape::Moon(phase), x, 0.0, 20.0, 20.0);
        // растущая — освещена справа; убывающая — слева
        assert!(cover(0.25, 12.0) > 0.99 && cover(0.25, -12.0) < 0.01);
        assert!(cover(0.75, -12.0) > 0.99 && cover(0.75, 12.0) < 0.01);
        // полнолуние — весь диск, новолуние — ничего
        assert!(cover(0.5, -12.0) > 0.99 && cover(0.5, 12.0) > 0.99);
        assert!(cover(0.0, 0.0) < 0.01 && cover(0.0, 12.0) < 0.01);
        // вне диска пусто всегда
        assert_eq!(cover(0.5, 30.0), 0.0);
    }
}
