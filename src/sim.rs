//! Физика осадков. Не зависит ни от ОС, ни от отрисовки.
//!
//! Одна и та же физика для всех материалов; чем они отличаются, описано в
//! `materials.rs`. Координаты: логические точки, начало в левом верхнем углу
//! экрана, y вниз.

use std::collections::HashMap;

use crate::materials::{Gather, Kind};

pub mod heap;
pub mod plow;

/// Ширина столбика кучи.
pub const COL_W: f32 = 2.0;
/// Куда дует ветер.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindDir {
    /// Как обычно: порывы то в одну сторону, то в другую.
    Auto,
    /// Слева направо, с порывами.
    Right,
    /// Справа налево, с порывами.
    Left,
    /// Ветра нет.
    Calm,
}

/// Постоянный снос при заданном направлении, поверх которого идут порывы.
const STEADY_WIND: f32 = 40.0;

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
    pub kind: Kind,
    /// Номер цвета из палитры материала.
    pub tint: u8,
    /// Поворот и скорость вращения (для листьев).
    pub angle: f32,
    pub spin: f32,
    /// Сколько секунд осталось жить (брызги); у обычных частиц — бесконечность.
    pub life: f32,
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
    fn byte(&mut self) -> u8 {
        (self.unit() * 256.0) as u8
    }
}

pub struct Sim {
    pub w: f32,
    pub h: f32,
    pub particles: Vec<Particle>,
    pub surfaces: HashMap<i64, Surface>,
    /// Что сейчас падает и из чего состоят кучи.
    kind: Kind,
    /// Снежинок в секунду на 1000 точек ширины экрана (для других материалов
    /// умножается на `Material::rate`).
    pub rate: f32,
    /// Окна непроницаемы: снег не идёт поверх их содержимого.
    pub solid: bool,
    /// Во сколько раз меньше частиц рождается (режим энергосбережения облегчает осадки).
    pub rate_scale: f32,
    /// Куда дует ветер и во сколько раз он сильнее обычного.
    pub wind_dir: WindDir,
    pub wind_force: f32,
    /// Украшения на верхней кромке окон (деревья и т. п.).
    pub decor: bool,
    /// Курсор сгребает кучи в шары и холмики (иначе разбрасывает).
    pub gather: bool,
    /// Во сколько раз кучи на кромках ниже предела материала (`Material::max_pile`).
    pub pile_scale: f32,
    /// Шары, холмики и снеговики, которые собрал курсор.
    pub heaps: Vec<heap::Heap>,
    /// Уборщик на кромке панели задач или Dock.
    pub plow: plow::Plow,
    solids: Vec<Rect>,
    t: f32,
    spawn_acc: f32,
    rng: Rng,
}

/// Комок материала `kind`, сбитый с кучи.
#[allow(clippy::too_many_arguments)]
fn debris(ps: &mut Vec<Particle>, rng: &mut Rng, kind: Kind, land: bool, x: f32, y: f32, vx: f32, vy: f32, mass: f32) {
    if ps.len() >= MAX_PARTICLES || mass <= 0.05 {
        return;
    }
    let m = kind.material();
    let r = (mass / (2.0 * m.density)).sqrt().clamp(m.scale, m.scale * 3.5);
    ps.push(Particle {
        x,
        y,
        vx,
        vy,
        r,
        mass,
        land,
        debris: true,
        layer: 2,
        phase: rng.range(0.0, 6.28),
        kind,
        tint: rng.byte(),
        angle: rng.range(0.0, 6.28),
        spin: rng.range(-m.spin, m.spin),
        life: f32::INFINITY,
    });
}

/// Превращает всю кучу в падающие комки со скоростью `vel`.
fn dump(s: &mut Surface, vel: (f32, f32), ps: &mut Vec<Particle>, rng: &mut Rng, kind: Kind, land: bool) {
    let chunk = kind.material().chunk * 1.6;
    for i in 0..s.h.len() {
        if s.h[i] <= 0.3 {
            s.h[i] = 0.0;
            continue;
        }
        let mass = s.h[i] * COL_W;
        let k = ((mass / chunk) as usize).max(1);
        let x = s.x0 + (i as f32 + 0.5) * COL_W;
        for _ in 0..k {
            let (px, py) = (x + rng.range(-1.0, 1.0), s.y - rng.range(0.0, s.h[i]));
            let (vx, vy) = (vel.0 * 0.4 + rng.range(-25.0, 25.0), vel.1 * 0.4 + rng.range(-15.0, 25.0));
            debris(ps, rng, kind, land, px, py, vx, vy, mass / k as f32);
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
            kind: Kind::Snow,
            rate: 40.0,
            solid: false,
            rate_scale: 1.0,
            wind_dir: WindDir::Auto,
            wind_force: 1.0,
            decor: true,
            gather: true,
            pile_scale: 1.0,
            heaps: Vec::new(),
            plow: plow::Plow::default(),
            solids: Vec::new(),
            t: 0.0,
            spawn_acc: 0.0,
            rng: Rng::new(seed),
        }
    }

    pub fn kind(&self) -> Kind {
        self.kind
    }

    /// Сколько секунд идёт симуляция: по нему движется то, что рисуется «живым» (дым).
    pub fn time(&self) -> f32 {
        self.t
    }

    /// Меняет материал. Старые кучи осыпаются и улетают за нижний край экрана,
    /// старые частицы долетают, но уже не ложатся — новые кучи растут с нуля.
    pub fn set_kind(&mut self, kind: Kind) {
        if kind == self.kind {
            return;
        }
        self.drop_all_heaps(false);
        for p in self.particles.iter_mut() {
            p.land = false;
        }
        for s in self.surfaces.values_mut() {
            dump(s, (0.0, 0.0), &mut self.particles, &mut self.rng, self.kind, false);
        }
        self.kind = kind;
    }

    /// Меняет предел высоты куч. Если он стал меньше, накопленные кучи осыпаются
    /// (масса остаётся частицами и ложится заново уже в новый предел): иначе кучи
    /// выше предела так и стояли бы, не принимая ничего нового.
    pub fn set_pile_scale(&mut self, scale: f32) {
        if scale < self.pile_scale {
            for s in self.surfaces.values_mut() {
                dump(s, (0.0, 0.0), &mut self.particles, &mut self.rng, self.kind, true);
            }
        }
        self.pile_scale = scale;
    }

    /// Сбрасывает все кучи — материал падает вниз.
    pub fn dump_all(&mut self) {
        self.drop_all_heaps(true);
        for s in self.surfaces.values_mut() {
            dump(s, (0.0, 0.0), &mut self.particles, &mut self.rng, self.kind, true);
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
                    dump(&mut old, (0.0, 0.0), &mut self.particles, &mut self.rng, self.kind, true);
                    old.h = h;
                    old.vis = vec![true; n];
                } else if dx.abs() > 0.5 || dy.abs() > 0.5 {
                    // окно подвинули — снег стряхивается
                    let v = ((dx / poll_dt).clamp(-800.0, 800.0), (dy / poll_dt).clamp(-800.0, 800.0));
                    dump(&mut old, v, &mut self.particles, &mut self.rng, self.kind, true);
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

    /// Дымка на весь экран: цвет и текущая прозрачность. Прозрачность медленно
    /// «дышит». У материалов без дымки прозрачность нулевая.
    pub fn haze(&self) -> [f32; 4] {
        let [r, g, b, a] = self.kind.material().haze;
        [r, g, b, a * (0.75 + 0.25 * (self.t * 0.45).sin())]
    }

    /// Рамки всех окон на экране в координатах симуляции. Небо вырезает по ним
    /// свою картинку, чтобы оно было «за» окнами.
    pub fn window_rects(&self) -> &[Rect] {
        &self.solids
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
                dump(&mut s, (0.0, 0.0), &mut self.particles, &mut self.rng, self.kind, true);
            }
        }
    }

    // ---- шаг физики --------------------------------------------------------

    /// `c` — положение курсора, `cv` — его скорость.
    pub fn step(&mut self, dt: f32, c: (f32, f32), cv: (f32, f32)) {
        // курсор собирает кучи в шары и холмики; у воды такого нет
        let gather_on = self.gather && self.kind.material().gather != Gather::None;
        if gather_on {
            self.heaps_step(dt, c, cv);
        } else if !self.heaps.is_empty() {
            self.drop_all_heaps(true);
        }
        self.plow_step(dt);
        let Sim { w, h, particles, surfaces, kind, rate, rate_scale, solid, wind_dir, wind_force, decor: _, gather: _, pile_scale, heaps: _, plow: _, solids, t, spawn_acc, rng } = self;
        let (w, h, solid, kind, wind_dir, force) = (*w, *h, *solid, *kind, *wind_dir, *wind_force);
        // материал, который падает сейчас и из которого состоят кучи
        let mat = kind.material();
        let max_pile = mat.max_pile * *pile_scale;
        *t += dt;
        let t = *t;

        // порывы: сумма двух медленных колебаний
        let gust = 25.0 * (t * 0.13).sin() + 12.0 * (t * 0.51 + 1.0).sin();
        // общий ветер для всех материалов; у материала поверх него свой постоянный снос
        let wind = force
            * match wind_dir {
                WindDir::Auto => gust,
                WindDir::Right => STEADY_WIND + 0.5 * gust,
                WindDir::Left => -STEADY_WIND + 0.5 * gust,
                WindDir::Calm => 0.0,
            };
        // постоянный снос материала (у бури) идёт туда же, куда выбран ветер
        let bias_of = |m: &crate::materials::Material| {
            force
                * match wind_dir {
                    WindDir::Auto => m.wind_bias,
                    WindDir::Right => m.wind_bias.abs(),
                    WindDir::Left => -m.wind_bias.abs(),
                    WindDir::Calm => 0.0,
                }
        };

        // новые частицы
        *spawn_acc += *rate * *rate_scale * mat.rate * dt * w / 1000.0;
        while *spawn_acc >= 1.0 {
            *spawn_acc -= 1.0;
            if particles.len() >= MAX_PARTICLES {
                continue;
            }
            let u = rng.range(0.0, 1.0);
            let size = 1.0 + 2.2 * u * u; // размер в «снежинках»: от 1 до 3.2
            let r = size * mat.scale;
            let layer = if size < 1.5 { 0 } else if size < 2.2 { 1 } else { 2 };
            // при сильном боковом ветре часть частиц рождается у наветренного края
            let bias = bias_of(mat);
            let side = mat.side_spawn > 0.0 && bias != 0.0 && rng.range(0.0, 1.0) < mat.side_spawn;
            let (sx, sy) = if side {
                (if bias > 0.0 { -60.0 } else { w + 60.0 }, rng.range(0.0, h))
            } else {
                (rng.range(-60.0, w + 60.0), -8.0)
            };
            particles.push(Particle {
                x: sx,
                y: sy,
                vx: 0.0,
                vy: (mat.fall_base + mat.fall_size * size) * 0.7,
                r,
                mass: r * r * 2.0 * mat.density,
                land: layer > 0 && rng.range(0.0, 1.0) < mat.land_share,
                debris: false,
                layer,
                phase: rng.range(0.0, 6.28),
                kind,
                tint: rng.byte(),
                angle: rng.range(0.0, 6.28),
                spin: rng.range(-mat.spin, mat.spin),
                life: f32::INFINITY,
            });
        }

        // курсор сгребает кучи
        for s in surfaces.values_mut() {
            if gather_on || !(c.1 > s.y - max_pile - CURSOR_R - 4.0 && c.1 < s.y + CURSOR_R) || s.h.is_empty() {
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
                let k = ((mass / mat.chunk) as usize).clamp(1, mat.max_chunks);
                for _ in 0..k {
                    let py = s.y - allowed - rng.range(0.0, ex);
                    let vx = cv.0 * 0.6 + dx / CURSOR_R * 70.0 + rng.range(-50.0, 50.0);
                    let vy = -cv.1.abs() * 0.3 - rng.range(30.0, 130.0);
                    debris(particles, rng, kind, true, x, py, vx, vy, mass / k as f32);
                }
            }
        }

        // частицы
        let mut splashes: Vec<(f32, f32)> = Vec::new();
        let mut i = 0;
        while i < particles.len() {
            let p = &mut particles[i];
            let m = p.kind.material();
            let size = p.r / m.scale;
            let (prev_x, prev_y) = (p.x, p.y);
            let mut term = if p.debris { m.debris_fall } else { m.fall_base + m.fall_size * size };
            term *= 1.0 + m.flutter * (t * 2.3 + p.phase * 1.7).sin();
            let k = if p.debris { m.debris_drag } else { m.drag };
            let swing = (t * m.sway_freq + p.phase).sin();
            let tx = if p.debris {
                (wind + bias_of(m)) * 0.3 * m.wind + m.debris_sway * swing
            } else {
                (wind + bias_of(m)) * m.wind * (0.5 + size * 0.25) + m.sway * swing
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
            // лист кружится и покачивается в такт сносу
            p.angle += (p.spin + m.spin * 0.7 * (t * m.sway_freq + p.phase).cos()) * dt;
            p.life -= dt;

            let mut dead = p.life <= 0.0 || p.y > h + 12.0 || p.x < -80.0 || p.x > w + 80.0;
            if !dead && (p.land || (solid && p.life.is_infinite())) && p.vy > 0.0 && p.kind == kind {
                for s in surfaces.values_mut() {
                    if !(p.y >= s.y - max_pile - 60.0 && prev_y <= s.y + 2.0) {
                        continue;
                    }
                    let col = ((p.x - s.x0) / COL_W).floor();
                    if col < 0.0 || col as usize >= s.h.len() || !s.vis[col as usize] {
                        continue;
                    }
                    let col = col as usize;
                    let top = s.y - s.h[col];
                    // садится, как только оказалась в слое. Раньше нужно было ещё и пересечь его
                    // верх сверху вниз, а куча могла вырасти под частицей между кадрами, и та
                    // пролетала насквозь, унося с собой массу
                    if p.y < top {
                        continue;
                    }
                    let fill = s.h[col] / max_pile;
                    if p.debris || rng.range(0.0, 1.0) > fill * fill {
                        deposit(s, col, p.mass);
                        dead = true;
                    } else if solid && s.y < h - 1.0 {
                        dead = true; // куча полна — частица тает, но в окно не залетает
                    }
                    if dead && mat.splash {
                        splashes.push((p.x, top));
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
                        dead = true; // упала на кромку без кучи или окно накрыло её
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

        // брызги: короткоживущие капельки без массы, чисто для вида
        for (x, y) in splashes {
            for _ in 0..2 {
                if particles.len() >= MAX_PARTICLES {
                    break;
                }
                particles.push(Particle {
                    x,
                    y: y - 1.0,
                    vx: rng.range(-70.0, 70.0),
                    vy: rng.range(-110.0, -45.0),
                    r: mat.scale * 0.9,
                    mass: 0.0,
                    land: false,
                    debris: true,
                    layer: 1,
                    phase: 0.0,
                    kind,
                    tint: 0,
                    angle: 0.0,
                    spin: 0.0,
                    life: rng.range(0.12, 0.28),
                });
            }
        }

        // осыпание куч, высыхание и срыв материала с краёв
        let spill = mat.chunk * 0.8;
        for s in surfaces.values_mut() {
            let n = s.h.len();
            if n < 2 {
                continue;
            }
            if mat.evaporate > 0.0 {
                for v in s.h.iter_mut() {
                    *v = (*v - mat.evaporate * dt).max(0.0);
                }
            }
            for pass in 0..mat.relax_passes {
                for j in 0..n - 1 {
                    let a = if pass % 2 == 0 { j } else { n - 2 - j };
                    if !(s.vis[a] && s.vis[a + 1]) {
                        continue;
                    }
                    let d = s.h[a] - s.h[a + 1];
                    if d > mat.slope {
                        let m = (d - mat.slope) * 0.25;
                        s.h[a] -= m;
                        s.h[a + 1] += m;
                    } else if d < -mat.slope {
                        let m = (-d - mat.slope) * 0.25;
                        s.h[a] += m;
                        s.h[a + 1] -= m;
                    }
                }
            }
            if s.y >= h - 1.0 {
                continue; // у земли краёв нет
            }
            if s.vis[0] && s.h[0] > mat.edge_keep {
                let m = (s.h[0] - mat.edge_keep) * 0.2;
                s.h[0] -= m;
                s.spill_l += m * COL_W;
                while s.spill_l >= spill {
                    s.spill_l -= spill;
                    let y = s.y - rng.range(0.0, s.h[0]);
                    let (vx, vy) = (rng.range(-30.0, -6.0), rng.range(-10.0, 10.0));
                    debris(particles, rng, kind, true, s.frame.min_x() - 1.0, y, vx, vy, spill);
                }
            }
            if s.vis[n - 1] && s.h[n - 1] > mat.edge_keep {
                let m = (s.h[n - 1] - mat.edge_keep) * 0.2;
                s.h[n - 1] -= m;
                s.spill_r += m * COL_W;
                while s.spill_r >= spill {
                    s.spill_r -= spill;
                    let y = s.y - rng.range(0.0, s.h[n - 1]);
                    let (vx, vy) = (rng.range(6.0, 30.0), rng.range(-10.0, 10.0));
                    debris(particles, rng, kind, true, s.frame.max_x() + 1.0, y, vx, vy, spill);
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
    fn smaller_pile_scale_keeps_piles_lower_and_loses_no_mass() {
        let win = [WindowInfo { id: 7, rect: Rect::new(200.0, 300.0, 400.0, 300.0) }];
        let peak = |scale: f32| {
            let mut s = sim();
            s.pile_scale = scale;
            let mut peak = 0.0f32;
            for _ in 0..(60 * 90) {
                s.refresh(&win, None, 0.05);
                s.step(1.0 / 60.0, (-500.0, -500.0), (0.0, 0.0));
                peak = peak.max(s.surfaces[&7].h.iter().copied().fold(0.0, f32::max));
            }
            peak
        };
        let (full, small) = (peak(1.0), peak(0.4));
        assert!(small < full * 0.7, "кучи не стали ниже: {small} против {full}");

        // уменьшили предел у готовых куч — они осыпались, а масса сохранилась частицами
        let mut s = sim();
        s.rate = 0.0;
        s.refresh(&win, None, 0.05);
        for h in s.surfaces.get_mut(&7).unwrap().h.iter_mut() {
            *h = 5.0;
        }
        s.set_pile_scale(0.5);
        assert_eq!(s.surfaces[&7].h.iter().sum::<f32>(), 0.0);
        assert!(s.particles.iter().map(|p| p.mass).sum::<f32>() > 0.0);
        // увеличили — ничего не трогаем
        s.set_pile_scale(1.0);
        assert_eq!(s.pile_scale, 1.0);
    }

    #[test]
    fn cursor_carves_the_pile() {
        let mut s = sim();
        s.gather = false;
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

    #[test]
    fn every_material_falls_and_lands() {
        use crate::materials::KINDS;
        for kind in KINDS {
            let mut s = sim();
            s.set_kind(kind);
            let win = [WindowInfo { id: 7, rect: Rect::new(200.0, 300.0, 400.0, 300.0) }];
            let mut peak = 0.0f32;
            for _ in 0..(60 * 40) {
                s.refresh(&win, None, 0.05);
                s.step(1.0 / 60.0, (-500.0, -500.0), (0.0, 0.0));
                peak = peak.max(s.surfaces[&7].h.iter().sum());
            }
            assert!(peak > 0.5, "{kind:?}: на окно ничего не легло");
            assert!(s.particles.len() <= MAX_PARTICLES);
            let max_pile = kind.material().max_pile;
            for surf in s.surfaces.values() {
                assert!(surf.h.iter().all(|h| h.is_finite() && *h >= 0.0 && *h < max_pile * 3.0), "{kind:?}");
            }
            assert!(s.particles.iter().all(|p| p.x.is_finite() && p.y.is_finite()), "{kind:?}");
        }
    }

    #[test]
    fn dust_storm_blows_sideways_and_fills_the_whole_screen() {
        let mut s = sim();
        s.set_kind(Kind::Dust);
        for _ in 0..(60 * 12) {
            s.step(1.0 / 60.0, (-500.0, -500.0), (0.0, 0.0));
        }
        let n = s.particles.len() as f32;
        assert!(n > 300.0, "бури нет: {n} частиц");
        let mean_vx = s.particles.iter().map(|p| p.vx).sum::<f32>() / n;
        assert!(mean_vx > 80.0, "ветер слабый или не вправо: {mean_vx}");
        // пыль залетает сбоку, поэтому и внизу слева она есть
        let low_left = s.particles.iter().filter(|p| p.x < s.w * 0.4 && p.y > s.h * 0.6).count();
        assert!(low_left > 5, "нижний левый угол пуст: {low_left}");
        // у обычного снега порывов в одну сторону нет
        let mut snow = sim();
        for _ in 0..(60 * 12) {
            snow.step(1.0 / 60.0, (-500.0, -500.0), (0.0, 0.0));
        }
        let sn = snow.particles.len().max(1) as f32;
        assert!((snow.particles.iter().map(|p| p.vx).sum::<f32>() / sn).abs() < 60.0);
    }

    /// Средняя горизонтальная скорость частиц после 12 секунд снегопада при данном ветре.
    fn mean_vx(kind: Kind, dir: WindDir, force: f32) -> f32 {
        let mut s = sim();
        s.set_kind(kind);
        s.wind_dir = dir;
        s.wind_force = force;
        for _ in 0..(60 * 12) {
            s.step(1.0 / 60.0, (-500.0, -500.0), (0.0, 0.0));
        }
        s.particles.iter().map(|p| p.vx).sum::<f32>() / s.particles.len().max(1) as f32
    }

    #[test]
    fn wind_direction_and_force_steer_the_particles() {
        let right = mean_vx(Kind::Snow, WindDir::Right, 1.0);
        let left = mean_vx(Kind::Snow, WindDir::Left, 1.0);
        let calm = mean_vx(Kind::Snow, WindDir::Calm, 1.0);
        assert!(right > 20.0, "вправо: {right}");
        assert!(left < -20.0, "влево: {left}");
        assert!(calm.abs() < 12.0, "без ветра: {calm}");
        // сильнее ветер — быстрее снос, слабее — медленнее
        let strong = mean_vx(Kind::Snow, WindDir::Right, 2.0);
        let weak = mean_vx(Kind::Snow, WindDir::Right, 0.5);
        assert!(strong > right * 1.5 && weak < right * 0.75, "слабый {weak}, средний {right}, сильный {strong}");
        // «как обычно» остаётся прежним качанием
        assert!(mean_vx(Kind::Snow, WindDir::Auto, 1.0).abs() < 60.0);
    }

    #[test]
    fn dust_storm_follows_the_chosen_wind() {
        // против ветра по умолчанию: влево пыль залетает справа
        let mut s = sim();
        s.set_kind(Kind::Dust);
        s.wind_dir = WindDir::Left;
        for _ in 0..(60 * 12) {
            s.step(1.0 / 60.0, (-500.0, -500.0), (0.0, 0.0));
        }
        let n = s.particles.len() as f32;
        assert!(s.particles.iter().map(|p| p.vx).sum::<f32>() / n < -80.0);
        assert!(s.particles.iter().filter(|p| p.x > s.w * 0.6 && p.y > s.h * 0.6).count() > 5);
        // без ветра буря превращается в тихую пыль: ни сноса, ни залёта сбоку
        assert!(mean_vx(Kind::Dust, WindDir::Calm, 1.0).abs() < 20.0);
        // ураган сильнее обычной бури
        assert!(mean_vx(Kind::Dust, WindDir::Right, 2.0) > mean_vx(Kind::Dust, WindDir::Right, 1.0) * 1.4);
    }

    #[test]
    fn power_saving_makes_the_precipitation_lighter() {
        let count = |scale: f32| {
            let mut s = sim();
            s.rate_scale = scale;
            for _ in 0..(60 * 10) {
                s.step(1.0 / 60.0, (-500.0, -500.0), (0.0, 0.0));
            }
            s.particles.len() as f32
        };
        let (full, light) = (count(1.0), count(0.4));
        assert!(light < full * 0.55 && light > full * 0.25, "{light} против {full}");
    }

    #[test]
    fn haze_exists_only_for_the_dust_storm() {
        let mut s = sim();
        assert_eq!(s.haze()[3], 0.0);
        s.set_kind(Kind::Dust);
        let a = s.haze()[3];
        assert!(a > 0.1 && a <= 0.18, "{a}");
        for _ in 0..(60 * 5) {
            s.step(1.0 / 60.0, (-500.0, -500.0), (0.0, 0.0));
            let a = s.haze()[3];
            assert!((0.13..=0.181).contains(&a), "дымка вышла за границы: {a}");
        }
    }

    #[test]
    fn water_levels_out_and_dries() {
        let mut s = sim();
        s.set_kind(Kind::Rain);
        s.rate = 0.0;
        let win = [WindowInfo { id: 7, rect: Rect::new(200.0, 300.0, 400.0, 300.0) }];
        s.refresh(&win, None, 0.05);
        s.surfaces.get_mut(&7).unwrap().h[100] = 3.0; // капля посреди кромки
        for _ in 0..30 {
            s.step(1.0 / 60.0, (-500.0, -500.0), (0.0, 0.0));
        }
        let surf = &s.surfaces[&7];
        assert!(surf.h[100] < 1.5, "вода не растеклась: {}", surf.h[100]);
        assert!(surf.h[97] > 0.0 && surf.h[103] > 0.0);
        for _ in 0..(60 * 20) {
            s.step(1.0 / 60.0, (-500.0, -500.0), (0.0, 0.0));
        }
        assert_eq!(s.surfaces[&7].h.iter().sum::<f32>(), 0.0, "вода не высохла");
    }

    #[test]
    fn changing_material_clears_piles() {
        let mut s = sim();
        s.rate = 0.0;
        let win = [WindowInfo { id: 7, rect: Rect::new(200.0, 300.0, 400.0, 300.0) }];
        s.refresh(&win, None, 0.05);
        for h in s.surfaces.get_mut(&7).unwrap().h.iter_mut() {
            *h = 5.0;
        }
        s.set_kind(Kind::Sand);
        assert_eq!(s.kind(), Kind::Sand);
        assert_eq!(total_pile(&s), 0.0);
        // старый снег осыпается, но больше нигде не ложится
        assert!(s.particles.iter().all(|p| p.kind == Kind::Snow && !p.land));
        for _ in 0..(60 * 10) {
            s.refresh(&win, None, 0.05);
            s.step(1.0 / 60.0, (-500.0, -500.0), (0.0, 0.0));
        }
        assert_eq!(total_pile(&s), 0.0);
        assert!(s.particles.is_empty());
    }
}
