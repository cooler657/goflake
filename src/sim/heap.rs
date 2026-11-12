//! Шары и кучи, которые собирает курсор.
//!
//! Если провести курсором вдоль кучи у кромки окна, она не разлетается, а
//! сгребается в шар (снег) или в холмик (песок, листья, пыль). Шар катится за
//! курсором и растёт, подбирая всё на пути. Снежные шары можно ставить друг на
//! друга — выйдет снеговик; холмики при встрече сливаются.
//!
//! Масса сохраняется: всё, что сгребено, лежит в шаре, а когда шар падает с края
//! или окно сдвигают, он рассыпается обратно в частицы.

use std::f32::consts::PI;

use super::*;
use crate::look::{FlakeLook, FlakeShape, hash01};
use crate::materials::Gather;

/// Сколько фигур кучи могут отдать за кадр (под них запасён буфер на GPU).
pub const HEAP_MAX: usize = 512;
/// Сколько шаров и куч может быть сразу. Лишние старые рассыпаются.
pub const MAX_HEAPS: usize = 12;
/// Сколько из них может быть снеговиками (два шара и больше).
pub const MAX_SNOWMEN: usize = 5;
/// Наибольший радиус шара, точки.
const R_MAX: f32 = 13.0;
/// Наименьший радиус, при котором курсор вообще что-то сгребает.
const R_CREATE: f32 = 3.0;
/// Сколько шаров в снеговике.
const MAX_TIERS: usize = 3;
const MAX_MASS: f32 = PI * R_MAX * R_MAX;

/// Шар, холмик или снеговик на поверхности.
pub struct Heap {
    pub surface: i64,
    pub x: f32,
    pub vx: f32,
    /// Массы ярусов снизу вверх. У холмика — один ярус, у снеговика — два-три.
    pub tiers: Vec<f32>,
    pub kind: Kind,
    /// Где была поверхность, когда куча на ней появилась: сдвинули окно — куча падает.
    pub anchor: (f32, f32),
    pub seed: u32,
    /// Сколько секунд прошло с появления или с последней перестройки (слияния, нового яруса):
    /// самые «старые» рассыпаются первыми, когда шаров слишком много.
    pub age: f32,
}

fn radius(mass: f32) -> f32 {
    (mass / PI).sqrt()
}

impl Heap {
    pub fn mass(&self) -> f32 {
        self.tiers.iter().sum()
    }

    fn style(&self) -> Gather {
        self.kind.material().gather
    }

    /// Половина ширины основания.
    fn half_width(&self) -> f32 {
        let r = radius(self.tiers[0]);
        if self.style() == Gather::Heap { r * 1.5 } else { r }
    }

    /// Центры и радиусы ярусов шара при основании на высоте `base` (y вниз).
    fn ball_layout(&self, base: f32) -> Vec<(f32, f32)> {
        let mut out: Vec<(f32, f32)> = Vec::with_capacity(self.tiers.len());
        for &m in &self.tiers {
            let r = radius(m);
            let cy = match out.last() {
                None => base - r,
                Some(&(py, pr)) => py - 0.82 * (pr + r),
            };
            out.push((cy, r));
        }
        out
    }

    /// Высота над основанием.
    fn height(&self) -> f32 {
        match self.style() {
            Gather::Heap => radius(self.tiers[0]) * 1.4,
            _ => {
                let l = self.ball_layout(0.0);
                let (cy, r) = *l.last().unwrap();
                -(cy - r)
            }
        }
    }
}

impl Sim {
    /// Рассыпает шар обратно в частицы. `land` — смогут ли они лечь на поверхности.
    fn drop_heap(&mut self, heap: Heap, land: bool) {
        let base = self.surfaces.get(&heap.surface).map_or(heap.anchor.1, |s| s.y);
        let chunk = heap.kind.material().chunk;
        let r = heap.half_width();
        for m in heap.tiers.iter().copied() {
            let k = ((m / (chunk * 2.0)) as usize).clamp(1, 30);
            for _ in 0..k {
                let (x, y) = (heap.x + self.rng.range(-r, r), base - self.rng.range(0.0, r * 1.6));
                let (vx, vy) = (heap.vx * 0.5 + self.rng.range(-30.0, 30.0), -self.rng.range(10.0, 70.0));
                debris(&mut self.particles, &mut self.rng, heap.kind, land, x, y, vx, vy, m / k as f32);
            }
        }
    }

    /// Рассыпает все шары: при смене материала или «стряхнуть всё».
    pub(super) fn drop_all_heaps(&mut self, land: bool) {
        for h in std::mem::take(&mut self.heaps) {
            self.drop_heap(h, land);
        }
    }

    /// Шаг шаров и куч: рождение под курсором, толкание, подбор, столкновения.
    pub(super) fn heaps_step(&mut self, dt: f32, c: (f32, f32), cv: (f32, f32)) {
        let kind = self.kind;
        for h in &mut self.heaps {
            h.age += dt;
        }
        // 1. упавшие: поверхность исчезла, сдвинулась или кончилась под шаром
        let mut i = 0;
        while i < self.heaps.len() {
            let h = &self.heaps[i];
            let (hw, x) = (h.half_width(), h.x);
            let stands = self.surfaces.get(&h.surface).is_some_and(|s| {
                (s.x0 - h.anchor.0).abs() < 0.5
                    && (s.y - h.anchor.1).abs() < 0.5
                    && x - hw > s.x0
                    && x + hw < s.x0 + s.h.len() as f32 * COL_W
            });
            if stands {
                i += 1;
            } else {
                let h = self.heaps.swap_remove(i);
                self.drop_heap(h, true);
            }
        }

        // 2. рождение: быстрый проход курсора у кромки сгребает кучу в шар
        if cv.0.abs() > 60.0 {
            let mut born = Vec::new();
            for (&id, s) in self.surfaces.iter_mut() {
                if !(c.1 < s.y + 8.0 && c.1 > s.y - 60.0) {
                    continue;
                }
                let col = ((c.0 - s.x0) / COL_W).floor() as isize;
                if col < 0 || col as usize >= s.h.len() || !s.vis[col as usize] {
                    continue;
                }
                let taken = |hs: &[Heap]| hs.iter().any(|h| h.surface == id && (h.x - c.0).abs() < h.half_width() + 10.0);
                if taken(&self.heaps) || taken(&born) {
                    continue;
                }
                let (a, b) = ((col - 4).max(0) as usize, (col + 4).min(s.h.len() as isize - 1) as usize);
                let mass = (a..=b).map(|k| s.h[k]).sum::<f32>() * COL_W;
                if mass < PI * R_CREATE * R_CREATE {
                    continue;
                }
                for k in a..=b {
                    s.h[k] = 0.0;
                }
                born.push(Heap {
                    surface: id,
                    x: c.0,
                    vx: cv.0 * 0.8,
                    tiers: vec![mass.min(MAX_MASS)],
                    kind,
                    anchor: (s.x0, s.y),
                    seed: (c.0 as i32 as u32).wrapping_mul(2_654_435_761) ^ (id as u32),
                    age: 0.0,
                });
            }
            self.heaps.extend(born);
        }

        // 3. толкание курсором, качение и подбор всего на пути
        for i in 0..self.heaps.len() {
            let id = self.heaps[i].surface;
            let Some(s) = self.surfaces.get_mut(&id) else { continue };
            let h = &mut self.heaps[i];
            let (hw, top) = (h.half_width(), s.y - h.height());
            let near = (c.0 - h.x).abs() < hw + 16.0 && c.1 > top - 30.0 && c.1 < s.y + 10.0;
            if near {
                h.vx += (cv.0 * 0.95 - h.vx) * (12.0 * dt).min(1.0);
            } else {
                h.vx *= (-3.0 * dt).exp();
            }
            if h.vx.abs() < 2.0 {
                h.vx = 0.0;
            }
            let nx = h.x + h.vx * dt;
            // плуг: шар подбирает всё, что лежит перед ним; растёт только одиночный
            if h.tiers.len() == 1 && h.vx != 0.0 {
                let dir = h.vx.signum();
                let col = |x: f32| ((x - s.x0) / COL_W).floor() as isize;
                let (c0, c1) = (col(h.x + dir * hw), col(nx + dir * hw));
                // диапазон держим внутри поверхности: иначе из-за одной ошибки в x он мог бы
                // оказаться огромным, и цикл не закончился бы
                let last = s.h.len() as isize - 1;
                for k in c0.min(c1).max(0)..=c0.max(c1).min(last) {
                    if k < 0 || k > last || !s.vis[k as usize] {
                        continue;
                    }
                    let k = k as usize;
                    let m = s.h[k] * COL_W;
                    if m > 0.0 && h.tiers[0] + m <= MAX_MASS {
                        h.tiers[0] += m;
                        s.h[k] = 0.0;
                    }
                }
            }
            h.x = nx;
        }

        // 4. столкновения: шары ставятся друг на друга, холмики сливаются. Цикл
        // перезапускается только когда число куч уменьшилось, поэтому он кончается:
        // упёршиеся шары лишь останавливаются, а раздвигает их `separate` ниже
        'again: loop {
            let n = self.heaps.len();
            for a in 0..n {
                for b in a + 1..n {
                    if self.heaps[a].surface != self.heaps[b].surface {
                        continue;
                    }
                    let need = self.heaps[a].half_width() + self.heaps[b].half_width();
                    if (self.heaps[a].x - self.heaps[b].x).abs() >= need {
                        continue;
                    }
                    let (m, t) = if self.heaps[a].vx.abs() >= self.heaps[b].vx.abs() { (a, b) } else { (b, a) };
                    if self.collide(m, t) {
                        continue 'again;
                    }
                }
            }
            break;
        }
        self.separate();
        self.enforce_limits();
    }

    /// Рассыпает самых старых, если снеговиков или шаров вообще стало больше предела.
    /// Рассыпанное превращается в частицы, так что масса не пропадает.
    fn enforce_limits(&mut self) {
        let oldest = |heaps: &[Heap], only_snowmen: bool| {
            heaps
                .iter()
                .enumerate()
                .filter(|(_, h)| !only_snowmen || h.tiers.len() >= 2)
                .max_by(|a, b| a.1.age.total_cmp(&b.1.age))
                .map(|(i, _)| i)
        };
        while self.heaps.iter().filter(|h| h.tiers.len() >= 2).count() > MAX_SNOWMEN {
            let Some(i) = oldest(&self.heaps, true) else { break };
            let h = self.heaps.swap_remove(i);
            self.drop_heap(h, true);
        }
        while self.heaps.len() > MAX_HEAPS {
            let Some(i) = oldest(&self.heaps, false) else { break };
            let h = self.heaps.swap_remove(i);
            self.drop_heap(h, true);
        }
    }

    /// Раздвигает наложившиеся шары, не меняя их числа: слева направо, каждый
    /// следующий отодвигается от предыдущего. Так ряд тесно стоящих шаров
    /// сдвигается целиком, а выдавленный за край падает и рассыпается.
    fn separate(&mut self) {
        let mut order: Vec<usize> = (0..self.heaps.len()).collect();
        order.sort_by(|&a, &b| {
            let (ha, hb) = (&self.heaps[a], &self.heaps[b]);
            ha.surface.cmp(&hb.surface).then(ha.x.total_cmp(&hb.x))
        });
        for pair in order.windows(2) {
            let (l, r) = (pair[0], pair[1]);
            if self.heaps[l].surface != self.heaps[r].surface {
                continue;
            }
            let need = self.heaps[l].half_width() + self.heaps[r].half_width();
            if self.heaps[r].x - self.heaps[l].x < need {
                self.heaps[r].x = self.heaps[l].x + need + 0.01;
            }
        }
    }

    /// Мовер `m` врезался в цель `t`: поставить сверху или слить (тогда `true`, число
    /// куч уменьшилось) либо просто остановиться (`false`).
    fn collide(&mut self, m: usize, t: usize) -> bool {
        let single = self.heaps[m].tiers.len() == 1;
        let (mm, tm) = (self.heaps[m].mass(), self.heaps[t].mass());
        let merge_ok = single && self.heaps[t].tiers.len() == 1 && mm + tm <= MAX_MASS;
        let (rm, r_top, r_base) =
            (radius(self.heaps[m].tiers[0]), radius(*self.heaps[t].tiers.last().unwrap()), radius(self.heaps[t].tiers[0]));
        let action = match self.heaps[t].style() {
            Gather::Ball if single && self.heaps[t].tiers.len() < MAX_TIERS && rm < 0.85 * r_top => 1,
            Gather::Ball if merge_ok && rm >= 0.85 * r_base => 2,
            Gather::Heap if merge_ok => 2,
            _ => 0,
        };
        match action {
            1 | 2 => {
                let mover = self.heaps.remove(m);
                let t = if t > m { t - 1 } else { t };
                let target = &mut self.heaps[t];
                if action == 1 {
                    target.tiers.push(mover.tiers[0]);
                } else {
                    let w = target.mass() + mover.mass();
                    target.x = (target.x * target.mass() + mover.x * mover.mass()) / w;
                    target.tiers[0] += mover.tiers[0];
                }
                target.vx = (target.vx + mover.vx) * 0.3;
                target.age = 0.0;
                true
            }
            _ => {
                // упёрлись: мовер останавливается, раздвинет их `separate`
                self.heaps[m].vx = 0.0;
                false
            }
        }
    }
}

/// Высота слоя под шаром: на нём он стоит.
fn ground(s: &Surface, x: f32) -> f32 {
    let col = ((x - s.x0) / COL_W) as isize;
    let (a, b) = ((col - 2).max(0) as usize, (col + 2).min(s.h.len() as isize - 1).max(0) as usize);
    s.h[a..=b].iter().sum::<f32>() / (b - a + 1) as f32
}

fn circle(c: [f32; 2], r: f32, rgb: [f32; 3]) -> FlakeLook {
    FlakeLook { center: c, half: [r, r], rotation: 0.0, shape: FlakeShape::Capsule, color: [rgb[0], rgb[1], rgb[2], 1.0] }
}

fn bar(p0: [f32; 2], p1: [f32; 2], r: f32, rgb: [f32; 3]) -> FlakeLook {
    let (dx, dy) = (p1[0] - p0[0], p1[1] - p0[1]);
    let len = (dx * dx + dy * dy).sqrt();
    FlakeLook {
        center: [(p0[0] + p1[0]) / 2.0, (p0[1] + p1[1]) / 2.0],
        half: [len / 2.0 + r, r],
        rotation: dy.atan2(dx),
        shape: FlakeShape::Capsule,
        color: [rgb[0], rgb[1], rgb[2], 1.0],
    }
}

fn mix(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// Дописывает в `out` фигуры шаров и куч.
pub fn looks(sim: &Sim, out: &mut Vec<FlakeLook>) {
    let start = out.len();
    for h in &sim.heaps {
        let Some(s) = sim.surfaces.get(&h.surface) else { continue };
        let col = ((h.x - s.x0) / COL_W) as isize;
        if col < 0 || col as usize >= s.vis.len() || !s.vis[col as usize] {
            continue;
        }
        let base = s.y - ground(s, h.x);
        match h.style() {
            Gather::Ball => snow_ball(h, base, out),
            _ => mound(h, base, out),
        }
        if out.len() - start >= HEAP_MAX {
            break;
        }
    }
    out.truncate(start + HEAP_MAX);
}

/// Снежный шар или снеговик: объёмные шары, у снеговика — лицо, пуговицы и руки.
fn snow_ball(h: &Heap, base: f32, out: &mut Vec<FlakeLook>) {
    let layout = h.ball_layout(base);
    for &(cy, r) in &layout {
        out.push(circle([h.x, cy], r + 0.5, [0.66, 0.74, 0.90]));
        out.push(circle([h.x, cy], r, [0.80, 0.86, 0.96]));
        out.push(circle([h.x - r * 0.07, cy - r * 0.09], r * 0.88, [0.98, 0.99, 1.0]));
    }
    if layout.len() < 2 {
        return;
    }
    let dark = [0.12, 0.12, 0.15];
    let &(hy, hr) = layout.last().unwrap();
    // лицо: глаза, морковка, рот
    for dx in [-0.32, 0.32] {
        out.push(circle([h.x + dx * hr, hy - hr * 0.2], (hr * 0.11).max(0.5), dark));
    }
    out.push(bar([h.x, hy + hr * 0.02], [h.x + hr * 0.75, hy + hr * 0.14], (hr * 0.1).max(0.5), [0.96, 0.50, 0.10]));
    for k in -1..=1 {
        out.push(circle([h.x + k as f32 * hr * 0.2, hy + hr * 0.42], (hr * 0.07).max(0.4), dark));
    }
    // пуговицы на среднем ярусе (при двух ярусах — на нижнем)
    let mid = if layout.len() >= 3 { layout[1] } else { layout[0] };
    for k in 0..3 {
        out.push(circle([h.x + mid.1 * 0.05, mid.0 - mid.1 * 0.4 + k as f32 * mid.1 * 0.38], (mid.1 * 0.09).max(0.5), dark));
    }
    // руки-веточки
    for side in [-1.0f32, 1.0] {
        let p0 = [h.x + side * mid.1 * 0.85, mid.0 - mid.1 * 0.1];
        out.push(bar(p0, [p0[0] + side * mid.1 * 0.9, p0[1] - mid.1 * 0.7], (mid.1 * 0.05).max(0.4), [0.38, 0.26, 0.16]));
    }
}

/// Холмик из песка, листьев или пыли: гора из горизонтальных брусков, у листьев — ещё
/// и россыпь листиков.
fn mound(h: &Heap, base: f32, out: &mut Vec<FlakeLook>) {
    let m = h.kind.material();
    let r = radius(h.tiers[0]);
    let (w, height) = (r * 3.0, r * 1.4);
    const BARS: usize = 10;
    let thick = height / BARS as f32;
    for j in 0..BARS {
        let t = (j as f32 + 0.5) / BARS as f32;
        let width = w * (1.0 - t).powf(0.6);
        if width < 1.5 {
            continue;
        }
        let rb = thick * 0.62;
        let y = base - t * height;
        let rgb = if m.pile_mottled {
            let pick = (hash01(h.seed.wrapping_add(j as u32 * 13)) * m.colors.len() as f32) as usize % m.colors.len();
            m.colors[pick]
        } else {
            mix(m.pile_base, m.pile_top, t)
        };
        out.push(bar([h.x - width / 2.0 + rb, y], [h.x + width / 2.0 - rb, y], rb, rgb));
    }
    if m.pile_mottled {
        for i in 0..7u32 {
            let t = 0.25 + 0.6 * hash01(h.seed ^ (i * 31));
            let width = w * (1.0 - t).powf(0.6);
            let pick = (hash01(h.seed ^ (i * 17 + 5)) * m.colors.len() as f32) as usize % m.colors.len();
            let c = m.colors[pick];
            out.push(FlakeLook {
                center: [h.x + (hash01(h.seed ^ (i * 7 + 3)) - 0.5) * width * 0.9, base - t * height],
                half: [2.6, 1.3],
                rotation: hash01(h.seed ^ (i * 5 + 1)) * 3.14,
                shape: FlakeShape::Leaf,
                color: [c[0], c[1], c[2], 1.0],
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::{Rect, WindowInfo};

    const WIN: Rect = Rect { x: 100.0, y: 300.0, w: 800.0, h: 400.0 };

    /// Снег на окне ровным слоем в 6 pt, без падающих частиц.
    fn field(kind: Kind) -> Sim {
        let mut s = Sim::new(1);
        s.w = 1400.0;
        s.h = 900.0;
        s.rate = 0.0;
        s.set_kind(kind);
        s.refresh(&[WindowInfo { id: 5, rect: WIN }], None, 0.05);
        for h in s.surfaces.get_mut(&5).unwrap().h.iter_mut() {
            *h = 6.0;
        }
        s
    }

    /// То же, но поверхность голая: шары ничего не подбирают по пути.
    fn bare(kind: Kind) -> Sim {
        let mut s = field(kind);
        for h in s.surfaces.get_mut(&5).unwrap().h.iter_mut() {
            *h = 0.0;
        }
        s
    }

    fn pile_mass(s: &Sim) -> f32 {
        s.surfaces.values().flat_map(|s| s.h.iter()).sum::<f32>() * COL_W
    }

    fn debris_mass(s: &Sim) -> f32 {
        s.particles.iter().map(|p| p.mass).sum()
    }

    fn total(s: &Sim) -> f32 {
        pile_mass(s) + debris_mass(s) + s.heaps.iter().map(|h| h.mass()).sum::<f32>()
    }

    /// Проводит курсор вдоль кромки окна: из `x0` со скоростью `vx` в течение `secs`.
    fn sweep(s: &mut Sim, x0: f32, vx: f32, secs: f32) -> f32 {
        let steps = (secs * 60.0) as usize;
        let mut x = x0;
        for _ in 0..steps {
            x += vx / 60.0;
            s.step(1.0 / 60.0, (x, 296.0), (vx, 0.0));
        }
        x
    }

    fn heap_at(x: f32, mass: f32, kind: Kind, vx: f32) -> Heap {
        Heap { surface: 5, x, vx, tiers: vec![mass], kind, anchor: (108.0, 300.0), seed: 1, age: 0.0 }
    }

    #[test]
    fn a_fast_pass_gathers_the_pile_into_a_ball_and_keeps_the_mass() {
        let mut s = field(Kind::Snow);
        let before = total(&s);
        s.step(1.0 / 60.0, (400.0, 296.0), (200.0, 0.0));
        assert_eq!(s.heaps.len(), 1, "шар не собрался");
        assert!(s.heaps[0].mass() > PI * R_CREATE * R_CREATE);
        assert!((total(&s) - before).abs() < before * 0.005, "масса не сохранилась: {} против {before}", total(&s));
        // медленное движение сгребать не должно, и разлёта на месте больше нет
        let mut slow = field(Kind::Snow);
        slow.step(1.0 / 60.0, (400.0, 296.0), (30.0, 0.0));
        assert!(slow.heaps.is_empty() && slow.particles.is_empty());
    }

    #[test]
    fn the_ball_rolls_after_the_cursor_and_grows() {
        let mut s = field(Kind::Snow);
        let before = total(&s);
        let x = sweep(&mut s, 400.0, 200.0, 1.0);
        let h = &s.heaps[0];
        assert!(h.x > 480.0 && (h.x - x).abs() < 40.0, "шар отстал от курсора: {} против {x}", h.x);
        assert!(h.mass() > 100.0, "шар не вырос: {}", h.mass());
        assert!((total(&s) - before).abs() < before * 0.005, "масса потерялась: {} против {before}", total(&s));
        // курсор ушёл вверх — шар катится по инерции и останавливается
        let stop = h.x;
        for _ in 0..120 {
            s.step(1.0 / 60.0, (stop, 100.0), (0.0, 0.0));
        }
        assert_eq!(s.heaps.len(), 1);
        assert!(s.heaps[0].vx == 0.0 && s.heaps[0].x - stop < 80.0);
    }

    #[test]
    fn a_ball_pushed_off_the_edge_falls_back_into_particles() {
        let mut s = field(Kind::Snow);
        let before = total(&s);
        sweep(&mut s, 800.0, 220.0, 2.0);
        assert!(s.heaps.is_empty(), "шар остался на краю: {:?}", s.heaps.iter().map(|h| h.x).collect::<Vec<_>>());
        assert!(!s.particles.is_empty());
        assert!((total(&s) - before).abs() / before < 0.02, "масса не сохранилась: {} против {before}", total(&s));
    }

    #[test]
    fn moving_the_window_drops_its_balls() {
        let mut s = field(Kind::Snow);
        s.step(1.0 / 60.0, (400.0, 296.0), (200.0, 0.0));
        assert_eq!(s.heaps.len(), 1);
        s.refresh(&[WindowInfo { id: 5, rect: Rect { x: 140.0, ..WIN } }], None, 0.05);
        s.step(1.0 / 60.0, (-500.0, -500.0), (0.0, 0.0));
        assert!(s.heaps.is_empty());
    }

    #[test]
    fn a_smaller_ball_pushed_into_a_bigger_one_makes_a_snowman() {
        let mut s = bare(Kind::Snow);
        let big = PI * 10.0 * 10.0;
        let small = PI * 5.0 * 5.0;
        s.heaps.push(heap_at(400.0, big, Kind::Snow, 0.0));
        s.heaps.push(heap_at(412.0, small, Kind::Snow, -90.0));
        let before: f32 = s.heaps.iter().map(|h| h.mass()).sum();
        s.step(1.0 / 60.0, (-500.0, -500.0), (0.0, 0.0));
        assert_eq!(s.heaps.len(), 1, "шар не встал сверху");
        assert_eq!(s.heaps[0].tiers.len(), 2);
        assert!(s.heaps[0].tiers[0] > s.heaps[0].tiers[1], "большой должен быть внизу");
        assert!((s.heaps[0].mass() - before).abs() < 0.01);
        // третий, ещё меньше, достраивает снеговика; четвёртого уже не будет
        s.heaps.push(heap_at(412.0, PI * 3.0 * 3.0, Kind::Snow, -90.0));
        s.step(1.0 / 60.0, (-500.0, -500.0), (0.0, 0.0));
        assert_eq!(s.heaps.len(), 1);
        assert_eq!(s.heaps[0].tiers.len(), 3);
        s.heaps.push(heap_at(412.0, PI * 2.0 * 2.0, Kind::Snow, -90.0));
        s.step(1.0 / 60.0, (-500.0, -500.0), (0.0, 0.0));
        assert_eq!(s.heaps[0].tiers.len(), 3, "в снеговике больше трёх шаров");
    }

    #[test]
    fn similar_balls_merge_and_a_big_one_is_not_stacked_on_a_small_one() {
        let mut s = bare(Kind::Snow);
        s.heaps.push(heap_at(400.0, PI * 8.0 * 8.0, Kind::Snow, 0.0));
        s.heaps.push(heap_at(414.0, PI * 8.0 * 8.0, Kind::Snow, -90.0));
        s.step(1.0 / 60.0, (-500.0, -500.0), (0.0, 0.0));
        assert_eq!(s.heaps.len(), 1);
        assert_eq!(s.heaps[0].tiers.len(), 1);
        assert!((s.heaps[0].mass() - 2.0 * PI * 64.0).abs() < 0.1);
        // крупный, врезавшийся в мелкий, поглощает его, если вместе они помещаются в предельный шар
        let mut t = bare(Kind::Snow);
        t.heaps.push(heap_at(400.0, PI * 4.0 * 4.0, Kind::Snow, 0.0));
        t.heaps.push(heap_at(414.0, PI * 9.0 * 9.0, Kind::Snow, -90.0));
        t.step(1.0 / 60.0, (-500.0, -500.0), (0.0, 0.0));
        assert_eq!(t.heaps.len(), 1);
        assert!((t.heaps[0].mass() - PI * (16.0 + 81.0)).abs() < 0.1);
        // а если вместе они больше предельного, мелкий не поглощается и не взбирается — упирается
        let mut u = bare(Kind::Snow);
        u.heaps.push(heap_at(400.0, PI * 6.0 * 6.0, Kind::Snow, 0.0));
        u.heaps.push(heap_at(414.0, PI * 12.0 * 12.0, Kind::Snow, -90.0));
        u.step(1.0 / 60.0, (-500.0, -500.0), (0.0, 0.0));
        assert_eq!(u.heaps.len(), 2);
        assert_eq!(u.heaps[0].tiers.len() + u.heaps[1].tiers.len(), 2);
        assert!((u.heaps[0].x - u.heaps[1].x).abs() >= 18.0, "шары накладываются друг на друга");
    }

    #[test]
    fn too_many_snowmen_or_balls_make_the_oldest_fall_apart() {
        // восемь снеговиков вдали друг от друга: остаются пять самых новых
        let mut s = bare(Kind::Snow);
        for i in 0..8 {
            let mut h = heap_at(160.0 + i as f32 * 80.0, PI * 100.0, Kind::Snow, 0.0);
            h.tiers = vec![PI * 100.0, PI * 49.0, PI * 25.0];
            h.age = (8 - i) as f32; // самый левый — самый старый
            s.heaps.push(h);
        }
        let before: f32 = s.heaps.iter().map(|h| h.mass()).sum();
        s.step(1.0 / 60.0, (-500.0, -500.0), (0.0, 0.0));
        assert_eq!(s.heaps.iter().filter(|h| h.tiers.len() >= 2).count(), MAX_SNOWMEN);
        assert!(s.heaps.iter().all(|h| h.x > 160.0 + 2.5 * 80.0), "остались не самые новые: {:?}", s.heaps.iter().map(|h| h.x).collect::<Vec<_>>());
        // рассыпавшиеся стали частицами, масса цела
        assert!(!s.particles.is_empty());
        let after: f32 = s.heaps.iter().map(|h| h.mass()).sum::<f32>() + debris_mass(&s);
        assert!((after - before).abs() < before * 0.001 + 1.0, "{after} против {before}");

        // и просто шары: больше предела не бывает
        let mut t = bare(Kind::Snow);
        for i in 0..20 {
            let mut h = heap_at(130.0 + i as f32 * 35.0, PI * 30.0, Kind::Snow, 0.0);
            h.age = (20 - i) as f32;
            t.heaps.push(h);
        }
        t.step(1.0 / 60.0, (-500.0, -500.0), (0.0, 0.0));
        assert_eq!(t.heaps.len(), MAX_HEAPS);
        // новый снеговик (после постановки шара сверху) считается свежим и не рассыпается первым
        let mut u = bare(Kind::Snow);
        u.heaps.push(heap_at(400.0, PI * 100.0, Kind::Snow, 0.0));
        u.heaps[0].age = 50.0;
        u.heaps.push(heap_at(412.0, PI * 25.0, Kind::Snow, -90.0));
        u.step(1.0 / 60.0, (-500.0, -500.0), (0.0, 0.0));
        assert_eq!(u.heaps[0].tiers.len(), 2);
        assert!(u.heaps[0].age < 1.0, "возраст не обнулился: {}", u.heaps[0].age);
    }

    #[test]
    fn a_crowd_of_snowmen_never_hangs_the_collision_loop() {
        // Десять и больше снеговиков вплотную друг к другу: прежний цикл упирался и
        // отодвигал шары по кругу бесконечно, и приложение зависало.
        for count in [3, 10, 24] {
            let mut s = bare(Kind::Snow);
            for i in 0..count {
                let mut h = heap_at(300.0 + i as f32 * 3.0, PI * 100.0, Kind::Snow, if i % 2 == 0 { 120.0 } else { -120.0 });
                h.tiers = vec![PI * 100.0, PI * 49.0, PI * 25.0];
                s.heaps.push(h);
            }
            let before: f32 = s.heaps.iter().map(|h| h.mass()).sum();
            for frame in 0..180 {
                // курсор ходит туда-сюда среди них и толкает
                let x = 300.0 + (frame as f32 * 5.0) % 200.0;
                s.step(1.0 / 60.0, (x, 296.0), (150.0, 0.0));
            }
            // выжившие шары не налезают друг на друга
            let mut live: Vec<&Heap> = s.heaps.iter().collect();
            live.sort_by(|a, b| a.x.total_cmp(&b.x));
            for w in live.windows(2) {
                assert!(w[1].x - w[0].x >= w[0].half_width() + w[1].half_width() - 0.5, "{count}: шары наложились");
            }
            // масса цела: то, что не стоит, рассыпалось в частицы (и немного осталось в счётчике срыва)
            let after: f32 = s.heaps.iter().map(|h| h.mass()).sum::<f32>() + debris_mass(&s) + pile_mass(&s);
            assert!((after - before).abs() < before * 0.01 + 5.0, "{count}: масса {after} против {before}");
        }
    }

    /// Случайные расстановки вплотную стоящих шаров и куч: шаг обязан кончаться, шары —
    /// не налезать друг на друга, масса — сохраняться. Прежний цикл столкновений
    /// зависал уже на пяти шарах.
    #[test]
    fn random_crowds_always_terminate_and_stay_apart() {
        let mut seed: u64 = 12345;
        let mut rnd = move |lo: f32, hi: f32| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            lo + (hi - lo) * ((seed >> 40) as f32 / (1u64 << 24) as f32)
        };
        for round in 0..1500 {
            let kind = [Kind::Snow, Kind::Sand, Kind::Leaves][round % 3];
            let mut s = bare(kind);
            for _ in 0..(3 + round % 12) {
                let mut h = heap_at(rnd(250.0, 420.0), PI * rnd(20.0, 150.0), kind, rnd(-200.0, 200.0));
                if kind == Kind::Snow && rnd(0.0, 1.0) < 0.6 {
                    h.tiers = vec![PI * rnd(60.0, 150.0), PI * rnd(20.0, 55.0), PI * rnd(8.0, 20.0)];
                }
                s.heaps.push(h);
            }
            let before: f32 = s.heaps.iter().map(|h| h.mass()).sum();
            s.step(1.0 / 60.0, (rnd(250.0, 420.0), 296.0), (rnd(-200.0, 200.0), 0.0));
            let mut live: Vec<&Heap> = s.heaps.iter().collect();
            live.sort_by(|a, b| a.x.total_cmp(&b.x));
            for w in live.windows(2) {
                assert!(w[1].x - w[0].x >= w[0].half_width() + w[1].half_width() - 0.5, "раунд {round}: шары наложились");
            }
            let after: f32 = s.heaps.iter().map(|h| h.mass()).sum::<f32>() + debris_mass(&s);
            assert!((after - before).abs() < before * 0.001 + 1.0, "раунд {round}: масса {after} против {before}");
        }
    }

    #[test]
    fn sand_and_leaf_heaps_merge_and_never_stack() {
        for kind in [Kind::Sand, Kind::Leaves, Kind::Dust] {
            let mut s = bare(kind);
            s.heaps.push(heap_at(400.0, PI * 4.0 * 4.0, kind, 0.0));
            s.heaps.push(heap_at(412.0, PI * 8.0 * 8.0, kind, -90.0));
            let before: f32 = s.heaps.iter().map(|h| h.mass()).sum();
            s.step(1.0 / 60.0, (-500.0, -500.0), (0.0, 0.0));
            assert_eq!(s.heaps.len(), 1, "{kind:?}");
            assert_eq!(s.heaps[0].tiers.len(), 1, "{kind:?}: холмик не должен вставать на холмик");
            assert!((s.heaps[0].mass() - before).abs() < 0.01);
        }
    }

    #[test]
    fn water_is_not_gathered_and_the_switch_restores_scattering() {
        let mut rain = field(Kind::Rain);
        rain.step(1.0 / 60.0, (400.0, 296.0), (200.0, 0.0));
        assert!(rain.heaps.is_empty());
        // выключено — курсор разбрасывает, как раньше
        let mut off = field(Kind::Snow);
        off.gather = false;
        off.step(1.0 / 60.0, (400.0, 297.0), (300.0, 0.0));
        assert!(off.heaps.is_empty() && off.particles.iter().any(|p| p.debris));
        // выключили при готовых шарах — они рассыпались
        let mut on = field(Kind::Snow);
        on.heaps.push(heap_at(400.0, PI * 100.0, Kind::Snow, 0.0));
        on.gather = false;
        on.step(1.0 / 60.0, (-500.0, -500.0), (0.0, 0.0));
        assert!(on.heaps.is_empty() && !on.particles.is_empty());
    }

    #[test]
    fn changing_the_material_or_shaking_drops_the_heaps() {
        let mut s = field(Kind::Snow);
        s.heaps.push(heap_at(400.0, PI * 100.0, Kind::Snow, 0.0));
        s.set_kind(Kind::Sand);
        assert!(s.heaps.is_empty() && s.particles.iter().all(|p| !p.land));
        let mut t = field(Kind::Snow);
        t.heaps.push(heap_at(400.0, PI * 100.0, Kind::Snow, 0.0));
        t.dump_all();
        assert!(t.heaps.is_empty());
    }

    #[test]
    fn shapes_stay_within_budget_and_stand_on_the_surface() {
        for kind in [Kind::Snow, Kind::Sand, Kind::Leaves, Kind::Dust] {
            let mut s = field(kind);
            for i in 0..MAX_HEAPS {
                let mut h = heap_at(150.0 + i as f32 * 30.0, PI * 40.0, kind, 0.0);
                if kind == Kind::Snow && i % 2 == 0 {
                    h.tiers = vec![PI * 100.0, PI * 49.0, PI * 25.0];
                }
                s.heaps.push(h);
            }
            let mut out = Vec::new();
            looks(&s, &mut out);
            assert!(!out.is_empty() && out.len() <= HEAP_MAX, "{kind:?}: {} фигур", out.len());
            // основания стоят на слое (кромка 300 минус слой 6 pt), ниже окна ничего нет
            assert!(out.iter().all(|f| f.center[1] < 300.0 && f.center[0].is_finite()), "{kind:?}");
        }
    }
}
