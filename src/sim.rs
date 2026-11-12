//! Физика снега. Не зависит ни от ОС, ни от отрисовки.
//!
//! Координаты: логические точки, начало в левом верхнем углу экрана, y вниз.

use std::collections::HashMap;

/// Ширина столбика сугроба.
pub const COL_W: f32 = 2.0;
/// Предел высоты сугроба.
pub const MAX_PILE: f32 = 18.0;
/// Максимальный перепад между соседними столбиками (угол осыпания).
pub const SLOPE: f32 = 1.4;
/// Радиус «лопаты» курсора.
pub const CURSOR_R: f32 = 12.0;
/// Потолок числа частиц.
pub const MAX_PARTICLES: usize = 6000;

/// Идентификатор поверхности «низ экрана».
const GROUND: i64 = 0;
/// Идентификатор поверхности «Dock / панель задач».
const DOCK: i64 = -1;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }
    pub fn min_x(&self) -> f32 {
        self.x
    }
    pub fn max_x(&self) -> f32 {
        self.x + self.w
    }
    pub fn min_y(&self) -> f32 {
        self.y
    }
    pub fn max_y(&self) -> f32 {
        self.y + self.h
    }
}

/// Окно на экране. Список окон передаётся от переднего к заднему.
#[derive(Clone, Copy, Debug)]
pub struct WindowInfo {
    pub id: i64,
    pub rect: Rect,
}

pub struct Particle {
    pub x: f32,
    pub y: f32,
    pub vx: f32,
    pub vy: f32,
    pub r: f32,
    pub mass: f32,
    /// Может ложиться на поверхности.
    pub land: bool,
    /// Комок, сбитый с сугроба: тяжёлый, падает быстро.
    pub debris: bool,
    /// 0 — дальний … 2 — ближний.
    pub layer: u8,
    pub phase: f32,
}

/// Кромка, на которой лежит снег: верх окна, Dock или низ экрана.
pub struct Surface {
    pub frame: Rect,
    pub x0: f32,
    pub y: f32,
    /// Высоты столбиков шириной `COL_W`.
    pub h: Vec<f32>,
    /// Виден ли столбик (не перекрыт ли окном спереди).
    pub vis: Vec<bool>,
    seen: bool,
    spill_l: f32,
    spill_r: f32,
}

impl Surface {
    fn new(frame: Rect, x0: f32, y: f32, n: usize) -> Self {
        Self {
            frame,
            x0,
            y,
            h: vec![0.0; n],
            vis: vec![true; n],
            seen: true,
            spill_l: 0.0,
            spill_r: 0.0,
        }
    }
}

/// Маленький генератор случайных чисел (xorshift), чтобы не тянуть зависимость.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed | 1)
    }
    fn unit(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }
    pub fn range(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.unit()
    }
}

pub struct Sim {
    pub w: f32,
    pub h: f32,
    pub particles: Vec<Particle>,
    pub surfaces: HashMap<i64, Surface>,
    /// Снежинок в секунду на 1000 точек ширины экрана.
    pub rate: f32,
    /// Окна непроницаемы: снег не идёт поверх их содержимого.
    pub solid: bool,
    solids: Vec<Rect>,
    t: f32,
    spawn_acc: f32,
    rng: Rng,
}

fn debris(ps: &mut Vec<Particle>, x: f32, y: f32, vx: f32, vy: f32, mass: f32) {
    if ps.len() >= MAX_PARTICLES || mass <= 0.05 {
        return;
    }
    let r = (mass / 2.0).sqrt().clamp(1.0, 3.5);
    ps.push(Particle { x, y, vx, vy, r, mass, land: true, debris: true, layer: 2, phase: 0.0 });
}

/// Превращает весь снег поверхности в падающие комки со скоростью `vel`.
fn dump(s: &mut Surface, vel: (f32, f32), ps: &mut Vec<Particle>, rng: &mut Rng) {
    for i in 0..s.h.len() {
        if s.h[i] <= 0.3 {
            continue;
        }
        let mass = s.h[i] * COL_W;
        let k = ((mass / 8.0) as usize).max(1);
        let x = s.x0 + (i as f32 + 0.5) * COL_W;
        for _ in 0..k {
            debris(
                ps,
                x + rng.range(-1.0, 1.0),
                s.y - rng.range(0.0, s.h[i]),
                vel.0 * 0.4 + rng.range(-25.0, 25.0),
                vel.1 * 0.4 + rng.range(-15.0, 25.0),
                mass / k as f32,
            );
        }
        s.h[i] = 0.0;
    }
}

fn deposit(s: &mut Surface, c: usize, mass: f32) {
    let dh = mass / COL_W;
    let n = s.h.len();
    let l = if c > 0 && s.vis[c - 1] { c - 1 } else { c };
    let r = if c + 1 < n && s.vis[c + 1] { c + 1 } else { c };
    s.h[c] += dh * 0.5;
    s.h[l] += dh * 0.25;
    s.h[r] += dh * 0.25;
}

impl Sim {
    pub fn new(seed: u64) -> Self {
        Self {
            w: 0.0,
            h: 0.0,
            particles: Vec::with_capacity(MAX_PARTICLES),
            surfaces: HashMap::new(),
            rate: 40.0,
            solid: false,
            solids: Vec::new(),
            t: 0.0,
            spawn_acc: 0.0,
            rng: Rng::new(seed),
        }
    }

    /// Сбрасывает все сугробы — снег падает вниз.
    pub fn dump_all(&mut self) {
        for s in self.surfaces.values_mut() {
            dump(s, (0.0, 0.0), &mut self.particles, &mut self.rng);
        }
    }

    // ---- окна --------------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    fn upsert(
        &mut self,
        id: i64,
        frame: Rect,
        xa: f32,
        xb: f32,
        y: f32,
        occ: &[(Rect, Option<i64>)],
        poll_dt: f32,
    ) -> bool {
        let n = ((xb - xa) / COL_W).max(0.0) as usize;
        if n <= 4 {
            return false;
        }
        let mut surf = match self.surfaces.remove(&id) {
            Some(mut old) => {
                let dx = frame.x - old.frame.x;
                let dy = frame.y - old.frame.y;
                if old.h.len() != n {
                    // окно растянули: снег остаётся на своём месте на экране, лишнее падает
                    let shift = ((xa - old.x0) / COL_W).round() as isize;
                    let mut h = vec![0.0; n];
                    for i in 0..old.h.len() {
                        let j = i as isize - shift;
                        if old.h[i] > 0.0 && j >= 0 && (j as usize) < n {
                            h[j as usize] = old.h[i];
                            old.h[i] = 0.0;
                        }
                    }
                    dump(&mut old, (0.0, 0.0), &mut self.particles, &mut self.rng);
                    old.h = h;
                    old.vis = vec![true; n];
                } else if dx.abs() > 0.5 || dy.abs() > 0.5 {
                    // окно подвинули — снег стряхивается
                    let v = ((dx / poll_dt).clamp(-800.0, 800.0), (dy / poll_dt).clamp(-800.0, 800.0));
                    dump(&mut old, v, &mut self.particles, &mut self.rng);
                }
                old.frame = frame;
                old.x0 = xa;
                old.y = y;
                old
            }
            None => Surface::new(frame, xa, y, n),
        };
        surf.seen = true;
        for i in 0..n {
            let x = xa + (i as f32 + 0.5) * COL_W;
            let cover = occ
                .iter()
                .find(|(r, _)| r.min_x() <= x && x < r.max_x() && r.min_y() < y - 1.0 && r.max_y() > y - 1.0);
            surf.vis[i] = cover.is_none() && x >= 0.0 && x < self.w;
            // окно спереди наехало на сугроб — снег поднимается на его кромку
            if surf.h[i] > 0.0
                && let Some((_, Some(cid))) = cover
                && let Some(front) = self.surfaces.get_mut(cid)
            {
                let c = (((x - front.x0) / COL_W) as isize).clamp(0, front.h.len() as isize - 1) as usize;
                front.h[c] += surf.h[i];
                surf.h[i] = 0.0;
            }
        }
        self.surfaces.insert(id, surf);
        true
    }

    /// Сверяет поверхности со списком окон (от переднего к заднему).
    ///
    /// `dock` — кромка Dock или панели задач, если она известна и расположена снизу.
    pub fn refresh(&mut self, windows: &[WindowInfo], dock: Option<Rect>, poll_dt: f32) {
        for s in self.surfaces.values_mut() {
            s.seen = false;
        }
        let mut occ: Vec<(Rect, Option<i64>)> = Vec::with_capacity(windows.len());
        for w in windows {
            let b = w.rect;
            let mut sid = None;
            if b.w >= 160.0
                && b.h >= 80.0
                && b.min_y() > 5.0
                && b.min_y() < self.h - 20.0
                && b.max_x() > 0.0
                && b.min_x() < self.w
                // отступ 8 pt — скруглённые углы окна
                && self.upsert(w.id, b, b.min_x() + 8.0, b.max_x() - 8.0, b.min_y(), &occ, poll_dt)
            {
                sid = Some(w.id);
            }
            if b.w >= 50.0 && b.h >= 50.0 {
                occ.push((b, sid));
            }
        }
        self.solids.clear();
        self.solids.extend(occ.iter().map(|(r, _)| *r));

        let mut ground_occ = Vec::new();
        if let Some(d) = dock {
            self.upsert(DOCK, d, d.min_x(), d.max_x(), d.min_y(), &[], poll_dt);
            ground_occ.push((Rect::new(d.min_x() - 12.0, 0.0, d.w + 24.0, self.h + 10.0), None));
        }
        let ground = Rect::new(0.0, self.h, self.w, 1.0);
        self.upsert(GROUND, ground, 0.0, self.w, self.h, &ground_occ, poll_dt);

        // окно закрыли — снег падает
        let gone: Vec<i64> = self.surfaces.iter().filter(|(_, s)| !s.seen).map(|(id, _)| *id).collect();
        for id in gone {
            if let Some(mut s) = self.surfaces.remove(&id) {
                dump(&mut s, (0.0, 0.0), &mut self.particles, &mut self.rng);
            }
        }
    }

    // ---- шаг физики --------------------------------------------------------

    /// `c` — положение курсора, `cv` — его скорость.
    pub fn step(&mut self, dt: f32, c: (f32, f32), cv: (f32, f32)) {
        let Sim { w, h, particles, surfaces, rate, solid, solids, t, spawn_acc, rng } = self;
        let (w, h, solid) = (*w, *h, *solid);
        *t += dt;
        let t = *t;

        // новые снежинки
        *spawn_acc += *rate * dt * w / 1000.0;
        while *spawn_acc >= 1.0 {
            *spawn_acc -= 1.0;
            if particles.len() >= MAX_PARTICLES {
                continue;
            }
            let u = rng.range(0.0, 1.0);
            let r = 1.0 + 2.2 * u * u;
            let layer = if r < 1.5 { 0 } else if r < 2.2 { 1 } else { 2 };
            particles.push(Particle {
                x: rng.range(-60.0, w + 60.0),
                y: -8.0,
                vx: 0.0,
                vy: 20.0 + r * 15.0,
                r,
                mass: r * r * 2.0,
                land: layer > 0 && rng.range(0.0, 1.0) < 0.7,
                debris: false,
                layer,
                phase: rng.range(0.0, 6.28),
            });
        }

        let wind = 25.0 * (t * 0.13).sin() + 12.0 * (t * 0.51 + 1.0).sin();

        // курсор сгребает сугробы
        for s in surfaces.values_mut() {
            if !(c.1 > s.y - MAX_PILE - CURSOR_R - 4.0 && c.1 < s.y + CURSOR_R) || s.h.is_empty() {
                continue;
            }
            let a = (((c.0 - CURSOR_R - s.x0) / COL_W).floor() as isize).max(0);
            let b = (((c.0 + CURSOR_R - s.x0) / COL_W).floor() as isize).min(s.h.len() as isize - 1);
            for i in a..=b {
                let i = i as usize;
                if s.h[i] <= 0.3 {
                    continue;
                }
                let x = s.x0 + (i as f32 + 0.5) * COL_W;
                let dx = x - c.0;
                if dx.abs() >= CURSOR_R {
                    continue;
                }
                let sh = (CURSOR_R * CURSOR_R - dx * dx).sqrt();
                if c.1 - sh >= s.y {
                    continue;
                }
                let allowed = (s.y - (c.1 + sh)).max(0.0);
                if s.h[i] <= allowed + 0.3 {
                    continue;
                }
                let ex = s.h[i] - allowed;
                s.h[i] = allowed;
                let mass = ex * COL_W;
                let k = ((mass / 5.0) as usize).clamp(1, 3);
                for _ in 0..k {
                    debris(
                        particles,
                        x,
                        s.y - allowed - rng.range(0.0, ex),
                        cv.0 * 0.6 + dx / CURSOR_R * 70.0 + rng.range(-50.0, 50.0),
                        -cv.1.abs() * 0.3 - rng.range(30.0, 130.0),
                        mass / k as f32,
                    );
                }
            }
        }

        // частицы
        let mut i = 0;
        while i < particles.len() {
            let p = &mut particles[i];
            let (prev_x, prev_y) = (p.x, p.y);
            let term = if p.debris { 460.0 } else { 28.0 + p.r * 20.0 };
            let k = if p.debris { 2.2 } else { 3.0 };
            let tx = if p.debris {
                wind * 0.3
            } else {
                wind * (0.5 + p.r * 0.25) + 14.0 * (t * 1.7 + p.phase).sin()
            };

            // вихрь от курсора
            let (dx, dy) = (p.x - c.0, p.y - c.1);
            let d2 = dx * dx + dy * dy;
            if d2 < 4900.0 {
                let d = d2.sqrt().max(1.0);
                let f = 1.0 - d / 70.0;
                let g = (f * 8.0 * dt).min(1.0);
                p.vx += (cv.0 * 0.6 - p.vx) * g + dx / d * f * 260.0 * dt;
                p.vy += (cv.1 * 0.6 - p.vy) * g + dy / d * f * 260.0 * dt;
            }
            p.vx += (tx - p.vx) * k * dt;
            p.vy += (term - p.vy) * k * dt;
            p.x += p.vx * dt;
            p.y += p.vy * dt;

            let mut dead = p.y > h + 12.0 || p.x < -80.0 || p.x > w + 80.0;
            if !dead && (p.land || solid) && p.vy > 0.0 {
                for s in surfaces.values_mut() {
                    if !(p.y >= s.y - MAX_PILE - 30.0 && prev_y <= s.y + 2.0) {
                        continue;
                    }
                    let col = ((p.x - s.x0) / COL_W).floor();
                    if col < 0.0 || col as usize >= s.h.len() || !s.vis[col as usize] {
                        continue;
                    }
                    let col = col as usize;
                    let top = s.y - s.h[col];
                    if !(prev_y <= top + 1.5 && p.y >= top) {
                        continue;
                    }
                    let fill = s.h[col] / MAX_PILE;
                    if p.debris || rng.range(0.0, 1.0) > fill * fill {
                        deposit(s, col, p.mass);
                        dead = true;
                    } else if solid && s.y < h - 1.0 {
                        dead = true; // сугроб полон — снежинка тает, но в окно не залетает
                    }
                    break;
                }
            }
            if !dead && solid {
                for r in solids.iter() {
                    if !(p.x > r.min_x() && p.x < r.max_x() && p.y > r.min_y() && p.y < r.max_y()) {
                        continue;
                    }
                    if prev_y > r.min_y() && prev_x <= r.min_x() {
                        // удар о левый край окна
                        p.x = r.min_x() - 0.5;
                        p.vx = -p.vx.abs() * 0.3;
                    } else if prev_y > r.min_y() && prev_x >= r.max_x() {
                        // удар о правый край
                        p.x = r.max_x() + 0.5;
                        p.vx = p.vx.abs() * 0.3;
                    } else {
                        dead = true; // упала на кромку без сугроба или окно накрыло её
                    }
                    break;
                }
            }
            if dead {
                particles.swap_remove(i);
            } else {
                i += 1;
            }
        }

        // осыпание сугробов и срыв снега с краёв
        for s in surfaces.values_mut() {
            let n = s.h.len();
            if n < 2 {
                continue;
            }
            for pass in 0..2 {
                for j in 0..n - 1 {
                    let a = if pass == 0 { j } else { n - 2 - j };
                    if !(s.vis[a] && s.vis[a + 1]) {
                        continue;
                    }
                    let d = s.h[a] - s.h[a + 1];
                    if d > SLOPE {
                        let m = (d - SLOPE) * 0.25;
                        s.h[a] -= m;
                        s.h[a + 1] += m;
                    } else if d < -SLOPE {
                        let m = (-d - SLOPE) * 0.25;
                        s.h[a] += m;
                        s.h[a + 1] -= m;
                    }
                }
            }
            if s.y >= h - 1.0 {
                continue; // у земли краёв нет
            }
            if s.vis[0] && s.h[0] > 2.0 {
                let m = (s.h[0] - 2.0) * 0.2;
                s.h[0] -= m;
                s.spill_l += m * COL_W;
                while s.spill_l >= 4.0 {
                    s.spill_l -= 4.0;
                    let y = s.y - rng.range(0.0, s.h[0]);
                    debris(particles, s.frame.min_x() - 1.0, y, rng.range(-30.0, -6.0), rng.range(-10.0, 10.0), 4.0);
                }
            }
            if s.vis[n - 1] && s.h[n - 1] > 2.0 {
                let m = (s.h[n - 1] - 2.0) * 0.2;
                s.h[n - 1] -= m;
                s.spill_r += m * COL_W;
                while s.spill_r >= 4.0 {
                    s.spill_r -= 4.0;
                    let y = s.y - rng.range(0.0, s.h[n - 1]);
                    debris(particles, s.frame.max_x() + 1.0, y, rng.range(6.0, 30.0), rng.range(-10.0, 10.0), 4.0);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sim() -> Sim {
        let mut s = Sim::new(42);
        s.w = 1000.0;
        s.h = 800.0;
        s
    }

    fn total_pile(s: &Sim) -> f32 {
        s.surfaces.values().flat_map(|s| s.h.iter()).sum::<f32>() * COL_W
    }

    #[test]
    fn snow_accumulates_on_window_and_ground() {
        let mut s = sim();
        let win = [WindowInfo { id: 7, rect: Rect::new(200.0, 300.0, 400.0, 300.0) }];
        for _ in 0..(60 * 40) {
            s.refresh(&win, None, 0.05);
            s.step(1.0 / 60.0, (-500.0, -500.0), (0.0, 0.0));
        }
        let on_window: f32 = s.surfaces[&7].h.iter().sum();
        let on_ground: f32 = s.surfaces[&GROUND].h.iter().sum();
        assert!(on_window > 1.0, "на окне нет снега: {on_window}");
        assert!(on_ground > 1.0, "на земле нет снега: {on_ground}");
        assert!(s.surfaces[&7].h.iter().all(|h| *h >= 0.0 && h.is_finite()));
    }

    #[test]
    fn moving_window_shakes_snow_off() {
        let mut s = sim();
        s.rate = 0.0;
        let a = [WindowInfo { id: 7, rect: Rect::new(200.0, 300.0, 400.0, 300.0) }];
        s.refresh(&a, None, 0.05);
        for h in s.surfaces.get_mut(&7).unwrap().h.iter_mut() {
            *h = 5.0;
        }
        let b = [WindowInfo { id: 7, rect: Rect::new(260.0, 300.0, 400.0, 300.0) }];
        s.refresh(&b, None, 0.05);
        assert_eq!(s.surfaces[&7].h.iter().sum::<f32>(), 0.0);
        assert!(!s.particles.is_empty());
    }

    #[test]
    fn covering_window_lifts_snow_onto_itself() {
        let mut s = sim();
        s.rate = 0.0;
        let back = WindowInfo { id: 1, rect: Rect::new(100.0, 400.0, 600.0, 300.0) };
        s.refresh(&[back], None, 0.05);
        for h in s.surfaces.get_mut(&1).unwrap().h.iter_mut() {
            *h = 4.0;
        }
        let before = total_pile(&s);
        // переднее окно накрывает середину кромки заднего
        let front = WindowInfo { id: 2, rect: Rect::new(300.0, 200.0, 200.0, 400.0) };
        s.refresh(&[front, back], None, 0.05);
        let lifted: f32 = s.surfaces[&2].h.iter().sum();
        assert!(lifted > 0.0, "снег не поднялся на переднее окно");
        assert!((total_pile(&s) - before).abs() < 0.01, "масса снега не сохранилась");
        assert!(s.surfaces[&1].vis.iter().any(|v| !v));
    }

    #[test]
    fn solid_windows_keep_particles_out() {
        let mut s = sim();
        s.solid = true;
        let win = [WindowInfo { id: 7, rect: Rect::new(200.0, 300.0, 400.0, 300.0) }];
        for _ in 0..(60 * 20) {
            s.refresh(&win, None, 0.05);
            s.step(1.0 / 60.0, (-500.0, -500.0), (0.0, 0.0));
            let r = win[0].rect;
            assert!(
                !s.particles
                    .iter()
                    .any(|p| p.x > r.min_x() && p.x < r.max_x() && p.y > r.min_y() + 1.0 && p.y < r.max_y()),
                "частица оказалась поверх содержимого окна"
            );
        }
    }

    #[test]
    fn cursor_carves_the_pile() {
        let mut s = sim();
        s.rate = 0.0;
        let win = [WindowInfo { id: 7, rect: Rect::new(200.0, 300.0, 400.0, 300.0) }];
        s.refresh(&win, None, 0.05);
        for h in s.surfaces.get_mut(&7).unwrap().h.iter_mut() {
            *h = 6.0;
        }
        s.step(1.0 / 60.0, (400.0, 298.0), (300.0, 0.0));
        let surf = &s.surfaces[&7];
        let col = ((400.0 - surf.x0) / COL_W) as usize;
        assert!(surf.h[col] < 1.0, "курсор не срезал сугроб: {}", surf.h[col]);
        assert!(s.particles.iter().any(|p| p.debris));
    }
}
