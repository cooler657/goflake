//! Украшения на верхней кромке окон: ели, деревья, кактусы, камни — по виду осадков.
//!
//! Строятся из тех же фигур, что частицы и небо (круги и штрихи `Capsule`),
//! поэтому обе отрисовки рисуют их одинаково. Привязаны к окну: двигаются
//! вместе с ним и скрываются, если кромку закрыло другое окно.

use std::f32::consts::{FRAC_PI_2, PI};

use crate::look::{FlakeLook, FlakeShape, hash01};
use crate::materials::Decor;
use crate::sim::{COL_W, Sim};

/// Сколько фигур украшения могут отдать за кадр (под них запасён буфер на GPU).
pub const DECOR_MAX: usize = 2048;

/// Одно украшение: где стоит и какое.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Item {
    pub x: f32,
    /// Y основания (верх окна).
    pub base: f32,
    pub height: f32,
    pub seed: u32,
}

fn shape(center: [f32; 2], half: [f32; 2], rotation: f32, rgb: [f32; 3]) -> FlakeLook {
    FlakeLook { center, half, rotation, shape: FlakeShape::Capsule, color: [rgb[0], rgb[1], rgb[2], 1.0] }
}

fn circle(c: [f32; 2], r: f32, rgb: [f32; 3]) -> FlakeLook {
    shape(c, [r, r], 0.0, rgb)
}

/// Отрезок толщиной `2r` со скруглёнными концами от `p0` до `p1`.
fn bar(p0: [f32; 2], p1: [f32; 2], r: f32, rgb: [f32; 3]) -> FlakeLook {
    let (dx, dy) = (p1[0] - p0[0], p1[1] - p0[1]);
    let len = (dx * dx + dy * dy).sqrt();
    shape([(p0[0] + p1[0]) / 2.0, (p0[1] + p1[1]) / 2.0], [len / 2.0 + r, r], dy.atan2(dx), rgb)
}

/// Где стоят украшения: несколько штук вдоль верха каждого окна, не закрытые
/// другими окнами. Положение определяется номером окна, поэтому не мигает.
pub fn plan(sim: &Sim) -> Vec<Item> {
    let mut items = Vec::new();
    let mut ids: Vec<i64> = sim.surfaces.keys().copied().filter(|id| *id > 0).collect();
    ids.sort_unstable();
    for id in ids {
        let s = &sim.surfaces[&id];
        let width = s.h.len() as f32 * COL_W;
        if width < 200.0 {
            continue;
        }
        let n = ((width / 260.0) as usize).clamp(1, 4);
        for k in 0..n {
            let seed = (id as u32).wrapping_mul(2_654_435_761).wrapping_add(k as u32 * 977);
            // положение: окно делится на n равных долей, в каждой дерево стоит в своём месте
            let x = s.x0 + (k as f32 + 0.2 + 0.6 * hash01(seed)) / n as f32 * width;
            let col = ((x - s.x0) / COL_W) as isize;
            // основание должно быть видно целиком, не под другим окном
            let seen = (col - 6..=col + 6).all(|c| c >= 0 && (c as usize) < s.vis.len() && s.vis[c as usize]);
            if !seen || x < 0.0 || x > sim.w {
                continue;
            }
            items.push(Item { x, base: s.y, height: 48.0 + 50.0 * hash01(seed ^ 0x5bd1), seed });
        }
    }
    items
}

/// Дописывает в `out` фигуры украшений, от задних к передним.
pub fn looks(sim: &Sim, out: &mut Vec<FlakeLook>) {
    let kind = sim.kind().material().decor;
    if !sim.decor || kind == Decor::None {
        return;
    }
    let start = out.len();
    for it in plan(sim) {
        match kind {
            Decor::None => {}
            Decor::Fir => fir(&it, out),
            Decor::Leafy => leafy(&it, out),
            Decor::Cactus => cactus(&it, out),
            Decor::Bare => bare(&it, out),
            Decor::Rocks => rocks(&it, out),
        }
        if out.len() - start >= DECOR_MAX {
            break;
        }
    }
    out.truncate(start + DECOR_MAX);
}

/// Ель: ступенчатый треугольник из горизонтальных брусков, на уступах — снег.
fn fir(it: &Item, out: &mut Vec<FlakeLook>) {
    let (x, b, h) = (it.x, it.base, it.height);
    let trunk = h * 0.14;
    let rt = (h * 0.04).max(2.0);
    out.push(bar([x, b - rt], [x, b - trunk], rt, [0.30, 0.20, 0.12]));
    const BARS: usize = 16;
    const TIERS: f32 = 5.0;
    let wmax = h * 0.6;
    let thick = (h - trunk) / BARS as f32;
    for j in 0..BARS {
        let t = (j as f32 + 0.5) / BARS as f32;
        let y = b - trunk - t * (h - trunk);
        // в пределах яруса ширина убывает вверх, на следующем ярусе — снова шире
        let f = (t * TIERS).fract();
        let w = wmax * (1.0 - t * 0.85) * (0.55 + 0.45 * (1.0 - f));
        let green = if j % 2 == 0 { [0.11, 0.33, 0.20] } else { [0.15, 0.40, 0.24] };
        let r = thick * 0.62;
        out.push(bar([x - w / 2.0 + r, y], [x + w / 2.0 - r, y], r, green));
        // снег лежит на самых широких местах ярусов
        if f < 1.0 / TIERS * 1.6 && w > 8.0 {
            out.push(bar([x - w * 0.38, y - r * 0.5], [x + w * 0.38, y - r * 0.5], r * 0.4, [0.97, 0.98, 1.0]));
        }
    }
    // снег на макушке
    out.push(circle([x, b - h + thick * 0.3], thick * 0.7, [0.97, 0.98, 1.0]));
}

/// Дерево весной: тёмный ствол, пышная зелёная крона и розовые цветы.
fn leafy(it: &Item, out: &mut Vec<FlakeLook>) {
    let (x, b, h) = (it.x, it.base, it.height);
    let rt = (h * 0.045).max(2.0);
    out.push(bar([x, b - rt], [x, b - h * 0.5], rt, [0.42, 0.28, 0.17]));
    let c = [x, b - h * 0.66];
    let greens = [[0.28, 0.58, 0.28], [0.36, 0.68, 0.33], [0.24, 0.52, 0.27]];
    let puffs = [(0.0, -0.05, 0.22), (-0.2, 0.08, 0.17), (0.2, 0.08, 0.18), (-0.08, -0.18, 0.16), (0.1, -0.16, 0.15)];
    for (i, (dx, dy, r)) in puffs.into_iter().enumerate() {
        out.push(circle([c[0] + dx * h, c[1] + dy * h], r * h, greens[i % 3]));
    }
    // цветы: мелкие розовые точки в пределах кроны
    for i in 0..8u32 {
        let a = hash01(it.seed + i * 7) * 2.0 * PI;
        let d = hash01(it.seed + i * 7 + 1).sqrt() * h * 0.26;
        out.push(circle([c[0] + a.cos() * d, c[1] - 0.04 * h + a.sin() * d], 2.2, [0.97, 0.72, 0.82]));
    }
}

/// Кактус: ствол и одна-две «руки».
fn cactus(it: &Item, out: &mut Vec<FlakeLook>) {
    let (x, b, h) = (it.x, it.base, it.height * 0.85);
    let r = (h * 0.085).max(3.5);
    let green = [0.28, 0.56, 0.31];
    let light = [0.36, 0.65, 0.37];
    out.push(bar([x, b - r], [x, b - h + r], r, green));
    // правая рука бывает не всегда, левая — всегда
    for (side, ya, reach, up, present) in [
        (-1.0, 0.45, 0.28, 0.28, true),
        (1.0, 0.62, 0.25, 0.22, hash01(it.seed ^ 0x77) > 0.3),
    ] {
        if !present {
            continue;
        }
        let y = b - h * ya;
        let tip = x + side * h * reach;
        out.push(bar([x, y], [tip, y], r * 0.75, green));
        out.push(bar([tip, y], [tip, y - h * up], r * 0.75, light));
    }
}

/// Голое дерево: ствол и ветви, раздваивающиеся и укорачивающиеся к концам.
fn bare(it: &Item, out: &mut Vec<FlakeLook>) {
    fn branch(out: &mut Vec<FlakeLook>, p: [f32; 2], angle: f32, len: f32, r: f32, depth: u32, seed: u32) {
        let end = [p[0] + angle.sin() * len, p[1] - angle.cos() * len];
        out.push(bar(p, end, r.max(0.8), [0.42, 0.30, 0.21]));
        if depth == 0 {
            return;
        }
        for (i, side) in [-1.0f32, 1.0].into_iter().enumerate() {
            let s = seed.wrapping_mul(31).wrapping_add(i as u32 * 17 + depth);
            let spread = 0.3 + 0.4 * hash01(s);
            let shorter = 0.66 + 0.14 * hash01(s + 5);
            branch(out, end, angle + side * spread + (hash01(s + 9) - 0.5) * 0.2, len * shorter, r * 0.66, depth - 1, s);
        }
    }
    let lean = (hash01(it.seed ^ 0x33) - 0.5) * 0.18;
    let r = (it.height * 0.04).max(1.8);
    branch(out, [it.x, it.base - r], lean, it.height * 0.36, r, 4, it.seed);
}

/// Камни: один крупный и два поменьше, как на Марсе.
fn rocks(it: &Item, out: &mut Vec<FlakeLook>) {
    let (x, b) = (it.x, it.base);
    let a = it.height * 0.42;
    let tones = [[0.50, 0.26, 0.16], [0.42, 0.21, 0.14], [0.58, 0.31, 0.20]];
    // (смещение по x, размер, оттенок)
    for (i, (dx, k, tone)) in [(0.0, 1.0, 0), (-1.25, 0.6, 1), (1.2, 0.5, 2)].into_iter().enumerate() {
        let (ra, rb) = (a * k, a * k * 0.7);
        let tilt = (hash01(it.seed + i as u32) - 0.5) * 0.3;
        out.push(shape([x + dx * a, b - rb * 0.8], [ra, rb], tilt, tones[tone]));
        // светлая грань сверху слева
        out.push(shape([x + dx * a - ra * 0.2, b - rb * 1.25], [ra * 0.55, rb * 0.3], tilt - 0.15, [0.68, 0.38, 0.25]));
    }
    let _ = FRAC_PI_2;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::materials::{KINDS, Kind};
    use crate::sim::{Rect, WindowInfo};

    fn sim_with(rect: Rect) -> Sim {
        let mut s = Sim::new(1);
        s.w = 1400.0;
        s.h = 900.0;
        s.rate = 0.0;
        s.refresh(&[WindowInfo { id: 5, rect }], None, 0.05);
        s
    }

    #[test]
    fn a_wide_window_gets_a_few_trees_and_a_narrow_one_none() {
        let wide = plan(&sim_with(Rect::new(100.0, 300.0, 800.0, 400.0)));
        assert!((2..=4).contains(&wide.len()), "{} украшений", wide.len());
        assert!(plan(&sim_with(Rect::new(100.0, 300.0, 150.0, 400.0))).is_empty());
        // стоят на верхней кромке, внутри окна и не слипаются
        for it in &wide {
            assert_eq!(it.base, 300.0);
            assert!(it.x > 108.0 && it.x < 892.0);
        }
        for pair in wide.windows(2) {
            assert!(pair[1].x - pair[0].x > 60.0, "слишком близко: {pair:?}");
        }
    }

    #[test]
    fn placement_is_stable_and_follows_the_window() {
        let a = plan(&sim_with(Rect::new(100.0, 300.0, 800.0, 400.0)));
        let b = plan(&sim_with(Rect::new(100.0, 300.0, 800.0, 400.0)));
        assert_eq!(a, b);
        // окно сдвинули — деревья сдвинулись вместе с ним
        let moved = plan(&sim_with(Rect::new(150.0, 340.0, 800.0, 400.0)));
        assert_eq!(a.len(), moved.len());
        for (p, q) in a.iter().zip(&moved) {
            assert!((q.x - p.x - 50.0).abs() < 1.5 && (q.base - p.base - 40.0).abs() < 0.01);
        }
    }

    #[test]
    fn a_window_in_front_hides_the_trees_beneath_it() {
        let mut s = Sim::new(1);
        s.w = 1400.0;
        s.h = 900.0;
        let back = WindowInfo { id: 1, rect: Rect::new(100.0, 400.0, 800.0, 400.0) };
        s.refresh(&[back], None, 0.05);
        let free = plan(&s).len();
        assert!(free >= 2);
        // переднее окно закрывает левую часть кромки заднего
        let front = WindowInfo { id: 2, rect: Rect::new(80.0, 300.0, 500.0, 300.0) };
        s.refresh(&[front, back], None, 0.05);
        let behind: Vec<_> = plan(&s).into_iter().filter(|i| i.base == 400.0).collect();
        assert!(behind.iter().all(|i| i.x > 580.0), "дерево осталось под окном: {behind:?}");
    }

    #[test]
    fn every_kind_draws_its_decoration_above_the_window_edge() {
        for kind in KINDS {
            let mut s = sim_with(Rect::new(100.0, 300.0, 800.0, 400.0));
            s.set_kind(kind);
            let mut out = Vec::new();
            looks(&s, &mut out);
            assert!(out.len() > 10 && out.len() <= DECOR_MAX, "{kind:?}: {} фигур", out.len());
            // всё стоит над кромкой (основание чуть заходит вниз) и в пределах окна по ширине
            for f in &out {
                assert!(f.center[0].is_finite() && f.center[1].is_finite() && f.color[3] <= 1.0);
                assert!(f.center[1] < 300.0 + 4.0, "{kind:?}: фигура ниже кромки: {:?}", f.center);
            }
        }
    }

    #[test]
    fn decorations_can_be_switched_off() {
        let mut s = sim_with(Rect::new(100.0, 300.0, 800.0, 400.0));
        s.decor = false;
        let mut out = Vec::new();
        looks(&s, &mut out);
        assert!(out.is_empty());
        s.decor = true;
        s.set_kind(Kind::Snow);
        looks(&s, &mut out);
        assert!(!out.is_empty());
    }
}
