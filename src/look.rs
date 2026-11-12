//! Внешний вид: как частица и куча превращаются в фигуры и цвета.
//!
//! Общий слой для обеих отрисовок — на видеокарте (`render.rs`) и на
//! процессоре (`soft.rs`), чтобы они рисовали одно и то же.

use crate::materials::Shape;
use crate::sim::{COL_W, Particle, Sim};

/// Прозрачность слоёв частиц: дальний, средний, ближний.
const LAYER_ALPHA: [f32; 3] = [0.45, 0.7, 0.95];
/// Цвет тени над кромкой кучи.
pub const SHADOW_RGB: [f32; 3] = [0.1, 0.2, 0.4];
/// Высота тени и сглаживающей полоски над кромкой, физические пиксели.
pub const SHADOW_PX: f32 = 3.5;
pub const FEATHER_PX: f32 = 1.5;

/// Частица, готовая к отрисовке. Размеры — в логических точках.
pub struct FlakeLook {
    pub center: [f32; 2],
    /// Полуразмеры вдоль и поперёк собственной оси.
    pub half: [f32; 2],
    pub rotation: f32,
    /// Лист (заострённый овал с прожилкой) вместо крупинки или штриха.
    pub leaf: bool,
    /// Цвет без умножения на альфу.
    pub color: [f32; 4],
}

pub fn flake(p: &Particle) -> FlakeLook {
    let m = p.kind.material();
    let c = m.colors[p.tint as usize % m.colors.len()];
    let color = [c[0], c[1], c[2], LAYER_ALPHA[p.layer.min(2) as usize] * m.alpha];
    let (half, rotation, leaf) = match m.shape {
        Shape::Dot => ([p.r, p.r], 0.0, false),
        Shape::Streak => {
            // штрих вытянут вдоль скорости: чем быстрее капля, тем он длиннее
            let speed = (p.vx * p.vx + p.vy * p.vy).sqrt();
            let radius = (p.r * 0.4).max(0.4);
            ([radius + (speed * 0.007).clamp(1.0, 7.0), radius], p.vy.atan2(p.vx), false)
        }
        Shape::Leaf => ([p.r, p.r * 0.5], p.angle, true),
    };
    FlakeLook { center: [p.x, p.y], half, rotation, leaf, color }
}

/// Доля пикселя, покрытая частицей. `x`, `y` — точка в осях частицы,
/// `a`, `b` — полуразмеры; всё в пикселях. Та же формула, что в шейдере.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn coverage(leaf: bool, x: f32, y: f32, a: f32, b: f32) -> f32 {
    if leaf {
        let t = (x / a).clamp(-1.0, 1.0);
        let f = b * (1.0 - t * t) - y.abs();
        let k = 2.0 * b * t / a;
        (f / (1.0 + k * k).sqrt() + 0.5).clamp(0.0, 1.0)
    } else {
        // капсула: при a == b это круг, при a > b — штрих со скруглёнными концами
        let qx = (x.abs() - (a - b)).max(0.0);
        (b - (qx * qx + y * y).sqrt() + 0.5).clamp(0.0, 1.0)
    }
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

/// Псевдослучайное число от 0 до 1 по целому — для пёстрых куч.
fn hash01(n: u32) -> f32 {
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
