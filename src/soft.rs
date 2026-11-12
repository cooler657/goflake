//! Программная отрисовка: тот же кадр, что рисует видеокарта, но на процессоре.
//!
//! Результат — растр с цветом, умноженным на альфу. Как показать его на экране,
//! решает платформа. Используется там, где у системы нет программного
//! графического адаптера (macOS).

// Вне macOS программный режим обеспечивает системный адаптер, и этот модуль
// нужен только тестам.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use crate::look::{self, FEATHER_PX, FlakeShape, SHADOW_PX, SHADOW_RGB};
use crate::sim::Sim;
use crate::sky::Sky;

pub struct Canvas {
    pub width: usize,
    pub height: usize,
    /// Пиксели `0xAARRGGBB`, цвет умножен на альфу. В памяти это байты B, G, R, A.
    pub pixels: Vec<u32>,
}

impl Canvas {
    pub fn new() -> Self {
        Self { width: 0, height: 0, pixels: Vec::new() }
    }

    /// Очищает растр до прозрачного, при необходимости меняя размер.
    fn reset(&mut self, width: usize, height: usize) {
        self.width = width;
        self.height = height;
        self.pixels.clear();
        self.pixels.resize(width * height, 0);
    }

    /// Накладывает цвет `rgb` с прозрачностью `alpha` (0–1) на пиксель.
    #[inline]
    fn blend(&mut self, x: usize, y: usize, rgb: [f32; 3], alpha: f32) {
        if alpha <= 0.003 {
            return;
        }
        let a = alpha.min(1.0);
        let dst = &mut self.pixels[y * self.width + x];
        let keep = 1.0 - a;
        let ch = |shift: u32, src: f32| -> u32 {
            let d = ((*dst >> shift) & 0xFF) as f32;
            ((src * a * 255.0 + d * keep + 0.5) as u32).min(255)
        };
        let out_a = (((a * 255.0) + ((*dst >> 24) as f32) * keep + 0.5) as u32).min(255);
        *dst = (out_a << 24) | (ch(16, rgb[0]) << 16) | (ch(8, rgb[1]) << 8) | ch(0, rgb[2]);
    }

    /// Рисует кадр. `scale` — пикселей растра на логическую точку.
    pub fn draw(&mut self, sim: &Sim, sky: &Sky, scale: f32) {
        let width = (sim.w * scale).ceil().max(1.0) as usize;
        let height = (sim.h * scale).ceil().max(1.0) as usize;
        self.reset(width, height);
        // небо рисуется под частицами
        let mut looks = Vec::new();
        sky.looks(sim.w, sim.h, &mut looks);
        for f in &looks {
            self.flake(f, scale);
        }
        // небо — за окнами: по их рамкам картинка вырезается
        if !looks.is_empty() {
            for r in sim.window_rects() {
                self.clear_rect(r.min_x() * scale, r.min_y() * scale, r.max_x() * scale, r.max_y() * scale);
            }
        }
        for p in &sim.particles {
            self.flake(&look::flake(p), scale);
        }
        // кучи поверх частиц — как и на видеокарте
        look::pile_runs(sim, |run, base, shadow| self.pile(run, base, shadow, scale));
    }

    /// Делает прямоугольник (в пикселях) полностью прозрачным.
    fn clear_rect(&mut self, x0: f32, y0: f32, x1: f32, y1: f32) {
        let (xa, xb) = (x0.round().max(0.0) as usize, (x1.round().max(0.0) as usize).min(self.width));
        let (ya, yb) = (y0.round().max(0.0) as usize, (y1.round().max(0.0) as usize).min(self.height));
        for y in ya..yb {
            if xa < xb {
                self.pixels[y * self.width + xa..y * self.width + xb].fill(0);
            }
        }
    }

    fn flake(&mut self, f: &look::FlakeLook, scale: f32) {
        let (cx, cy) = (f.center[0] * scale, f.center[1] * scale);
        let (a, b) = (f.half[0] * scale, f.half[1] * scale);
        let reach = a.max(b) + 1.0;
        let x0 = (cx - reach).floor().max(0.0) as usize;
        let y0 = (cy - reach).floor().max(0.0) as usize;
        let x1 = ((cx + reach).ceil() as isize).min(self.width as isize);
        let y1 = ((cy + reach).ceil() as isize).min(self.height as isize);
        if x1 <= 0 || y1 <= 0 {
            return;
        }
        let (sin, cos) = f.rotation.sin_cos();
        let rgb = [f.color[0], f.color[1], f.color[2]];
        for y in y0..y1 as usize {
            for x in x0..x1 as usize {
                // центр пикселя в осях частицы
                let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
                let (lx, ly) = (dx * cos + dy * sin, -dx * sin + dy * cos);
                let cover = look::coverage(f.shape, lx, ly, a, b);
                if cover <= 0.0 {
                    continue;
                }
                let shade = if f.shape == FlakeShape::Leaf { look::leaf_rib(ly) } else { 1.0 };
                self.blend(x, y, [rgb[0] * shade, rgb[1] * shade, rgb[2] * shade], f.color[3] * cover);
            }
        }
    }

    fn pile(&mut self, run: &[look::ContourPoint], base: f32, shadow: f32, scale: f32) {
        let base = base * scale;
        for pair in run.windows(2) {
            let (p, q) = (pair[0], pair[1]);
            let (xa, xb) = (p.x * scale, q.x * scale);
            let first = (xa - 0.5).ceil().max(0.0) as usize;
            let last = ((xb - 0.5).ceil() as isize).min(self.width as isize);
            for x in first..last.max(0) as usize {
                let t = ((x as f32 + 0.5 - xa) / (xb - xa).max(1e-3)).clamp(0.0, 1.0);
                let top = (p.top + (q.top - p.top) * t) * scale;
                let mix = |a: [f32; 4], b: [f32; 4]| [0, 1, 2, 3].map(|i| a[i] + (b[i] - a[i]) * t);
                let (top_c, base_c) = (mix(p.top_color, q.top_color), mix(p.base_color, q.base_color));
                let depth = (base - top).max(1e-3);
                let y_from = (top - SHADOW_PX).floor().max(0.0) as usize;
                let y_to = (base.ceil() as isize).min(self.height as isize).max(0) as usize;
                for y in y_from..y_to {
                    let c = y as f32 + 0.5;
                    if c < top {
                        let above = top - c;
                        // тень: гуще у кромки, сходит на нет выше
                        if shadow > 0.0 {
                            self.blend(x, y, SHADOW_RGB, shadow * (1.0 - above / SHADOW_PX).max(0.0));
                        }
                        // сглаживание кромки
                        let feather = (1.0 - above / FEATHER_PX).max(0.0);
                        self.blend(x, y, [top_c[0], top_c[1], top_c[2]], top_c[3] * feather);
                    } else {
                        // тело кучи: цвет плавно меняется от верха к низу
                        let v = ((c - top) / depth).clamp(0.0, 1.0);
                        let rgb = [0, 1, 2].map(|i| top_c[i] + (base_c[i] - top_c[i]) * v);
                        let alpha = top_c[3] + (base_c[3] - top_c[3]) * v;
                        self.blend(x, y, rgb, alpha * (base - c + 0.5).clamp(0.0, 1.0));
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::materials::KINDS;
    use crate::clock::Clock;
    use crate::sim::{Rect, WindowInfo};
    use crate::sky::{SkyMode, SkyPlace};

    fn sky(mode: SkyMode) -> Sky {
        Sky::new(mode, Clock { hour: 12.5, season: 5.5, unix: 0.0 })
    }

    fn alpha_sum(c: &Canvas) -> u64 {
        c.pixels.iter().map(|p| (p >> 24) as u64).sum()
    }

    #[test]
    fn empty_scene_is_fully_transparent() {
        let mut sim = Sim::new(1);
        sim.w = 320.0;
        sim.h = 200.0;
        let mut canvas = Canvas::new();
        canvas.draw(&sim, &sky(SkyMode::Off), 1.0);
        assert_eq!((canvas.width, canvas.height), (320, 200));
        assert_eq!(alpha_sum(&canvas), 0);
    }

    #[test]
    fn every_material_draws_particles_and_piles() {
        for kind in KINDS {
            let mut sim = Sim::new(3);
            sim.w = 640.0;
            sim.h = 400.0;
            sim.rate = 110.0;
            sim.set_kind(kind);
            let win = [WindowInfo { id: 5, rect: Rect::new(100.0, 200.0, 400.0, 150.0) }];
            for _ in 0..(60 * 12) {
                sim.refresh(&win, None, 0.05);
                sim.step(1.0 / 60.0, (-500.0, -500.0), (0.0, 0.0));
            }
            let mut canvas = Canvas::new();
            canvas.draw(&sim, &sky(SkyMode::Off), 1.0);
            assert!(alpha_sum(&canvas) > 1000, "{kind:?}: кадр пуст");
            // цвет умножен на альфу: ни один канал не превышает альфу
            for p in &canvas.pixels {
                let a = p >> 24;
                assert!((p >> 16) & 0xFF <= a + 1 && (p >> 8) & 0xFF <= a + 1 && p & 0xFF <= a + 1, "{kind:?}");
            }
            // прямо над кромкой окна есть куча
            let row = 199 * canvas.width;
            assert!(canvas.pixels[row + 110..row + 490].iter().any(|p| p >> 24 > 60), "{kind:?}: нет кучи");
        }
    }

    #[test]
    fn sky_is_hidden_behind_windows() {
        let mut sim = Sim::new(1);
        sim.w = 960.0;
        sim.h = 540.0;
        sim.rate = 0.0;
        let mut sun = sky(SkyMode::Sun);
        sun.set_place(SkyPlace::Right); // солнце в (870, 96)
        let alpha = |canvas: &Canvas, x: usize, y: usize| canvas.pixels[y * canvas.width + x] >> 24;

        let mut open = Canvas::new();
        open.draw(&sim, &sun, 1.0);
        assert!(alpha(&open, 870, 96) > 200 && alpha(&open, 830, 96) > 200, "солнце не нарисовано");

        // окно закрывает правую половину солнца и больше ничего
        let win = [WindowInfo { id: 5, rect: Rect::new(870.0, 50.0, 400.0, 300.0) }];
        sim.refresh(&win, None, 0.05);
        let mut covered = Canvas::new();
        covered.draw(&sim, &sun, 1.0);
        assert_eq!(alpha(&covered, 880, 96), 0, "за окном небо должно пропасть");
        assert_eq!(alpha(&covered, 940, 300), 0);
        assert!(alpha(&covered, 830, 96) > 200, "вне окна небо остаётся");
        assert_eq!(covered.pixels[96 * covered.width + 830], open.pixels[96 * open.width + 830]);
    }

    #[test]
    fn scale_changes_resolution() {
        let mut sim = Sim::new(1);
        sim.w = 100.0;
        sim.h = 50.0;
        let mut canvas = Canvas::new();
        canvas.draw(&sim, &sky(SkyMode::Off), 2.0);
        assert_eq!((canvas.width, canvas.height), (200, 100));
    }

    /// Сохраняет кадры, чтобы посмотреть на них глазами. Запуск:
    /// `SNOWDRIFT_DUMP=/папка cargo test dump_preview -- --ignored`.
    /// Каждый кадр — файл `имя.raw` (байты B, G, R, A, цвет умножен на альфу)
    /// и `имя.txt` с шириной и высотой.
    #[test]
    #[ignore]
    fn dump_preview() {
        let Some(dir) = std::env::var_os("SNOWDRIFT_DUMP") else { return };
        let dir = std::path::PathBuf::from(dir);
        let save = |name: &str, canvas: &Canvas| {
            let bytes: Vec<u8> = canvas.pixels.iter().flat_map(|p| p.to_le_bytes()).collect();
            std::fs::write(dir.join(format!("{name}.raw")), bytes).unwrap();
            std::fs::write(dir.join(format!("{name}.txt")), format!("{} {}", canvas.width, canvas.height)).unwrap();
        };
        let at = |hour: f32, season: f32, phase: f32| Clock {
            hour,
            season,
            unix: 947_182_440.0 + phase as f64 * 29.530_588_853 * 86_400.0,
        };
        // небо: полдень лета, закат лета, полная луна, растущий серп, убывающая луна
        let scenes = [
            ("sky_noon", SkyMode::Auto, at(12.5, 5.5, 0.5)),
            ("sky_sunset", SkyMode::Auto, at(20.6, 5.5, 0.5)),
            ("sky_full", SkyMode::Auto, at(23.0, 5.5, 0.5)),
            ("sky_crescent", SkyMode::Auto, at(21.5, 5.5, 0.12)),
            ("sky_waning", SkyMode::Auto, at(3.0, 5.5, 0.85)),
            ("sky_newmoon", SkyMode::Moon, at(1.0, 5.5, 0.0)),
            ("sky_corner_sun", SkyMode::Sun, at(13.0, 5.5, 0.5)),
            ("sky_corner_moon", SkyMode::Moon, at(22.5, 5.5, 0.35)),
        ];
        for (name, mode, clock) in scenes {
            let mut sim = Sim::new(1);
            sim.w = 960.0;
            sim.h = 540.0;
            let mut sky = Sky::new(mode, clock);
            sky.set_place(if name.contains("corner") { SkyPlace::Right } else { SkyPlace::Arc });
            sky.step(7.0);
            let mut canvas = Canvas::new();
            canvas.draw(&sim, &sky, 1.0);
            save(name, &canvas);
        }
        // осадки: четыре вида на окне
        for kind in KINDS {
            let mut sim = Sim::new(9);
            sim.w = 960.0;
            sim.h = 540.0;
            sim.rate = 110.0;
            sim.set_kind(kind);
            let win = [WindowInfo { id: 5, rect: Rect::new(160.0, 260.0, 640.0, 240.0) }];
            for _ in 0..(60 * 30) {
                sim.refresh(&win, None, 0.05);
                sim.step(1.0 / 60.0, (-500.0, -500.0), (0.0, 0.0));
            }
            let mut canvas = Canvas::new();
            canvas.draw(&sim, &sky(SkyMode::Off), 1.0);
            save(&format!("kind_{kind:?}").to_lowercase(), &canvas);
        }
    }
}
