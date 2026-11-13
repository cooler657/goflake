//! Уборщик (трактор, дворник с метлой, пингвин с лопатой, пикап или снегоуборщик),
//! который время от времени выезжает на кромку панели задач или Dock и сгребает с неё кучу.
//!
//! Едет вдоль всей кромки (в каждый раз в другую сторону), снимая столбики кучи
//! под лопатой. Снятое уборщик везёт с собой, а в конце пути сбрасывает за край
//! экрана комками; по дороге от лопаты летят лишь брызги для вида (без массы).
//! Если Dock или панели задач нет, чистится низ экрана. Выезжает, когда средняя
//! высота кучи достигает четверти предела, и отдыхает `Plow::every` секунд между выездами.

use super::*;
use crate::decor::{bar, circle};
use crate::look::FlakeLook;

/// Сколько фигур трактор может отдать за кадр (под них запасён буфер на GPU).
pub const PLOW_MAX: usize = 32;
/// Скорость, точек в секунду.
const SPEED: f32 = 90.0;
/// При какой средней высоте кучи (доля предела) уборщик выезжает.
const TRIGGER: f32 = 0.25;
/// Длина машины вместе с лопатой, точек.
const LEN: f32 = 32.0;
/// Расстояние от центра кузова до передней кромки лопаты, точек.
const FRONT: f32 = 16.8;

struct Run {
    surface: i64,
    /// Передняя кромка лопаты.
    x: f32,
    /// Масса, которую лопата уже сгребла и везёт.
    load: f32,
}

/// Состояние уборщика. Живёт в `Sim::plow`.
pub struct Plow {
    /// Пауза между выездами, секунд; 0 — уборщик выключен.
    pub every: f32,
    run: Option<Run>,
    /// Сколько секунд осталось отдыхать.
    wait: f32,
    /// Куда едет (или поедет в следующий раз): 1 — вправо, -1 — влево.
    dir: f32,
    /// Кто чистит.
    pub vehicle: Vehicle,
}

impl Default for Plow {
    fn default() -> Self {
        Self { every: 0.0, run: None, wait: 0.0, dir: 1.0, vehicle: Vehicle::Tractor }
    }
}

impl Plow {
    /// Меняет частоту выездов. Если паузу сократили, отдыхать остаётся не дольше новой.
    pub fn set_every(&mut self, every: f32) {
        self.every = every;
        self.wait = self.wait.min(every);
    }
}

impl Sim {
    /// Шаг уборщика: выезд, движение, снятие столбиков под лопатой.
    pub(super) fn plow_step(&mut self, dt: f32) {
        if self.plow.every <= 0.0 {
            self.plow.run = None;
            return;
        }
        let mat = self.kind.material();
        let Some(mut run) = self.plow.run.take() else {
            self.plow.wait -= dt;
            if self.plow.wait > 0.0 {
                return;
            }
            let id = if self.surfaces.contains_key(&DOCK) { DOCK } else { GROUND };
            let Some(s) = self.surfaces.get(&id) else { return };
            let (sum, n) = s.h.iter().zip(&s.vis).filter(|(_, v)| **v).fold((0.0, 0), |(a, n), (h, _)| (a + h, n + 1));
            if n == 0 || sum / (n as f32) < TRIGGER * mat.max_pile * self.pile_scale {
                return;
            }
            let (lo, hi) = (s.x0, s.x0 + s.h.len() as f32 * COL_W);
            self.plow.dir = -self.plow.dir;
            let x = if self.plow.dir > 0.0 { lo - LEN } else { hi + LEN };
            self.plow.run = Some(Run { surface: id, x, load: 0.0 });
            return;
        };

        let dir = self.plow.dir;
        let prev = run.x;
        run.x += dir * SPEED * dt;
        let Some(s) = self.surfaces.get_mut(&run.surface) else {
            self.plow.wait = self.plow.every; // кромка исчезла — выезд отменён, груз пропадает
            return;
        };
        let (lo, hi) = (s.x0, s.x0 + s.h.len() as f32 * COL_W);
        let (a, b) = if prev < run.x { (prev, run.x) } else { (run.x, prev) };
        for i in 0..s.h.len() {
            let cx = lo + (i as f32 + 0.5) * COL_W;
            // весь след за лопатой остаётся чистым: иначе куча впереди «стекала» бы на
            // очищенные столбики (сглаживание склонов) и оставляла за лопатой хвост
            let behind = if dir > 0.0 { cx < run.x } else { cx > run.x };
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
        let done = if dir > 0.0 { run.x > hi + LEN } else { run.x < lo - LEN };
        if done {
            // в конце пути груз сброшен за край экрана
            let k = ((run.load / mat.chunk) as usize).clamp(1, 40);
            let ex = if dir > 0.0 { hi - 1.0 } else { lo + 1.0 };
            for _ in 0..k {
                let py = s.y - self.rng.range(0.0, 6.0);
                let vx = dir * self.rng.range(60.0, 140.0);
                let vy = -self.rng.range(30.0, 90.0);
                debris(&mut self.particles, &mut self.rng, self.kind, false, ex, py, vx, vy, run.load / k as f32);
            }
            self.plow.wait = self.plow.every;
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

/// Дописывает в `out` фигуры уборщика, если он сейчас в пути.
pub fn looks(sim: &Sim, out: &mut Vec<FlakeLook>) {
    let Some(run) = &sim.plow.run else { return };
    let Some(s) = sim.surfaces.get(&run.surface) else { return };
    let dir = sim.plow.dir;
    let start = out.len();
    let g = Pose { ox: run.x - dir * FRONT, dir, base: s.y, t: sim.t };
    match sim.plow.vehicle {
        Vehicle::Tractor => tractor(&g, out),
        Vehicle::Janitor => janitor(&g, out),
        Vehicle::Penguin => penguin(&g, out),
        Vehicle::Pickup => pickup(&g, out),
        Vehicle::Blower => blower(&g, out),
    }
    out.truncate(start + PLOW_MAX);
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

    #[test]
    fn plow_clears_a_big_pile_and_then_rests() {
        let mut s = sim_with_pile(0.8 * s_max());
        s.plow.every = 30.0;
        let before = mean(&s);
        let total0 = before * 500.0 * COL_W;
        let mut seen_looks = false;
        let mut finished = false;
        for _ in 0..(60 * 30) {
            s.step(1.0 / 60.0, (-500.0, -500.0), (0.0, 0.0));
            let mut shapes = Vec::new();
            looks(&s, &mut shapes);
            assert!(shapes.len() <= PLOW_MAX);
            assert!(shapes.iter().all(|f| f.center[0].is_finite() && f.center[1] <= 760.0 + 1.0), "техника ушла под кромку");
            seen_looks |= !shapes.is_empty();
            if seen_looks && s.plow.run.is_none() {
                finished = true;
                break;
            }
        }
        assert!(seen_looks, "уборщик не выехал");
        assert!(finished, "уборщик не закончил выезд");
        assert!(mean(&s) < before * 0.3, "куча осталась: {} из {before}", mean(&s));
        // отдыхает, а не едет сразу снова
        assert_eq!(s.plow.wait, 30.0);
        // масса сохранена: всё, что лопата сняла, сброшено в конце пути комками
        let kept = s.particles.iter().map(|p| p.mass).sum::<f32>() + mean(&s) * 500.0 * COL_W;
        assert!((kept - total0).abs() < 0.03 * total0, "масса: было {total0}, стало {kept}");
    }

    #[test]
    fn plow_leaves_small_piles_and_obeys_the_switch() {
        let mut s = sim_with_pile(0.05 * s_max());
        s.plow.every = 30.0;
        for _ in 0..(60 * 5) {
            s.step(1.0 / 60.0, (-500.0, -500.0), (0.0, 0.0));
        }
        assert!(s.plow.run.is_none(), "выехал к почти пустой кромке");

        let mut s = sim_with_pile(0.8 * s_max());
        for _ in 0..(60 * 5) {
            s.step(1.0 / 60.0, (-500.0, -500.0), (0.0, 0.0));
        }
        assert!(s.plow.run.is_none(), "выехал при выключенном уборщике");
        assert!(mean(&s) > 0.5 * s_max());
    }

    #[test]
    fn every_vehicle_is_drawn_in_front_of_its_blade_in_both_directions() {
        for v in VEHICLES {
            for dir in [1.0f32, -1.0] {
                let mut s = sim_with_pile(1.0);
                s.plow.vehicle = v;
                s.plow.dir = dir;
                s.plow.run = Some(Run { surface: DOCK, x: 500.0, load: 0.0 });
                let mut shapes = Vec::new();
                looks(&s, &mut shapes);
                assert!(shapes.len() > 5 && shapes.len() <= PLOW_MAX, "{v:?}: {} фигур", shapes.len());
                for f in &shapes {
                    assert!(f.center[0].is_finite() && f.center[1].is_finite() && f.color[3] <= 1.0);
                    // стоит на кромке (панель задач начинается на 760), не проваливается под неё
                    assert!(f.center[1] <= 760.0 + 0.1 && f.center[1] >= 760.0 - 22.0, "{v:?}: {:?}", f.center);
                    // сзади от лопаты, не дальше длины машины; спереди — не дальше самой лопаты
                    let ahead = dir * (f.center[0] - 500.0);
                    assert!(ahead <= 0.5 && ahead >= -(LEN + 2.0), "{v:?}, {dir}: вынос {ahead}");
                }
            }
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
    /// `SNOWDRIFT_DUMP=/папка cargo test dump_vehicles -- --ignored`; формат — как у
    /// `soft::tests::dump_preview` (`имя.raw` и `имя.txt`).
    #[test]
    #[ignore]
    fn dump_vehicles() {
        use crate::clock::Clock;
        use crate::sky::{Sky, SkyMode};
        let Some(dir) = std::env::var_os("SNOWDRIFT_DUMP") else { return };
        let dir = std::path::PathBuf::from(dir);
        for v in VEHICLES {
            let mut s = sim_with_pile(0.0);
            s.w = 120.0;
            s.h = 60.0;
            s.refresh(&[], Some(Rect::new(0.0, 44.0, 120.0, 16.0)), 0.05);
            s.plow.vehicle = v;
            s.plow.run = Some(Run { surface: DOCK, x: 90.0, load: 0.0 });
            s.t = 0.3;
            let mut canvas = crate::soft::Canvas::new();
            let sky = Sky::new(SkyMode::Off, Clock { hour: 12.0, season: 5.5, unix: 0.0 });
            canvas.draw(&s, &sky, 8.0);
            let bytes: Vec<u8> = canvas.pixels.iter().flat_map(|p| p.to_le_bytes()).collect();
            std::fs::write(dir.join(format!("plow_{v:?}.raw")), bytes).unwrap();
            std::fs::write(dir.join(format!("plow_{v:?}.txt")), format!("{} {}", canvas.width, canvas.height)).unwrap();
        }
    }

    /// Предел высоты кучи у снега при текущем размере куч.
    fn s_max() -> f32 {
        Kind::Snow.material().max_pile
    }
}
