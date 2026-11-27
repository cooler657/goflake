//! Уборщик (трактор, дворник с метлой, пингвин с лопатой, пикап или снегоуборщик).
//!
//! Он всегда стоит на кромке панели задач или Dock (а если их нет — на низу экрана)
//! в левом конце, «на стоянке». Если на стоянке нажать на него мышью, он выезжает и
//! сгребает кучу вдоль всей кромки; если перетащить его на окно, он сразу начинает
//! чистить верхнюю кромку этого окна с того места, куда его отпустили. Кроме того, когда
//! выбрана автоматическая работа, он сам выезжает раз в `Plow::every` секунд, если куча
//! на кромке выросла.
//!
//! Снятое уборщик везёт с собой, а в конце пути сбрасывает за край комками; по дороге
//! от лопаты летят лишь брызги для вида (без массы). После кромки панели он едет обратно
//! налегке и встаёт на стоянку; у окна просто встаёт на стоянку в конце пути.

use super::*;
use crate::decor::{bar, circle};
use crate::look::FlakeLook;

/// Сколько фигур могут отдать две машины за кадр: своя и несомая курсором (под них
/// запасён буфер на GPU).
pub const PLOW_MAX: usize = 64;
/// Сколько фигур в одной машине.
const VEHICLE_MAX: usize = 32;
/// Скорость, точек в секунду.
const SPEED: f32 = 90.0;
/// Налегке (обратно на стоянку) едет быстрее.
const BACK_SPEED: f32 = 140.0;
/// При какой средней высоте кучи (доля предела) уборщик выезжает сам.
const TRIGGER: f32 = 0.25;
/// Длина машины вместе с лопатой, точек.
const LEN: f32 = 32.0;
/// Расстояние от центра кузова до передней кромки лопаты, точек.
const FRONT: f32 = 16.8;
/// Центр стоянки от левого конца кромки, точек.
const PARK: f32 = 26.0;
/// Область, за которую машину можно взять мышью: вправо и влево от центра, вверх от
/// основания (выше самой машины, чтобы в неё было легко попасть) и вниз.
const GRAB_X: f32 = 22.0;
const GRAB_UP: f32 = 30.0;
const GRAB_DOWN: f32 = 4.0;
/// Окно считается «под курсором», если курсор не выше его верхней кромки больше чем на столько.
const DROP_ABOVE: f32 = 24.0;
/// На сколько выше курсора рисуется несомая машина (чтобы курсор был у неё посередине).
const CARRY_LIFT: f32 = 12.0;

struct Run {
    surface: i64,
    /// Передняя кромка лопаты.
    x: f32,
    /// Куда едет: 1 — вправо, -1 — влево.
    dir: f32,
    /// Откуда начата работа: позади этой точки никто не чистится.
    from: f32,
    /// Центр стоянки, куда нужно вернуться (только если `exit`).
    home: f32,
    /// Кромка панели или низ экрана: в конце пути машина уезжает за край, а потом
    /// возвращается. У окна она просто встаёт на стоянку.
    exit: bool,
    /// Возвращается на стоянку налегке и ничего не чистит.
    back: bool,
    /// Масса, которую лопата уже сгребла и везёт.
    load: f32,
}

/// Состояние уборщика. Живёт в `Sim::plow`.
pub struct Plow {
    /// Уборщик есть на экране (стоит на стоянке и откликается на мышь).
    pub enabled: bool,
    /// Пауза между автоматическими выездами, секунд; 0 — только вручную.
    pub every: f32,
    run: Option<Run>,
    /// Сколько секунд осталось отдыхать.
    wait: f32,
    /// Кто чистит.
    pub vehicle: Vehicle,
    /// Машину взяли мышью (на этом экране): со стоянки она исчезла.
    pub held: bool,
    /// Где рисовать машину, которую несёт курсор (в координатах этого экрана).
    pub carry: Option<(f32, f32)>,
}

impl Default for Plow {
    fn default() -> Self {
        Self { enabled: false, every: 0.0, run: None, wait: 0.0, vehicle: Vehicle::Tractor, held: false, carry: None }
    }
}

impl Plow {
    /// Меняет частоту выездов. Если паузу сократили, отдыхать остаётся не дольше новой.
    pub fn set_every(&mut self, every: f32) {
        self.every = every;
        self.wait = self.wait.min(every);
    }

    /// Выехал ли уборщик сейчас.
    #[cfg(test)]
    pub fn busy(&self) -> bool {
        self.run.is_some()
    }
}

/// Средняя высота кучи под кузовом, чтобы машина стояла на ней, а не тонула.
fn pile_under(s: &Surface, x: f32) -> f32 {
    if s.h.is_empty() {
        return 0.0;
    }
    let a = (((x - 8.0 - s.x0) / COL_W).floor().max(0.0)) as usize;
    let b = (((x + 8.0 - s.x0) / COL_W).floor().max(0.0) as usize).min(s.h.len() - 1);
    if a > b {
        return 0.0;
    }
    s.h[a..=b].iter().sum::<f32>() / (b - a + 1) as f32
}

impl Sim {
    /// Кромка, на которой стоит и которую чистит уборщик: панель задач / Dock, иначе низ экрана.
    fn park_surface(&self) -> Option<i64> {
        [DOCK, GROUND].into_iter().find(|id| self.surfaces.contains_key(id))
    }

    /// Где стоит уборщик на стоянке: центр кузова и основание (с учётом кучи под ним).
    /// `None`, если он выключен, взят мышью или выехал.
    pub fn plow_spot(&self) -> Option<(f32, f32)> {
        if !self.plow.enabled || self.plow.held || self.plow.run.is_some() {
            return None;
        }
        let s = self.surfaces.get(&self.park_surface()?)?;
        let x = s.x0 + PARK;
        Some((x, s.y - pile_under(s, x)))
    }

    /// Лежит ли точка `c` на машине, которую можно взять мышью.
    pub fn plow_grab(&self, c: (f32, f32)) -> bool {
        self.plow_spot().is_some_and(|(x, base)| (c.0 - x).abs() <= GRAB_X && c.1 >= base - GRAB_UP && c.1 <= base + GRAB_DOWN)
    }

    /// Выезжает на кромку панели с левого конца и едет вправо. `false`, если нельзя.
    pub fn plow_start(&mut self) -> bool {
        if !self.plow.enabled || self.plow.run.is_some() {
            return false;
        }
        let Some(id) = self.park_surface() else { return false };
        let lo = self.surfaces[&id].x0;
        self.plow.run =
            Some(Run { surface: id, x: lo + PARK + FRONT, dir: 1.0, from: lo - LEN, home: lo + PARK, exit: true, back: false, load: 0.0 });
        true
    }

    /// Машину отпустили в точке `(x, y)`: если там окно, она сразу начинает чистить его верхнюю
    /// кромку, поехав в ту сторону, где кромка длиннее. `false`, если там не окно или занято.
    pub fn plow_drop(&mut self, x: f32, y: f32) -> bool {
        if !self.plow.enabled || self.plow.run.is_some() {
            return false;
        }
        // верхнее окно под точкой решает всё: сквозь него на нижнее не «уронить»
        let Some(id) = self
            .solids
            .iter()
            .zip(&self.solid_ids)
            .find(|(r, _)| x >= r.min_x() && x < r.max_x() && y >= r.min_y() - DROP_ABOVE && y < r.max_y())
            .and_then(|(_, id)| *id)
        else {
            return false;
        };
        let Some(s) = self.surfaces.get(&id) else { return false };
        let (lo, hi) = (s.x0, s.x0 + s.h.len() as f32 * COL_W);
        let x = x.clamp(lo, hi);
        let dir = if x - lo > hi - x { -1.0 } else { 1.0 };
        let front = x + dir * FRONT;
        self.plow.run = Some(Run { surface: id, x: front, dir, from: front, home: x, exit: false, back: false, load: 0.0 });
        true
    }

    /// Шаг уборщика: выезд, движение, снятие столбиков под лопатой.
    pub(super) fn plow_step(&mut self, dt: f32) {
        if !self.plow.enabled {
            self.plow.run = None;
            return;
        }
        let mat = self.kind.material();
        let Some(mut run) = self.plow.run.take() else {
            self.plow.wait -= dt;
            if self.plow.every <= 0.0 || self.plow.held || self.plow.wait > 0.0 {
                return;
            }
            let Some(id) = self.park_surface() else { return };
            let s = &self.surfaces[&id];
            let (sum, n) = s.h.iter().zip(&s.vis).filter(|(_, v)| **v).fold((0.0, 0), |(a, n), (h, _)| (a + h, n + 1));
            if n == 0 || sum / (n as f32) < TRIGGER * mat.max_pile * self.pile_scale {
                return;
            }
            self.plow_start();
            return;
        };

        let dir = run.dir;
        let Some(s) = self.surfaces.get_mut(&run.surface) else {
            self.plow.wait = self.plow.every; // кромка исчезла — выезд отменён, груз пропадает
            return;
        };
        let (lo, hi) = (s.x0, s.x0 + s.h.len() as f32 * COL_W);

        if run.back {
            // налегке обратно на стоянку, ничего не чистим
            run.x += dir * BACK_SPEED * dt;
            if dir * (run.x - dir * FRONT - run.home) <= 0.0 {
                self.plow.wait = self.plow.every;
            } else {
                self.plow.run = Some(run);
            }
            return;
        }

        let prev = run.x;
        run.x += dir * SPEED * dt;
        let (a, b) = if prev < run.x { (prev, run.x) } else { (run.x, prev) };
        for i in 0..s.h.len() {
            let cx = lo + (i as f32 + 0.5) * COL_W;
            // весь след за лопатой остаётся чистым: иначе куча впереди «стекала» бы на
            // очищенные столбики (сглаживание склонов) и оставляла за лопатой хвост
            let behind = if dir > 0.0 { cx >= run.from && cx < run.x } else { cx <= run.from && cx > run.x };
            if !behind || !s.vis[i] {
                continue;
            }
            let h = std::mem::take(&mut s.h[i]);
            run.load += h * COL_W;
            // брызги от лопаты: вперёд и вверх, живут недолго, массы не несут
            let front = cx >= a && cx < b;
            if front && h > 0.3 && self.particles.len() < MAX_PARTICLES && self.rng.range(0.0, 1.0) < 0.7 {
                self.particles.push(Particle {
                    x: cx,
                    y: s.y - self.rng.range(0.0, h),
                    vx: dir * self.rng.range(20.0, 90.0),
                    vy: -self.rng.range(40.0, 120.0),
                    r: mat.scale * 0.9,
                    mass: 0.0,
                    land: false,
                    debris: true,
                    layer: 1,
                    phase: 0.0,
                    kind: self.kind,
                    tint: self.rng.byte(),
                    angle: 0.0,
                    spin: 0.0,
                    life: self.rng.range(0.2, 0.5),
                });
            }
        }
        // у панели машина уезжает за край, у окна останавливается на краю
        let reach = if run.exit { LEN } else { 0.0 };
        let done = if dir > 0.0 { run.x > hi + reach } else { run.x < lo - reach };
        if done {
            // в конце пути груз сброшен за край
            let k = ((run.load / mat.chunk) as usize).clamp(1, 40);
            let ex = if dir > 0.0 { hi - 1.0 } else { lo + 1.0 };
            for _ in 0..k {
                let py = s.y - self.rng.range(0.0, 6.0);
                let vx = dir * self.rng.range(60.0, 140.0);
                let vy = -self.rng.range(30.0, 90.0);
                debris(&mut self.particles, &mut self.rng, self.kind, false, ex, py, vx, vy, run.load / k as f32);
            }
            if run.exit {
                // разворачивается и едет назад: центр кузова остаётся на месте, лопата теперь с другой стороны
                let center = run.x - dir * FRONT;
                run.dir = -dir;
                run.x = center + run.dir * FRONT;
                run.back = true;
                run.load = 0.0;
                self.plow.run = Some(run);
            } else {
                self.plow.wait = self.plow.every;
            }
        } else {
            self.plow.run = Some(run);
        }
    }
}

/// Кто чистит кромку.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Vehicle {
    Tractor,
    /// Дворник с метлой.
    Janitor,
    /// Пингвин с лопатой.
    Penguin,
    Pickup,
    /// Снегоуборочная машина со шнеком.
    Blower,
}

/// Все уборщики в порядке пунктов меню.
pub const VEHICLES: [Vehicle; 5] =
    [Vehicle::Tractor, Vehicle::Janitor, Vehicle::Penguin, Vehicle::Pickup, Vehicle::Blower];

impl Vehicle {
    pub fn label(self) -> &'static str {
        match self {
            Vehicle::Tractor => "Трактор",
            Vehicle::Janitor => "Дворник с метлой",
            Vehicle::Penguin => "Пингвин с лопатой",
            Vehicle::Pickup => "Пикап",
            Vehicle::Blower => "Снегоуборщик",
        }
    }
}

fn rgb(r: u8, g: u8, b: u8) -> [f32; 3] {
    [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0]
}

/// Где стоит уборщик: переводит координаты машины в координаты экрана.
/// `lx` — вперёд от центра кузова, `ly` — вверх от кромки; спереди (`lx` = `FRONT`) всегда
/// то, чем сгребают: лопата, метла, шнек.
struct Pose {
    ox: f32,
    dir: f32,
    base: f32,
    /// Время, секунд: пешие уборщики переступают.
    t: f32,
}

impl Pose {
    fn p(&self, lx: f32, ly: f32) -> [f32; 2] {
        [self.ox + self.dir * lx, self.base - ly]
    }
}

const TIRE: [f32; 3] = [0.125, 0.125, 0.15];
const HUB: [f32; 3] = [0.59, 0.60, 0.63];
const STEEL: [f32; 3] = [0.47, 0.49, 0.51];
const YELLOW: [f32; 3] = [0.95, 0.78, 0.16];

fn tractor(g: &Pose, out: &mut Vec<FlakeLook>) {
    let (orange, dark) = (rgb(232, 96, 36), rgb(52, 56, 66));
    // задние и передние колёса
    out.push(circle(g.p(-8.0, 5.0), 5.0, TIRE));
    out.push(circle(g.p(-8.0, 5.0), 2.0, HUB));
    out.push(circle(g.p(7.0, 3.0), 3.0, TIRE));
    out.push(circle(g.p(7.0, 3.0), 1.2, HUB));
    // рама и капот
    out.push(bar(g.p(-6.0, 7.5), g.p(8.0, 7.5), 2.3, orange));
    out.push(bar(g.p(3.0, 9.8), g.p(11.0, 9.8), 1.8, rgb(206, 80, 28)));
    // кабина: стекло, стойки, крыша, мигалка
    out.push(bar(g.p(-11.0, 13.0), g.p(-5.0, 13.0), 2.4, rgb(140, 190, 230)));
    out.push(bar(g.p(-11.6, 10.0), g.p(-11.6, 15.6), 0.6, dark));
    out.push(bar(g.p(-4.4, 10.0), g.p(-4.4, 15.6), 0.6, dark));
    out.push(bar(g.p(-12.4, 16.0), g.p(-3.6, 16.0), 0.8, dark));
    out.push(circle(g.p(-8.0, 17.4), 1.0, rgb(255, 180, 30)));
    // выхлопная труба
    out.push(bar(g.p(7.0, 11.0), g.p(7.0, 14.5), 0.7, dark));
    // лопата на раме
    out.push(bar(g.p(11.0, 6.0), g.p(15.0, 3.5), 0.8, STEEL));
    out.push(bar(g.p(15.6, 1.2), g.p(16.6, 9.5), 1.2, YELLOW));
}

fn janitor(g: &Pose, out: &mut Vec<FlakeLook>) {
    let (pants, vest, skin, cap, wood) =
        (rgb(40, 50, 90), rgb(240, 150, 30), rgb(240, 200, 170), rgb(50, 70, 140), rgb(150, 105, 60));
    // идёт: ноги качаются в такт
    let sw = (g.t * 9.0).sin() * 3.0;
    out.push(bar(g.p(-5.0, 8.5), g.p(-5.0 - sw, 0.8), 1.0, pants));
    out.push(bar(g.p(-5.0, 8.5), g.p(-5.0 + sw, 0.8), 1.0, pants));
    // жилет со светоотражающей полосой, голова, кепка
    out.push(bar(g.p(-5.0, 9.0), g.p(-5.0, 14.5), 2.3, vest));
    out.push(bar(g.p(-7.0, 12.2), g.p(-3.0, 12.2), 0.4, rgb(230, 230, 200)));
    out.push(circle(g.p(-4.6, 17.6), 2.0, skin));
    out.push(bar(g.p(-6.6, 19.2), g.p(-2.6, 19.2), 0.9, cap));
    out.push(bar(g.p(-2.8, 18.9), g.p(-0.8, 18.6), 0.4, cap));
    // рука держит метлу, метла — прямо перед ним
    out.push(bar(g.p(-4.0, 13.0), g.p(2.0, 9.5), 0.8, vest));
    out.push(circle(g.p(2.2, 9.4), 0.8, skin));
    out.push(bar(g.p(0.0, 11.0), g.p(15.5, 2.6), 0.45, wood));
    out.push(bar(g.p(16.0, 0.9), g.p(16.0, 4.2), 1.0, rgb(220, 180, 80)));
}

fn penguin(g: &Pose, out: &mut Vec<FlakeLook>) {
    let (navy, white, orange, wood) = (rgb(30, 36, 54), rgb(240, 242, 246), rgb(245, 150, 40), rgb(150, 105, 60));
    // вперевалочку: тело качается, лапы поднимаются по очереди
    let w = (g.t * 8.0).sin();
    let s = |lx: f32| lx + w * 0.5;
    out.push(bar(g.p(-4.6, 0.6 + w.max(0.0)), g.p(-2.0, 0.6 + w.max(0.0)), 0.6, orange));
    out.push(bar(g.p(-3.2, 0.6 + (-w).max(0.0)), g.p(-0.6, 0.6 + (-w).max(0.0)), 0.6, orange));
    out.push(bar(g.p(s(-3.0), 4.0), g.p(s(-3.0), 10.0), 3.6, navy));
    out.push(bar(g.p(s(-2.2), 4.6), g.p(s(-2.2), 9.4), 2.4, white));
    out.push(circle(g.p(s(-3.0), 14.0), 2.7, navy));
    out.push(circle(g.p(s(-1.6), 14.6), 0.55, white));
    out.push(circle(g.p(s(-1.4), 14.6), 0.28, navy));
    out.push(bar(g.p(s(-0.6), 13.4), g.p(s(1.6), 13.0), 0.7, orange));
    // шарф и крыло, которым он держит лопату
    out.push(bar(g.p(s(-3.0), 11.8), g.p(s(-1.0), 11.5), 0.7, rgb(210, 50, 50)));
    out.push(bar(g.p(s(-3.2), 10.0), g.p(s(-0.5), 7.4), 0.8, navy));
    out.push(bar(g.p(s(-0.5), 7.6), g.p(15.5, 2.4), 0.4, wood));
    out.push(bar(g.p(16.2, 0.9), g.p(16.6, 4.4), 1.0, rgb(150, 155, 165)));
}

fn pickup(g: &Pose, out: &mut Vec<FlakeLook>) {
    let (red, dark_red, glass) = (rgb(200, 42, 36), rgb(150, 30, 28), rgb(140, 190, 230));
    out.push(circle(g.p(-8.5, 3.2), 3.2, TIRE));
    out.push(circle(g.p(-8.5, 3.2), 1.3, HUB));
    out.push(circle(g.p(6.0, 3.2), 3.2, TIRE));
    out.push(circle(g.p(6.0, 3.2), 1.3, HUB));
    // шасси, борта кузова, высокая кабина со стеклом, капот
    out.push(bar(g.p(-13.0, 6.0), g.p(11.0, 6.0), 2.4, red));
    out.push(bar(g.p(-13.0, 9.4), g.p(-3.2, 9.4), 1.0, dark_red));
    out.push(bar(g.p(-13.0, 8.4), g.p(-13.0, 9.4), 0.6, dark_red));
    out.push(bar(g.p(-1.0, 11.0), g.p(5.6, 11.0), 2.9, red));
    out.push(bar(g.p(0.4, 12.5), g.p(4.6, 12.5), 1.3, glass));
    out.push(bar(g.p(6.8, 9.0), g.p(11.0, 9.0), 1.6, red));
    out.push(circle(g.p(11.6, 7.4), 0.8, YELLOW));
    // мигалка на крыше
    out.push(circle(g.p(2.5, 15.2), 0.9, rgb(255, 180, 30)));
    // отвал спереди
    out.push(bar(g.p(11.5, 4.4), g.p(15.0, 3.4), 0.7, STEEL));
    out.push(bar(g.p(15.8, 0.9), g.p(16.8, 7.6), 1.2, YELLOW));
}

fn blower(g: &Pose, out: &mut Vec<FlakeLook>) {
    let (red, dark) = (rgb(214, 48, 40), rgb(52, 56, 66));
    out.push(circle(g.p(0.5, 3.2), 3.2, TIRE));
    out.push(circle(g.p(0.5, 3.2), 1.3, HUB));
    // корпус, мотор, кожух шнека
    out.push(bar(g.p(-3.0, 6.4), g.p(9.0, 6.4), 2.6, red));
    out.push(bar(g.p(-1.0, 9.6), g.p(3.0, 9.6), 1.2, dark));
    out.push(bar(g.p(11.0, 3.8), g.p(14.4, 3.8), 2.8, rgb(170, 36, 32)));
    out.push(circle(g.p(13.6, 7.2), 0.7, YELLOW));
    // труба, из которой вылетает снег
    out.push(bar(g.p(10.0, 8.6), g.p(10.0, 11.5), 0.9, dark));
    out.push(bar(g.p(9.3, 12.0), g.p(11.8, 12.8), 0.7, dark));
    // ручки, за которые толкают
    out.push(bar(g.p(-3.5, 7.6), g.p(-12.0, 13.0), 0.5, dark));
    out.push(bar(g.p(-12.0, 13.0), g.p(-15.0, 13.6), 0.8, dark));
    // шнек
    out.push(bar(g.p(15.6, 0.9), g.p(15.6, 6.4), 1.2, STEEL));
}

fn draw(vehicle: Vehicle, g: &Pose, out: &mut Vec<FlakeLook>) {
    let start = out.len();
    match vehicle {
        Vehicle::Tractor => tractor(g, out),
        Vehicle::Janitor => janitor(g, out),
        Vehicle::Penguin => penguin(g, out),
        Vehicle::Pickup => pickup(g, out),
        Vehicle::Blower => blower(g, out),
    }
    out.truncate(start + VEHICLE_MAX);
}

/// Дописывает в `out` фигуры уборщика: в пути, на стоянке и/или несомого курсором.
pub fn looks(sim: &Sim, out: &mut Vec<FlakeLook>) {
    if !sim.plow.enabled {
        return;
    }
    let v = sim.plow.vehicle;
    if let Some(run) = &sim.plow.run {
        if let Some(s) = sim.surfaces.get(&run.surface) {
            draw(v, &Pose { ox: run.x - run.dir * FRONT, dir: run.dir, base: s.y, t: sim.t }, out);
        }
    } else if let Some((ox, base)) = sim.plow_spot() {
        // на стоянке пешие уборщики стоят на месте (t = 0)
        draw(v, &Pose { ox, dir: 1.0, base, t: 0.0 }, out);
    }
    if let Some((x, y)) = sim.plow.carry {
        draw(v, &Pose { ox: x, dir: 1.0, base: y + CARRY_LIFT, t: sim.t }, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Экран 1000×800 с панелью задач снизу, на её кромке лежит куча высотой `h`.
    fn sim_with_pile(h: f32) -> Sim {
        let mut s = Sim::new(5);
        s.w = 1000.0;
        s.h = 800.0;
        s.rate = 0.0;
        s.plow.enabled = true;
        s.refresh(&[], Some(Rect::new(0.0, 760.0, 1000.0, 40.0)), 0.05);
        for v in s.surfaces.get_mut(&DOCK).unwrap().h.iter_mut() {
            *v = h;
        }
        s
    }

    fn mean(s: &Sim) -> f32 {
        let h = &s.surfaces[&DOCK].h;
        h.iter().sum::<f32>() / h.len() as f32
    }

    fn run_until_idle(s: &mut Sim, seconds: u32) {
        for _ in 0..(60 * seconds) {
            s.step(1.0 / 60.0, (-500.0, -500.0), (0.0, 0.0));
            if !s.plow.busy() {
                return;
            }
        }
        panic!("уборщик не закончил за {seconds} с");
    }

    #[test]
    fn plow_clears_a_big_pile_and_then_rests() {
        let mut s = sim_with_pile(0.8 * s_max());
        s.plow.every = 30.0;
        let before = mean(&s);
        let total0 = before * 500.0 * COL_W;
        let mut started = false;
        let mut finished = false;
        for _ in 0..(60 * 40) {
            s.step(1.0 / 60.0, (-500.0, -500.0), (0.0, 0.0));
            let mut shapes = Vec::new();
            looks(&s, &mut shapes);
            assert!(!shapes.is_empty() && shapes.len() <= PLOW_MAX, "уборщика не видно");
            assert!(shapes.iter().all(|f| f.center[0].is_finite() && f.center[1] <= 760.0 + 1.0), "техника ушла под кромку");
            started |= s.plow.busy();
            if started && !s.plow.busy() {
                finished = true;
                break;
            }
        }
        assert!(started, "уборщик не выехал");
        assert!(finished, "уборщик не закончил выезд и не вернулся");
        assert!(mean(&s) < before * 0.3, "куча осталась: {} из {before}", mean(&s));
        // отдыхает, а не едет сразу снова, и снова стоит на стоянке
        assert_eq!(s.plow.wait, 30.0);
        assert!(s.plow_spot().is_some());
        // масса сохранена: всё, что лопата сняла, сброшено в конце пути комками
        let kept = s.particles.iter().map(|p| p.mass).sum::<f32>() + mean(&s) * 500.0 * COL_W;
        assert!((kept - total0).abs() < 0.03 * total0, "масса: было {total0}, стало {kept}");
    }

    #[test]
    fn plow_leaves_small_piles_and_obeys_the_switch() {
        // автоматика включена, но куча маленькая
        let mut s = sim_with_pile(0.05 * s_max());
        s.plow.every = 30.0;
        for _ in 0..(60 * 5) {
            s.step(1.0 / 60.0, (-500.0, -500.0), (0.0, 0.0));
        }
        assert!(!s.plow.busy(), "выехал к почти пустой кромке");

        // «только вручную»: куча большая, а сам не выезжает
        let mut s = sim_with_pile(0.8 * s_max());
        for _ in 0..(60 * 5) {
            s.step(1.0 / 60.0, (-500.0, -500.0), (0.0, 0.0));
        }
        assert!(!s.plow.busy(), "выехал сам при ручном режиме");
        assert!(mean(&s) > 0.5 * s_max());
        // а по нажатию выезжает, хотя бы куча и была маленькой
        assert!(s.plow_start());
        assert!(s.plow.busy() && s.plow_spot().is_none(), "со стоянки он исчез");
        assert!(!s.plow_start(), "второй раз подряд выехать нельзя");
        // выключенного уборщика нет совсем: ни на стоянке, ни в кадре
        s.plow.enabled = false;
        s.step(1.0 / 60.0, (-500.0, -500.0), (0.0, 0.0));
        let mut shapes = Vec::new();
        looks(&s, &mut shapes);
        assert!(shapes.is_empty() && !s.plow.busy() && s.plow_spot().is_none());
        assert!(!s.plow_start());
    }

    #[test]
    fn the_parked_vehicle_can_be_grabbed_only_where_it_stands() {
        let s = sim_with_pile(0.0);
        let (x, base) = s.plow_spot().expect("уборщик на стоянке");
        assert_eq!(base, 760.0);
        assert!(s.plow_grab((x, base - 10.0)));
        assert!(s.plow_grab((x + 20.0, base - 25.0)));
        assert!(!s.plow_grab((x + 60.0, base - 10.0)));
        assert!(!s.plow_grab((x, base - 80.0)));
        // куча под ним поднимает его, а область захвата идёт следом
        let mut s = sim_with_pile(10.0);
        let (_, lifted) = s.plow_spot().unwrap();
        assert!((lifted - 750.0).abs() < 0.01, "{lifted}");
        assert!(s.plow_grab((x, lifted - 10.0)));
        // взятого мышью на стоянке нет
        s.plow.held = true;
        assert!(s.plow_spot().is_none() && !s.plow_grab((x, lifted - 10.0)));
        // несомый курсором рисуется, где курсор
        s.plow.carry = Some((300.0, 200.0));
        let mut shapes = Vec::new();
        looks(&s, &mut shapes);
        assert!(shapes.len() > 5 && shapes.iter().all(|f| (f.center[0] - 300.0).abs() < 20.0 && (f.center[1] - 200.0).abs() < 40.0));
    }

    #[test]
    fn dropping_on_a_window_cleans_its_edge_from_the_drop_point() {
        let win = |id, x, y, w, h| WindowInfo { id, rect: Rect::new(x, y, w, h) };
        let (back, front) = (win(1, 100.0, 300.0, 700.0, 300.0), win(2, 300.0, 200.0, 200.0, 400.0));
        let mut s = sim_with_pile(0.0);
        s.refresh(&[front, back], Some(Rect::new(0.0, 760.0, 1000.0, 40.0)), 0.05);
        for id in [1, 2] {
            for h in s.surfaces.get_mut(&id).unwrap().h.iter_mut() {
                *h = 5.0;
            }
        }
        let mass = |s: &Sim, id: i64| s.surfaces[&id].h.iter().sum::<f32>() * COL_W;
        let front0 = mass(&s, 2);
        // мимо любого окна — ничего не происходит
        assert!(!s.plow_drop(900.0, 100.0));
        assert!(!s.plow.busy());
        // над перекрытой частью заднего окна верхним считается переднее
        assert!(s.plow_drop(312.0, 350.0));
        assert_eq!(s.plow.run.as_ref().unwrap().surface, 2);
        assert!(!s.plow_drop(312.0, 350.0), "занят");
        run_until_idle(&mut s, 30);
        assert!(mass(&s, 2) < 0.2 * front0, "кромка переднего окна не очищена");
        let mid = |s: &Sim| s.surfaces[&1].h[((650.0 - s.surfaces[&1].x0) / COL_W) as usize];
        assert!(mid(&s) > 4.9, "заднее окно не должно было чиститься, а там {}", mid(&s));
        assert!(s.plow_spot().is_some(), "после выезда он снова на стоянке");

        // отпустили на заднем окне у левой трети: чистится только правее точки
        let mut s = sim_with_pile(0.0);
        s.refresh(&[back], Some(Rect::new(0.0, 760.0, 1000.0, 40.0)), 0.05);
        for h in s.surfaces.get_mut(&1).unwrap().h.iter_mut() {
            *h = 5.0;
        }
        let total0 = mass(&s, 1);
        assert!(s.plow_drop(250.0, 450.0));
        run_until_idle(&mut s, 30);
        let surf = &s.surfaces[&1];
        let col = |x: f32| ((x - surf.x0) / COL_W) as usize;
        assert!(surf.h[col(150.0)] > 3.0 && surf.h[col(240.0)] > 3.0, "левее точки отпускания снег должен остаться");
        assert!(surf.h[col(300.0)] == 0.0 && surf.h[col(700.0)] == 0.0, "правее точки отпускания должно быть чисто");
        let kept = s.particles.iter().map(|p| p.mass).sum::<f32>() + mass(&s, 1);
        assert!((kept - total0).abs() < 0.03 * total0, "масса: было {total0}, стало {kept}");
    }

    #[test]
    fn every_vehicle_is_drawn_in_front_of_its_blade_in_both_directions() {
        for v in VEHICLES {
            for dir in [1.0f32, -1.0] {
                let mut s = sim_with_pile(1.0);
                s.plow.vehicle = v;
                s.plow.run = Some(Run { surface: DOCK, x: 500.0, dir, from: 0.0, home: 0.0, exit: true, back: false, load: 0.0 });
                let mut shapes = Vec::new();
                looks(&s, &mut shapes);
                assert!(shapes.len() > 5 && shapes.len() <= VEHICLE_MAX, "{v:?}: {} фигур", shapes.len());
                for f in &shapes {
                    assert!(f.center[0].is_finite() && f.center[1].is_finite() && f.color[3] <= 1.0);
                    // стоит на кромке (панель задач начинается на 760), не проваливается под неё
                    assert!(f.center[1] <= 760.0 + 0.1 && f.center[1] >= 760.0 - 22.0, "{v:?}: {:?}", f.center);
                    // сзади от лопаты, не дальше длины машины; спереди — не дальше самой лопаты
                    let ahead = dir * (f.center[0] - 500.0);
                    assert!(ahead <= 0.5 && ahead >= -(LEN + 2.0), "{v:?}, {dir}: вынос {ahead}");
                }
            }
            // на стоянке лопата смотрит вправо, а машина целиком в пределах области захвата
            let mut s = sim_with_pile(0.0);
            s.plow.vehicle = v;
            let (x, _) = s.plow_spot().unwrap();
            let mut shapes = Vec::new();
            looks(&s, &mut shapes);
            assert!(shapes.len() > 5, "{v:?} на стоянке не нарисован");
            assert!(shapes.iter().all(|f| f.center[0] <= x + FRONT + 0.5 && f.center[0] >= x - (LEN - FRONT) - 2.0), "{v:?}");
        }
        assert_eq!(VEHICLES.len(), 5);
        assert!(VEHICLES.iter().all(|v| !v.label().is_empty()));
    }

    #[test]
    fn shortening_the_pause_cuts_the_rest() {
        let mut p = Plow { wait: 100.0, ..Plow::default() };
        p.set_every(20.0);
        assert_eq!((p.every, p.wait), (20.0, 20.0));
        p.set_every(180.0);
        assert_eq!((p.every, p.wait), (180.0, 20.0), "удлинять отдых задним числом не нужно");
        p.set_every(0.0);
        assert_eq!(p.every, 0.0);
    }

    /// Сохраняет кадры всех уборщиков, чтобы посмотреть на них глазами. Запуск:
    /// `GOFLAKE_DUMP=/папка cargo test dump_vehicles -- --ignored`; формат — как у
    /// `soft::tests::dump_preview` (`имя.raw` и `имя.txt`).
    #[test]
    #[ignore]
    fn dump_vehicles() {
        use crate::clock::Clock;
        use crate::sky::{Sky, SkyMode};
        let Some(dir) = std::env::var_os("GOFLAKE_DUMP") else { return };
        let dir = std::path::PathBuf::from(dir);
        for v in VEHICLES {
            let mut s = sim_with_pile(0.0);
            s.w = 120.0;
            s.h = 60.0;
            s.refresh(&[], Some(Rect::new(0.0, 44.0, 120.0, 16.0)), 0.05);
            s.plow.vehicle = v;
            s.plow.run = Some(Run { surface: DOCK, x: 90.0, dir: 1.0, from: 0.0, home: 0.0, exit: true, back: false, load: 0.0 });
            s.t = 0.3;
            let mut canvas = crate::soft::Canvas::new();
            let sky = Sky::new(SkyMode::Off, Clock { hour: 12.0, season: 5.5, unix: 0.0 });
            canvas.draw(&s, &sky, 8.0);
            let bytes: Vec<u8> = canvas.pixels.iter().flat_map(|p| p.to_le_bytes()).collect();
            std::fs::write(dir.join(format!("plow_{v:?}.raw")), bytes).unwrap();
            std::fs::write(dir.join(format!("plow_{v:?}.txt")), format!("{} {}", canvas.width, canvas.height)).unwrap();
            // та же машина на стоянке
            s.plow.run = None;
            let mut canvas = crate::soft::Canvas::new();
            canvas.draw(&s, &sky, 8.0);
            let bytes: Vec<u8> = canvas.pixels.iter().flat_map(|p| p.to_le_bytes()).collect();
            std::fs::write(dir.join(format!("plow_parked_{v:?}.raw")), bytes).unwrap();
            std::fs::write(dir.join(format!("plow_parked_{v:?}.txt")), format!("{} {}", canvas.width, canvas.height)).unwrap();
        }
    }

    /// Предел высоты кучи у снега при текущем размере куч.
    fn s_max() -> f32 {
        Kind::Snow.material().max_pile
    }
}
