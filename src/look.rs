//! Внешний вид: как частица и куча превращаются в фигуры и цвета.
//!
//! Общий слой для обеих отрисовок — на видеокарте (`render.rs`) и на
//! процессоре (`soft.rs`), чтобы они рисовали одно и то же. Формулы покрытия
//! в `coverage` и в `shader.wgsl` — одни и те же.

use crate::materials::Shape;
use crate::sim::{COL_W, Particle, Sim};

/// Прозрачность слоёв частиц: дальний, средний, ближний.
const LAYER_ALPHA: [f32; 3] = [0.45, 0.7, 0.95];
/// Цвет тени над кромкой кучи.
pub const SHADOW_RGB: [f32; 3] = [0.1, 0.2, 0.4];
/// Высота тени и сглаживающей полоски над кромкой, физические пиксели.
pub const SHADOW_PX: f32 = 3.5;
pub const FEATHER_PX: f32 = 1.5;

/// Форма фигуры в её собственных осях (x вдоль, y поперёк).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FlakeShape {
    /// Круг или, если вытянут, штрих со скруглёнными концами.
    Capsule,
    /// Лист: заострённый овал с прожилкой.
    Leaf,
    /// Мягкое свечение: яркость падает от центра к краю по квадрату.
    Glow,
    /// Луна с фазой от 0 до 1: 0 — новолуние, 0.5 — полнолуние. Растущая
    /// освещена справа, убывающая — слева.
    Moon(f32),
    /// Луч света: клин, который расходится от левого конца (источника) к
    /// правому, мягкий по краям и затухающий вдаль. Полудлина — `half[0]`,
    /// полуширина на дальнем конце — `half[1]`.
    Beam,
}

impl FlakeShape {
    /// Числовой код для шейдера: (вид, параметр).
    pub fn code(self) -> (f32, f32) {
        match self {
            FlakeShape::Capsule => (0.0, 0.0),
            FlakeShape::Leaf => (1.0, 0.0),
            FlakeShape::Glow => (2.0, 0.0),
            FlakeShape::Moon(phase) => (3.0, phase),
            FlakeShape::Beam => (4.0, 0.0),
        }
    }
}

/// Фигура, готовая к отрисовке. Размеры — в логических точках.
pub struct FlakeLook {
    pub center: [f32; 2],
    /// Полуразмеры вдоль и поперёк собственной оси. У круга, свечения и луны —
    /// радиус в обоих.
    pub half: [f32; 2],
    pub rotation: f32,
    pub shape: FlakeShape,
    /// Цвет без умножения на альфу.
    pub color: [f32; 4],
}

pub fn flake(p: &Particle) -> FlakeLook {
    let m = p.kind.material();
    let c = m.colors[p.tint as usize % m.colors.len()];
    let color = [c[0], c[1], c[2], LAYER_ALPHA[p.layer.min(2) as usize] * m.alpha];
    let (half, rotation, shape) = match m.shape {
        Shape::Dot => ([p.r, p.r], 0.0, FlakeShape::Capsule),
        Shape::Streak => {
            // штрих вытянут вдоль скорости: чем быстрее капля, тем он длиннее
            let speed = (p.vx * p.vx + p.vy * p.vy).sqrt();
            let radius = (p.r * 0.4).max(0.4);
            ([radius + (speed * m.streak).clamp(1.0, 14.0), radius], p.vy.atan2(p.vx), FlakeShape::Capsule)
        }
        Shape::Leaf => ([p.r, p.r * 0.5], p.angle, FlakeShape::Leaf),
    };
    FlakeLook { center: [p.x, p.y], half, rotation, shape, color }
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Доля пикселя, покрытая фигурой. `x`, `y` — точка в осях фигуры, `a`, `b` —
/// полуразмеры; всё в пикселях. Та же формула, что в шейдере.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn coverage(shape: FlakeShape, x: f32, y: f32, a: f32, b: f32) -> f32 {
    match shape {
        FlakeShape::Capsule => {
            // при a == b это круг, при a > b — штрих со скруглёнными концами
            let qx = (x.abs() - (a - b)).max(0.0);
            (b - (qx * qx + y * y).sqrt() + 0.5).clamp(0.0, 1.0)
        }
        FlakeShape::Leaf => {
            let t = (x / a).clamp(-1.0, 1.0);
            let f = b * (1.0 - t * t) - y.abs();
            let k = 2.0 * b * t / a;
            (f / (1.0 + k * k).sqrt() + 0.5).clamp(0.0, 1.0)
        }
        FlakeShape::Glow => {
            let g = (1.0 - (x * x + y * y).sqrt() / a).clamp(0.0, 1.0);
            g * g
        }
        FlakeShape::Moon(phase) => {
            let disk = (a - (x * x + y * y).sqrt() + 0.5).clamp(0.0, 1.0);
            // терминатор — половина эллипса: при полуночи на краю диска он
            // прямой, при четверти проходит по середине, при полнолунии — по
            // противоположному краю
            let v = (y / a).clamp(-1.0, 1.0);
            let w = (1.0 - v * v).max(0.0).sqrt();
            let c = (std::f32::consts::TAU * phase).cos();
            let edge = if phase < 0.5 { x - c * w * a } else { -c * w * a - x };
            disk * (edge + 0.5).clamp(0.0, 1.0)
        }
        FlakeShape::Beam => {
            // u: 0 у источника, 1 на дальнем конце; ширина растёт вместе с u
            let u = (x + a) / (2.0 * a);
            let width = (b * u).max(0.75);
            let across = smoothstep(0.0, 1.0, 1.0 - y.abs() / width);
            let along = (1.0 - u).max(0.0) * smoothstep(0.0, 0.03, u);
            across * along
        }
    }
}

/// Цвет с прозрачностью, упакованный как пиксель растра `0xAARRGGBB` с цветом,
/// умноженным на альфу.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn pack_premul(c: [f32; 4]) -> u32 {
    let a = c[3].clamp(0.0, 1.0);
    let ch = |v: f32| ((v.clamp(0.0, 1.0) * a * 255.0 + 0.5) as u32).min(255);
    (((a * 255.0 + 0.5) as u32).min(255) << 24) | (ch(c[0]) << 16) | (ch(c[1]) << 8) | ch(c[2])
}

/// Затемнение прожилки листа: множитель яркости по расстоянию от оси, пиксели.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn leaf_rib(y: f32) -> f32 {
    let t = ((y.abs() - 0.3) / 1.0).clamp(0.0, 1.0);
    0.72 + 0.28 * (t * t * (3.0 - 2.0 * t))
}

/// Точка контура кучи.
#[derive(Clone, Copy)]
pub struct ContourPoint {
    pub x: f32,
    /// Верх кучи (y вниз).
    pub top: f32,
    pub top_color: [f32; 4],
    pub base_color: [f32; 4],
}

/// Псевдослучайное число от 0 до 1 по целому — для пёстрых куч и звёзд.
pub fn hash01(n: u32) -> f32 {
    let mut x = n.wrapping_mul(0x9E37_79B1);
    x ^= x >> 15;
    x = x.wrapping_mul(0x85EB_CA6B);
    x ^= x >> 13;
    (x & 0xFFFF) as f32 / 65_535.0
}

/// Обходит все видимые участки куч. Для каждого вызывает `f(контур, низ, тень)`:
/// контур идёт слева направо, на концах высота сходит к нулю.
pub fn pile_runs(sim: &Sim, mut f: impl FnMut(&[ContourPoint], f32, f32)) {
    let mat = sim.kind().material();
    let mut run: Vec<ContourPoint> = Vec::new();
    for (id, s) in sim.surfaces.iter() {
        let n = s.h.len();
        let seed = (*id as u32).wrapping_mul(7919);
        // цвета верха и низа кучи в точке контура; у листьев — пятнами по палитре
        let colors = |k: usize| -> ([f32; 4], [f32; 4]) {
            if !mat.pile_mottled {
                return (mat.pile_top, mat.pile_base);
            }
            let pick = (hash01(seed ^ (k as u32 / 3)) * mat.colors.len() as f32) as usize % mat.colors.len();
            let c = mat.colors[pick];
            ([c[0], c[1], c[2], 1.0], [c[0] * 0.7, c[1] * 0.7, c[2] * 0.7, 1.0])
        };
        let point = |x: f32, top: f32, k: usize| {
            let (top_color, base_color) = colors(k);
            ContourPoint { x, top, top_color, base_color }
        };
        let mut i = 0;
        while i < n {
            if !(s.vis[i] && s.h[i] > 0.15) {
                i += 1;
                continue;
            }
            let mut j = i;
            while j < n && s.vis[j] && s.h[j] > 0.15 {
                j += 1;
            }
            run.clear();
            run.push(point(s.x0 + i as f32 * COL_W, s.y, i));
            for k in i..j {
                let l = if k > i { s.h[k - 1] } else { 0.0 };
                let r = if k + 1 < j { s.h[k + 1] } else { 0.0 };
                let mut h = (l + 2.0 * s.h[k] + r) / 4.0;
                if mat.pile_mottled {
                    // торчащие листья: неровная кромка
                    h += (hash01(seed ^ (k as u32).wrapping_mul(31)) - 0.3) * h.min(3.0);
                }
                run.push(point(s.x0 + (k as f32 + 0.5) * COL_W, s.y - h.max(0.0), k));
            }
            run.push(point(s.x0 + j as f32 * COL_W, s.y, j));
            f(&run, s.y + 0.5, mat.pile_shadow);
            i = j;
        }
    }
}
