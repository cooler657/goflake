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

pub(crate) fn circle(c: [f32; 2], r: f32, rgb: [f32; 3]) -> FlakeLook {
    shape(c, [r, r], 0.0, rgb)
}

/// Отрезок толщиной `2r` со скруглёнными концами от `p0` до `p1`.
pub(crate) fn bar(p0: [f32; 2], p1: [f32; 2], r: f32, rgb: [f32; 3]) -> FlakeLook {
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
            items.push(Item { x, base: s.y, height: (48.0 + 50.0 * hash01(seed ^ 0x5bd1)) / 4.0, seed });
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
    let t = sim.time();
    for it in plan(sim) {
        match (kind, role(&it)) {
            (Decor::None, _) => {}
            (Decor::Fir, Role::Feature) => house(&it, t, out),
            (Decor::Fir, Role::Animal) => sparrows(&it, t, out),
            (Decor::Fir, Role::Plain) => fir(&it, out),
            (Decor::Leafy, Role::Feature) => mushrooms(&it, out),
            (Decor::Leafy, _) => leafy(&it, out),
            (Decor::Cactus, Role::Feature) => tent(&it, t, out),
            (Decor::Cactus, Role::Animal) => lizard(&it, t, out),
            (Decor::Cactus, Role::Plain) => cactus(&it, out),
            (Decor::Bare, Role::Feature) => scarecrow(&it, out),
            (Decor::Bare, Role::Animal) => hedgehog(&it, t, out),
            (Decor::Bare, Role::Plain) => bare(&it, out),
            (Decor::Rocks, Role::Feature) => rover(&it, out),
            (Decor::Rocks, _) => rocks(&it, out),
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
    let rt = (h * 0.04).max(0.6);
    out.push(bar([x, b - rt], [x, b - trunk], rt, [0.30, 0.20, 0.12]));
    const BARS: usize = 12;
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
        if f < 1.0 / TIERS * 1.6 && w > 2.5 {
            out.push(bar([x - w * 0.38, y - r * 0.5], [x + w * 0.38, y - r * 0.5], r * 0.4, [0.97, 0.98, 1.0]));
        }
    }
    // снег на макушке
    out.push(circle([x, b - h + thick * 0.3], thick * 0.7, [0.97, 0.98, 1.0]));
}

/// Что стоит на месте украшения.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    /// Особое: зимой домик, весной грибы, в пустыне палатка с костром, осенью пугало
    /// с тыквой, на Марсе ровер.
    Feature,
    /// Зверь: воробьи, ёжик, ящерица (у дождя и бури зверей нет — там обычное).
    Animal,
    /// Обычное: дерево, кактус, камни.
    Plain,
}

/// Роль украшения: каждое третье особое, каждое пятое зверь. Определяется зерном, как и
/// положение, поэтому не мигает.
fn role(it: &Item) -> Role {
    let r = hash01(it.seed ^ 0x4f1d);
    if r < 0.3 {
        Role::Feature
    } else if r < 0.5 {
        Role::Animal
    } else {
        Role::Plain
    }
}

/// Бревенчатый домик под снежной крышей: тёплые окна, кирпичная труба с дымком и
/// гирлянда по карнизу. Чуть выше ёлки. Дым плывёт по времени `t`.
fn house(it: &Item, t: f32, out: &mut Vec<FlakeLook>) {
    let (x, b) = (it.x, it.base);
    // всё чертится для домика высотой 19 условных единиц и умножается на `k`
    let k = it.height * 1.2 / 19.0;
    let (logs, door, brick) = ([[0.50, 0.31, 0.17], [0.43, 0.26, 0.14]], [0.20, 0.12, 0.08], [0.55, 0.27, 0.20]);
    let (snow, roof) = ([0.97, 0.98, 1.0], [0.30, 0.20, 0.16]);
    // стены: пять брёвен по две условных единицы толщиной
    for j in 0..5 {
        let y = b - k * (1.0 + 2.0 * j as f32);
        out.push(bar([x - 14.0 * k, y], [x + 14.0 * k, y], k, logs[j % 2]));
    }
    // фронтон под крышей: три всё более короткие бревна
    for (j, half) in [(5, 11.5), (6, 7.5), (7, 3.5)] {
        let y = b - k * (1.0 + 2.0 * j as f32);
        out.push(bar([x - half * k, y], [x + half * k, y], k, logs[j % 2]));
    }
    // дверь и два окна с тёплым светом
    out.push(bar([x - 6.0 * k, b - 1.7 * k], [x - 6.0 * k, b - 5.3 * k], 1.6 * k, door));
    out.push(circle([x - 4.6 * k, b - 3.6 * k], (0.35 * k).max(0.25), [1.0, 0.85, 0.35]));
    for cx in [0.5, 9.0] {
        out.push(bar([x + (cx - 0.9) * k, b - 5.6 * k], [x + (cx + 0.9) * k, b - 5.6 * k], 1.9 * k, [0.25, 0.15, 0.10]));
        out.push(bar([x + (cx - 0.9) * k, b - 5.6 * k], [x + (cx + 0.9) * k, b - 5.6 * k], 1.5 * k, [1.0, 0.80, 0.40]));
    }
    // труба позади крыши
    out.push(bar([x + 8.0 * k, b - 10.0 * k], [x + 8.0 * k, b - 18.0 * k], 1.5 * k, brick));
    // крыша: два ската и снег на них
    out.push(bar([x - 17.0 * k, b - 9.2 * k], [x, b - 16.6 * k], 1.5 * k, roof));
    out.push(bar([x, b - 16.6 * k], [x + 17.0 * k, b - 9.2 * k], 1.5 * k, roof));
    out.push(bar([x - 16.6 * k, b - 10.9 * k], [x + 0.1 * k, b - 18.0 * k], 1.4 * k, snow));
    out.push(bar([x - 0.1 * k, b - 18.0 * k], [x + 16.6 * k, b - 10.9 * k], 1.4 * k, snow));
    // снежная шапка на трубе
    out.push(bar([x + 6.5 * k, b - 19.2 * k], [x + 9.5 * k, b - 19.2 * k], 0.7 * k, snow));
    // гирлянда под карнизом
    let bulbs = [[0.95, 0.25, 0.25], [0.35, 0.55, 1.0], [1.0, 0.85, 0.30], [0.40, 0.85, 0.40]];
    for i in 0..9 {
        let c = [x + (-12.8 + 3.2 * i as f32) * k, b - 9.4 * k + (i % 2) as f32 * 0.5 * k];
        out.push(circle(c, (0.45 * k).max(0.3), bulbs[i % 4]));
    }
    // дымок: три облачка поднимаются, растут и тают
    for i in 0..3 {
        let ph = (t / 5.0 + i as f32 / 3.0).fract();
        let c = [x + (8.0 + 2.5 * ph + 0.6 * (ph * 9.0).sin()) * k, b - (20.6 + 9.0 * ph) * k];
        let mut puff = circle(c, (0.9 + 1.6 * ph) * k, [0.80, 0.82, 0.86]);
        puff.color[3] = 0.45 * (1.0 - ph);
        out.push(puff);
    }
}

/// Мягкое свечение радиусом `r` и цветом `rgb`.
fn glow(c: [f32; 2], r: f32, rgb: [f32; 3], alpha: f32) -> FlakeLook {
    FlakeLook { center: c, half: [r, r], rotation: 0.0, shape: FlakeShape::Glow, color: [rgb[0], rgb[1], rgb[2], alpha] }
}

/// Грибы после дождя: красный мухомор с пятнышками и два гриба поменьше на мху.
fn mushrooms(it: &Item, out: &mut Vec<FlakeLook>) {
    let (x, b) = (it.x, it.base);
    let u = it.height / 20.0;
    let stem = [0.93, 0.89, 0.78];
    out.push(bar([x - 8.0 * u, b - 0.7 * u], [x + 9.0 * u, b - 0.7 * u], (0.7 * u).max(0.4), [0.30, 0.55, 0.25]));
    // (смещение, размер, цвет шляпки, белые пятнышки)
    for (dx, s, cap, spots) in
        [(-5.5, 1.0, [0.62, 0.38, 0.22], false), (1.0, 1.5, [0.85, 0.16, 0.14], true), (7.0, 0.8, [0.92, 0.56, 0.20], false)]
    {
        let x0 = x + dx * u;
        let sr = (0.9 * s * u).max(0.3);
        let sh = 5.0 * s * u;
        let (cr, cw) = (2.1 * s * u, 4.2 * s * u);
        out.push(bar([x0, b - sr], [x0, b - sh], sr, stem));
        let cy = b - sh - cr * 0.3;
        out.push(bar([x0 - (cw - cr), cy], [x0 + (cw - cr), cy], cr, cap));
        if spots {
            for (sx, sy, k) in [(-0.45, -0.15, 0.55), (0.1, -0.45, 0.45), (0.5, 0.0, 0.5)] {
                out.push(circle([x0 + sx * cw, cy + sy * cr], (k * s * u).max(0.3), [1.0, 0.97, 0.92]));
            }
        }
    }
}

/// Тыква с рёбрами и хвостиком; у большой вырезано лицо с тёплым светом.
fn pumpkin(out: &mut Vec<FlakeLook>, cx: f32, base: f32, r: f32, face: bool) {
    let y = base - r * 0.9;
    out.push(shape([cx, y], [r * 1.15, r * 0.9], 0.0, [0.93, 0.50, 0.12]));
    for dx in [-0.45, 0.45] {
        out.push(bar([cx + dx * r, y - 0.5 * r], [cx + dx * r, y + 0.5 * r], (0.12 * r).max(0.25), [0.80, 0.38, 0.08]));
    }
    out.push(bar([cx, y - 0.8 * r], [cx + 0.15 * r, y - 1.25 * r], (0.22 * r).max(0.3), [0.30, 0.45, 0.18]));
    if face {
        for dx in [-0.4, 0.4] {
            out.push(circle([cx + dx * r, y - 0.15 * r], (0.2 * r).max(0.3), [1.0, 0.85, 0.30]));
        }
        out.push(bar([cx - 0.35 * r, y + 0.35 * r], [cx + 0.35 * r, y + 0.35 * r], (0.12 * r).max(0.25), [1.0, 0.85, 0.30]));
    }
}

/// Пугало на шесте в клетчатой рубашке, в шляпе, с соломой из рукавов и две тыквы у ног.
fn scarecrow(it: &Item, out: &mut Vec<FlakeLook>) {
    let (x, b) = (it.x, it.base);
    let u = it.height * 1.05 / 20.0;
    let (wood, red, denim) = ([0.45, 0.30, 0.18], [0.72, 0.22, 0.18], [0.25, 0.35, 0.55]);
    let (straw, burlap, dark, hat) = ([0.90, 0.78, 0.35], [0.84, 0.70, 0.45], [0.15, 0.10, 0.08], [0.35, 0.22, 0.12]);
    let thin = (0.3 * u).max(0.3);
    // шест и перекладина
    out.push(bar([x, b - 0.5 * u], [x, b - 15.0 * u], 0.55 * u, wood));
    out.push(bar([x - 7.0 * u, b - 11.5 * u], [x + 7.0 * u, b - 11.5 * u], 0.5 * u, wood));
    // штаны, рубашка в клетку, рукава, заплатка
    for dx in [-1.2, 1.2] {
        out.push(bar([x + dx * u, b - 1.2 * u], [x + dx * u, b - 6.0 * u], 1.1 * u, denim));
    }
    out.push(bar([x, b - 6.8 * u], [x, b - 10.2 * u], 2.8 * u, red));
    out.push(bar([x - 6.0 * u, b - 11.2 * u], [x + 6.0 * u, b - 11.2 * u], 1.1 * u, red));
    out.push(bar([x - 2.4 * u, b - 8.6 * u], [x + 2.4 * u, b - 8.6 * u], (0.35 * u).max(0.3), [0.45, 0.10, 0.10]));
    out.push(circle([x + 1.2 * u, b - 9.8 * u], 0.8 * u, [0.80, 0.62, 0.25]));
    // солома торчит из рукавов и штанин
    for (sx, sy, ex, ey) in [
        (-7.0, 11.2, -9.0, 10.0),
        (-7.0, 11.2, -9.2, 12.0),
        (7.0, 11.2, 9.0, 10.0),
        (7.0, 11.2, 9.2, 12.0),
        (-1.2, 1.2, -2.4, 0.5),
        (1.2, 1.2, 2.4, 0.5),
    ] {
        out.push(bar([x + sx * u, b - sy * u], [x + ex * u, b - ey * u], thin, straw));
    }
    // голова из мешковины с пришитым лицом и шляпа
    out.push(circle([x, b - 14.5 * u], 2.4 * u, burlap));
    for dx in [-0.9, 0.9] {
        out.push(circle([x + dx * u, b - 14.9 * u], (0.35 * u).max(0.3), dark));
    }
    out.push(bar([x - 0.9 * u, b - 13.7 * u], [x + 0.9 * u, b - 13.7 * u], (0.2 * u).max(0.25), dark));
    out.push(bar([x - 3.4 * u, b - 16.5 * u], [x + 3.4 * u, b - 16.5 * u], 0.5 * u, hat));
    out.push(bar([x - 1.9 * u, b - 17.8 * u], [x + 1.9 * u, b - 17.8 * u], 1.2 * u, hat));
    out.push(bar([x - 2.2 * u, b - 17.0 * u], [x + 2.2 * u, b - 17.0 * u], (0.25 * u).max(0.3), [0.80, 0.55, 0.15]));
    // тыквы у шеста
    pumpkin(out, x - 5.8 * u, b, 1.9 * u, false);
    pumpkin(out, x + 6.5 * u, b, 2.8 * u, true);
}

/// Палатка и рядом костёр: камни, поленья, мерцающее пламя, искры и тёплый отсвет.
fn tent(it: &Item, t: f32, out: &mut Vec<FlakeLook>) {
    let (x, b) = (it.x, it.base);
    let u = it.height / 20.0;
    let cx = x - 4.0 * u;
    let fx = x + 13.0 * u;
    let flick = 0.8 + 0.2 * (t * 7.0 + it.seed as f32).sin();
    // отсвет огня ложится позади всего остального
    out.push(glow([fx, b - 3.5 * u], 10.0 * u, [1.0, 0.55, 0.20], 0.30 * flick));
    // палатка: ступенчатый треугольник из горизонтальных брусков
    const ROWS: usize = 8;
    let (th, tw) = (13.0 * u, 22.0 * u);
    let row_h = th / ROWS as f32;
    let canvas = [[0.90, 0.72, 0.42], [0.84, 0.64, 0.34]];
    for j in 0..ROWS {
        let f = (j as f32 + 0.5) / ROWS as f32;
        let w = tw * (1.0 - f);
        let r = row_h * 0.62;
        out.push(bar([cx - w / 2.0 + r.min(w / 2.0), b - f * th], [cx + w / 2.0 - r.min(w / 2.0), b - f * th], r, canvas[j % 2]));
    }
    // тёмный вход и откинутые полы
    for j in 0..4 {
        let f = (j as f32 + 0.5) / 4.0;
        let w = 6.0 * u * (1.0 - f);
        let r = 2.0 * u * 0.62;
        out.push(bar([cx - w / 2.0 + r.min(w / 2.0), b - f * 8.0 * u], [cx + w / 2.0 - r.min(w / 2.0), b - f * 8.0 * u], r, [0.22, 0.14, 0.10]));
    }
    for side in [-1.0f32, 1.0] {
        out.push(bar([cx, b - th + 0.8 * u], [cx + side * 3.6 * u, b - 0.5 * u], (0.35 * u).max(0.3), [0.62, 0.42, 0.20]));
    }
    // шест с флажком
    out.push(bar([cx, b - th], [cx, b - th - 3.0 * u], (0.3 * u).max(0.3), [0.45, 0.30, 0.18]));
    out.push(bar([cx + 0.3 * u, b - th - 2.4 * u], [cx + 2.8 * u, b - th - 1.9 * u], (0.5 * u).max(0.3), [0.85, 0.20, 0.20]));
    // костёр: камни, скрещённые поленья
    for dx in [-2.4, -0.8, 0.8, 2.4] {
        out.push(circle([fx + dx * u, b - 0.9 * u], 0.9 * u, [0.55, 0.55, 0.58]));
    }
    let log = [0.30, 0.19, 0.11];
    out.push(bar([fx - 2.0 * u, b - 1.6 * u], [fx + 2.0 * u, b - 2.4 * u], (0.55 * u).max(0.3), log));
    out.push(bar([fx - 2.0 * u, b - 2.4 * u], [fx + 2.0 * u, b - 1.6 * u], (0.55 * u).max(0.3), log));
    // языки пламени: наружные оранжевые, внутри жёлтые, высота пляшет
    for (i, (dx, h)) in [(-1.0, 3.6), (0.0, 5.0), (1.0, 3.2)].into_iter().enumerate() {
        let k = 0.75 + 0.25 * (t * 9.0 + i as f32 * 2.1 + it.seed as f32).sin();
        let top = 2.0 * u + h * k * u;
        out.push(bar([fx + dx * u, b - 2.0 * u], [fx + dx * u, b - top], 1.0 * u, [1.0, 0.50, 0.12]));
        out.push(bar([fx + dx * u, b - 2.2 * u], [fx + dx * u, b - top * 0.7], 0.55 * u, [1.0, 0.85, 0.30]));
    }
    // искры поднимаются и гаснут
    for i in 0..2 {
        let ph = (t / 2.0 + i as f32 * 0.5).fract();
        let mut spark = circle(
            [fx + (1.2 * (ph * 11.0 + i as f32 * 3.0).sin()) * u, b - (5.0 + 9.0 * ph) * u],
            (0.3 * u).max(0.3),
            [1.0, 0.70, 0.30],
        );
        spark.color[3] = 1.0 - ph;
        out.push(spark);
    }
}

/// Марсоход: шесть колёс (с боку видны три), золотистая палуба, мачта с камерой,
/// тарелка антенны и манипулятор. Смотрит то вправо, то влево — по зерну.
fn rover(it: &Item, out: &mut Vec<FlakeLook>) {
    let (x, b) = (it.x, it.base);
    let u = it.height * 0.75 / 20.0;
    let s = if hash01(it.seed ^ 0xabc) < 0.5 { 1.0 } else { -1.0 };
    let p = |lx: f32, ly: f32| [x + s * lx * u, b - ly * u];
    let (steel, dark, silver, gold) = ([0.60, 0.60, 0.64], [0.22, 0.22, 0.25], [0.88, 0.88, 0.90], [0.82, 0.64, 0.28]);
    let thin = (0.45 * u).max(0.3);
    // подвеска и колёса
    for (wx, px) in [(-9.0, -4.0), (0.0, -4.0), (9.0, 5.0)] {
        out.push(bar(p(wx, 2.3), p(px, 6.5), thin, steel));
        out.push(circle(p(wx, 2.3), 2.3 * u, dark));
        out.push(circle(p(wx, 2.3), 1.0 * u, [0.65, 0.65, 0.68]));
    }
    // корпус, золотистая палуба, генератор сзади
    out.push(bar(p(-8.0, 8.0), p(7.0, 8.0), 3.0 * u, silver));
    out.push(bar(p(-7.5, 11.4), p(6.5, 11.4), (0.7 * u).max(0.3), gold));
    out.push(bar(p(-12.0, 9.0), p(-8.6, 9.0), 1.1 * u, [0.30, 0.30, 0.33]));
    // мачта с камерой
    out.push(bar(p(3.5, 12.0), p(3.5, 18.0), thin, steel));
    out.push(bar(p(2.2, 18.6), p(5.2, 18.6), 1.0 * u, dark));
    out.push(circle(p(5.0, 18.6), (0.55 * u).max(0.3), [0.15, 0.50, 0.80]));
    // тарелка антенны
    out.push(bar(p(-4.0, 12.0), p(-4.0, 14.0), (0.3 * u).max(0.3), steel));
    out.push(circle(p(-4.0, 14.8), 1.8 * u, [0.90, 0.90, 0.92]));
    out.push(circle(p(-4.0, 14.8), 0.9 * u, [0.70, 0.70, 0.74]));
    // манипулятор с буром
    out.push(bar(p(7.5, 9.0), p(11.5, 7.0), (0.55 * u).max(0.3), steel));
    out.push(bar(p(11.5, 7.0), p(13.2, 4.2), thin, steel));
    out.push(circle(p(13.4, 3.8), (0.7 * u).max(0.3), dark));
}

/// Во сколько раз звери крупнее, чем если бы их рисовали по высоте дерева: иначе они мелкие.
const ANIMAL: f32 = 1.4;

/// Куда смотрит зверь: вправо или влево — по зерну.
fn flip(seed: u32) -> f32 {
    if hash01(seed) < 0.5 { 1.0 } else { -1.0 }
}

/// Воробьи: три птички сидят на кромке, клюют крошки и поглядывают по сторонам.
fn sparrows(it: &Item, t: f32, out: &mut Vec<FlakeLook>) {
    let (x, b) = (it.x, it.base);
    let u = it.height * ANIMAL / 20.0;
    let (body, wing, belly, tail, head) =
        ([0.56, 0.38, 0.24], [0.40, 0.26, 0.16], [0.84, 0.74, 0.60], [0.38, 0.25, 0.16], [0.50, 0.34, 0.22]);
    // (смещение, размер, фаза клевания)
    for (i, (dx, s, ph)) in [(-7.0, 1.0, 0.0), (0.0, 0.85, 2.1), (6.8, 0.95, 4.2)].into_iter().enumerate() {
        let dir = flip(it.seed.wrapping_add(i as u32 * 31));
        let k = u * s;
        let p = |lx: f32, ly: f32| [x + dx * u + dir * lx * k, b - ly * k];
        // клюёт: голова ныряет вниз и вперёд, потом поднимается
        let peck = ((t * 2.6 + ph).sin() - 0.55).max(0.0) / 0.45;
        let (hx, hy) = (2.6 + peck * 0.9, 4.6 - peck * 1.5);
        out.push(bar(p(-1.8, 3.3), p(-5.6, 2.0), (0.5 * k).max(0.3), tail));
        out.push(shape(p(0.0, 3.0), [3.0 * k, 2.1 * k], 0.0, body));
        out.push(shape(p(0.7, 2.3), [2.0 * k, 1.2 * k], 0.0, belly));
        out.push(shape(p(-0.8, 3.3), [2.0 * k, 0.9 * k], 0.0, wing));
        out.push(circle(p(hx, hy), 1.5 * k, head));
        out.push(bar(p(hx + 1.4, hy - 0.1), p(hx + 2.6, hy - 0.4), (0.35 * k).max(0.3), [0.95, 0.72, 0.28]));
        out.push(circle(p(hx + 0.4, hy + 0.3), (0.3 * k).max(0.3), [0.10, 0.08, 0.06]));
        for lx in [0.1, 1.1] {
            out.push(bar(p(lx, 1.2), p(lx, 0.2), (0.2 * k).max(0.25), [0.78, 0.55, 0.30]));
        }
    }
}

/// Ёжик с яблоком на иголках: топает лапками и водит носом.
fn hedgehog(it: &Item, t: f32, out: &mut Vec<FlakeLook>) {
    let (x, b) = (it.x, it.base);
    let u = it.height * ANIMAL / 20.0;
    let dir = flip(it.seed ^ 0x51);
    let p = |lx: f32, ly: f32| [x + dir * lx * u, b - ly * u];
    let (spine_a, spine_b, fur, paw) = ([0.38, 0.26, 0.16], [0.62, 0.45, 0.28], [0.85, 0.70, 0.55], [0.55, 0.40, 0.30]);
    let thin = (0.3 * u).max(0.3);
    // лапки переступают, нос принюхивается
    let step = (t * 5.0).sin();
    let sniff = 0.25 * (t * 6.0).sin();
    for (fx, sign) in [(-3.0, 1.0), (2.5, -1.0)] {
        let sx = fx + step * sign * 0.6;
        out.push(bar(p(sx, 1.4), p(sx, 0.4), (0.6 * u).max(0.3), paw));
    }
    // тело-купол и иголки веером по спине, откинутые назад
    out.push(shape(p(-0.5, 3.6), [5.4 * u, 3.2 * u], 0.0, spine_a));
    for i in 0..10 {
        let a = std::f32::consts::PI * (1.0 - (i as f32 + 0.5) / 10.0);
        let (c, s) = (a.cos(), a.sin());
        let base = (-0.5 + 4.9 * c, 3.6 + 2.8 * s);
        out.push(bar(p(base.0, base.1), p(base.0 + 1.7 * c - 0.7, base.1 + 1.9 * s), thin, if i % 2 == 0 { spine_a } else { spine_b }));
    }
    // мордочка: ушко, глаз, длинный нос
    out.push(circle(p(4.2, 4.4), 0.8 * u, paw));
    out.push(circle(p(5.2, 2.6), 2.0 * u, fur));
    out.push(bar(p(5.9, 2.5), p(7.6, 2.0 + sniff), 0.9 * u, fur));
    out.push(circle(p(8.1, 2.0 + sniff), (0.55 * u).max(0.3), [0.12, 0.09, 0.08]));
    out.push(circle(p(5.6, 3.2), (0.4 * u).max(0.3), [0.12, 0.09, 0.08]));
    // яблоко, наколотое на иголки
    out.push(circle(p(-0.2, 7.6), 1.3 * u, [0.80, 0.18, 0.15]));
    out.push(bar(p(-0.2, 8.7), p(0.1, 9.6), (0.2 * u).max(0.25), [0.30, 0.45, 0.18]));
}

/// Ящерица на камне: отжимается на передних лапах, качает хвостом и показывает язык.
fn lizard(it: &Item, t: f32, out: &mut Vec<FlakeLook>) {
    let (x, b) = (it.x, it.base);
    let u = it.height * ANIMAL / 20.0;
    let dir = flip(it.seed ^ 0x77);
    let p = |lx: f32, ly: f32| [x + dir * lx * u, b - ly * u];
    let (green, dark, spot) = ([0.46, 0.60, 0.28], [0.30, 0.42, 0.18], [0.70, 0.80, 0.40]);
    let thin = (0.5 * u).max(0.3);
    let up = (t * 2.2).sin().max(0.0);
    let sway = (t * 3.0).sin();
    // хвост: цепочка отрезков, сужающихся к закрученному кончику
    let pts = [(-4.0, 2.0), (-7.5, 1.4), (-11.0, 1.8), (-13.2, 3.2 + sway * 0.8), (-13.4, 5.0 + sway * 1.2)];
    let rs = [1.3, 1.0, 0.75, 0.55];
    for i in 0..4 {
        out.push(bar(p(pts[i].0, pts[i].1), p(pts[i + 1].0, pts[i + 1].1), (rs[i] * u).max(0.3), green));
    }
    // лапы растопырены
    for (a, bx) in [(2.2, 3.8), (0.8, 2.2), (-3.0, -4.6), (-4.4, -5.8)] {
        out.push(bar(p(a, 1.8 + up * 0.5), p(bx, 0.4), thin, dark));
    }
    // тело с пятнами вдоль спины, голова, глаз
    out.push(bar(p(-4.5, 2.3), p(3.5, 2.3 + up * 0.6), 1.7 * u, green));
    for (sx, sy) in [(-2.5, 3.5), (0.0, 3.6), (2.2, 3.6 + up * 0.6)] {
        out.push(circle(p(sx, sy), (0.45 * u).max(0.3), spot));
    }
    let hy = 2.9 + up * 1.4;
    out.push(bar(p(4.2, hy), p(6.6, hy - 0.2), 1.2 * u, green));
    out.push(circle(p(5.0, hy + 0.7), (0.45 * u).max(0.3), [0.12, 0.09, 0.05]));
    // язычок мелькает раз в несколько секунд
    if (t / 3.0 + it.seed as f32 * 0.37).fract() < 0.1 {
        out.push(bar(p(7.8, hy - 0.2), p(9.4, hy + 0.4), (0.2 * u).max(0.25), [0.85, 0.20, 0.25]));
    }
}

/// Дерево весной: тёмный ствол, пышная зелёная крона и розовые цветы.
fn leafy(it: &Item, out: &mut Vec<FlakeLook>) {
    let (x, b, h) = (it.x, it.base, it.height);
    let rt = (h * 0.045).max(0.6);
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
        out.push(circle([c[0] + a.cos() * d, c[1] - 0.04 * h + a.sin() * d], 0.7, [0.97, 0.72, 0.82]));
    }
}

/// Кактус: ствол и одна-две «руки».
fn cactus(it: &Item, out: &mut Vec<FlakeLook>) {
    let (x, b, h) = (it.x, it.base, it.height * 0.85);
    let r = (h * 0.085).max(1.1);
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
        out.push(bar(p, end, r.max(0.35), [0.42, 0.30, 0.21]));
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
    let r = (it.height * 0.04).max(0.6);
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
            assert!(out.len() > 5 && out.len() <= DECOR_MAX, "{kind:?}: {} фигур", out.len());
            // всё стоит над кромкой (основание чуть заходит вниз) и в пределах окна по ширине
            for f in &out {
                assert!(f.center[0].is_finite() && f.center[1].is_finite() && f.color[3] <= 1.0);
                assert!(f.center[1] < 300.0 + 4.0, "{kind:?}: фигура ниже кромки: {:?}", f.center);
            }
        }
    }

    #[test]
    fn about_a_third_of_decorations_are_features_and_they_stand_on_the_edge() {
        let items: Vec<Item> = (0..600u32)
            .map(|i| Item { x: 500.0, base: 300.0, height: 20.0, seed: i.wrapping_mul(2_654_435_761) })
            .collect();
        let n = items.iter().filter(|it| role(it) == Role::Feature).count();
        assert!((120..=240).contains(&n), "особых украшений {n} из 600");
        let animals = items.iter().filter(|it| role(it) == Role::Animal).count();
        assert!((70..=170).contains(&animals), "зверей {animals} из 600");
        for height in [12.0f32, 18.0, 25.0] {
            let it = Item { x: 500.0, base: 300.0, height, seed: 7 };
            for t in [0.0f32, 1.7, 4.9] {
                let mut out = Vec::new();
                house(&it, t, &mut out);
                assert!(out.len() > 20 && out.len() <= 60, "{} фигур", out.len());
                let half = height * 1.2 / 19.0 * 18.0;
                for f in &out {
                    assert!(f.center[0].is_finite() && f.center[1].is_finite() && f.color[3] <= 1.0 && f.color[3] >= 0.0);
                    assert!(f.center[1] < 300.0, "фигура ниже кромки: {:?}", f.center);
                    assert!((f.center[0] - 500.0).abs() <= half, "фигура вылезла вбок: {:?}", f.center);
                }
            }
        }
        // дом выше ёлки того же размера
        let (mut h, mut f) = (Vec::new(), Vec::new());
        let it = Item { x: 500.0, base: 300.0, height: 20.0, seed: 1 };
        house(&it, 0.0, &mut h);
        fir(&it, &mut f);
        let top = |v: &Vec<FlakeLook>| v.iter().filter(|s| s.color[3] > 0.99).map(|s| s.center[1] - s.half[1]).fold(f32::MAX, f32::min);
        assert!(top(&h) < top(&f) - 2.0, "домик не выше ёлки: {} и {}", top(&h), top(&f));
    }

    /// Сохраняет кадры украшений каждого вида осадков, чтобы посмотреть на них глазами.
    /// `SNOWDRIFT_DUMP=/папка cargo test dump_decor -- --ignored`; формат — как у
    /// `soft::tests::dump_preview` (`имя.raw` и `имя.txt`).
    #[test]
    #[ignore]
    fn dump_decor() {
        use crate::clock::Clock;
        use crate::sky::{Sky, SkyMode};
        let Some(dir) = std::env::var_os("SNOWDRIFT_DUMP") else { return };
        let dir = std::path::PathBuf::from(dir);
        for kind in KINDS {
            let mut s = Sim::new(1);
            s.w = 1400.0;
            s.h = 160.0;
            s.rate = 0.0;
            s.set_kind(kind);
            // шесть окон подряд: три с особым украшением, три со зверем (если они есть у вида)
            let want = |r: Role| -> Vec<i64> {
                (1..400i64)
                    .filter(|id| role(&Item { x: 0.0, base: 0.0, height: 20.0, seed: (*id as u32).wrapping_mul(2_654_435_761) }) == r)
                    .take(3)
                    .collect()
            };
            let ids: Vec<i64> = want(Role::Feature).into_iter().chain(want(Role::Animal)).collect();
            let wins: Vec<WindowInfo> = ids
                .iter()
                .enumerate()
                .map(|(i, id)| WindowInfo { id: *id, rect: Rect::new(i as f32 * 233.0, 90.0, 233.0, 90.0) })
                .collect();
            s.refresh(&wins, None, 0.05);
            for t in [0.0f32, 2.0] {
                s.step(t.max(0.001), (-500.0, -500.0), (0.0, 0.0));
            }
            let mut canvas = crate::soft::Canvas::new();
            let sky = Sky::new(SkyMode::Off, Clock { hour: 12.0, season: 5.5, unix: 0.0 });
            canvas.draw(&s, &sky, 3.0);
            let bytes: Vec<u8> = canvas.pixels.iter().flat_map(|p| p.to_le_bytes()).collect();
            let name = format!("decor_{kind:?}").to_lowercase();
            std::fs::write(dir.join(format!("{name}.raw")), bytes).unwrap();
            std::fs::write(dir.join(format!("{name}.txt")), format!("{} {}", canvas.width, canvas.height)).unwrap();
        }
    }

    #[test]
    fn every_special_piece_stands_on_the_edge_and_stays_near_its_spot() {
        type Piece = fn(&Item, &mut Vec<FlakeLook>);
        let pieces: [(&str, Piece); 7] = [
            ("грибы", mushrooms),
            ("пугало", scarecrow),
            ("палатка", |it, out| tent(it, 1.3, out)),
            ("ровер", rover),
            ("воробьи", |it, out| sparrows(it, 1.3, out)),
            ("ёжик", |it, out| hedgehog(it, 1.3, out)),
            ("ящерица", |it, out| lizard(it, 0.7, out)),
        ];
        for (name, piece) in pieces {
            for height in [12.0f32, 18.0, 25.0] {
                for seed in [1u32, 2, 3, 4, 5, 6] {
                    let it = Item { x: 500.0, base: 300.0, height, seed };
                    let mut out = Vec::new();
                    piece(&it, &mut out);
                    assert!(out.len() >= 8 && out.len() <= 70, "{name}: {} фигур", out.len());
                    for f in &out {
                        assert!(f.center[0].is_finite() && f.center[1].is_finite(), "{name}");
                        assert!((0.0..=1.0).contains(&f.color[3]), "{name}: прозрачность {}", f.color[3]);
                        assert!(f.center[1] < 300.0, "{name}: фигура ниже кромки: {:?}", f.center);
                        assert!((f.center[0] - 500.0).abs() <= height * 1.2, "{name}: вылезла вбок: {:?}", f.center);
                    }
                }
            }
        }
        // костёр мерцает: пламя в разные моменты разной высоты, но не пропадает
        let it = Item { x: 500.0, base: 300.0, height: 20.0, seed: 9 };
        let (mut a, mut b) = (Vec::new(), Vec::new());
        tent(&it, 0.0, &mut a);
        tent(&it, 0.35, &mut b);
        assert_eq!(a.len(), b.len());
        assert!(a.iter().zip(&b).any(|(p, q)| p.center != q.center || p.color != q.color));
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
