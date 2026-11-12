//! Что падает с неба: снег, дождь, песок, листья.
//!
//! У всех материалов одна физика (`sim.rs`), различаются только числа в этой
//! таблице. Размеры заданы относительно снежинки: `scale` = 1 — это снег,
//! песчинка втрое мельче, лист в два с половиной раза крупнее.

/// Вид осадков. Первые четыре — по временам года, начиная с зимы; пыльная буря
/// по календарю не выбирается, только вручную.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Snow,
    Rain,
    Sand,
    Leaves,
    /// Марсианская пыльная буря.
    Dust,
}

pub const KINDS: [Kind; 5] = [Kind::Snow, Kind::Rain, Kind::Sand, Kind::Leaves, Kind::Dust];

/// Что стоит на верхней кромке окон при этом материале.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decor {
    /// Ничего.
    #[allow(dead_code)]
    None,
    /// Ёлки со снегом на ветках.
    Fir,
    /// Деревья с зелёной кроной и цветами.
    Leafy,
    /// Кактусы.
    Cactus,
    /// Голые деревья.
    Bare,
    /// Камни.
    Rocks,
}

/// Что получается, если курсором сгрести кучу этого материала.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gather {
    /// Ничего не собирается (вода).
    None,
    /// Шар, который катится за курсором; шары ставятся друг на друга — снеговик.
    Ball,
    /// Холмик, который сдвигается курсором; холмики сливаются.
    Heap,
}

/// Форма частицы при отрисовке.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// Круглая крупинка.
    Dot,
    /// Штрих, вытянутый вдоль скорости.
    Streak,
    /// Лист: заострённый овал с прожилкой, вращается.
    Leaf,
}

pub struct Material {
    /// Название в меню.
    pub label: &'static str,

    // ---- частица ----
    /// Размер относительно снежинки. Радиус частицы — от `scale` до `3.2 * scale` точек.
    pub scale: f32,
    /// Сколько массы несёт частица единичной площади (снег = 1).
    pub density: f32,
    /// Во сколько раз частиц больше, чем снежинок при той же интенсивности.
    pub rate: f32,
    /// Доля ближних частиц, которые ложатся на окна (остальные пролетают «за» ними).
    pub land_share: f32,

    // ---- полёт ----
    /// Предельная скорость падения: `fall_base + fall_size * (радиус / scale)`.
    pub fall_base: f32,
    pub fall_size: f32,
    /// Как быстро частица набирает предельную скорость.
    pub drag: f32,
    /// Насколько сильно сносит ветром.
    pub wind: f32,
    /// Размах и частота покачивания из стороны в сторону.
    pub sway: f32,
    pub sway_freq: f32,
    /// Порхание: доля, на которую колеблется скорость падения.
    pub flutter: f32,
    /// Наибольшая скорость вращения, радиан в секунду.
    pub spin: f32,
    /// Для штрихов: на сколько точек штрих удлиняется на каждую точку в секунду
    /// скорости. Чем больше, тем длиннее и «быстрее» выглядит штрих.
    pub streak: f32,
    /// Постоянный снос в одну сторону, точек в секунду до умножения на `wind`.
    /// У обычных материалов 0 — ветер просто качается туда-сюда; у бури он
    /// почти всегда дует вправо, порывами.
    pub wind_bias: f32,
    /// Доля частиц, которые рождаются не сверху, а у наветренного края экрана
    /// на любой высоте. Нужна при сильном боковом ветре: иначе нижняя часть
    /// экрана со стороны ветра остаётся пустой.
    pub side_spawn: f32,

    // ---- комки, сбитые с кучи ----
    pub debris_fall: f32,
    pub debris_drag: f32,
    pub debris_sway: f32,
    /// Масса одного комка и наибольшее число комков с одного столбика за раз.
    pub chunk: f32,
    pub max_chunks: usize,
    /// При падении разлетаются брызги.
    pub splash: bool,

    // ---- куча ----
    /// Наибольший перепад высот между соседними столбиками (угол осыпания).
    pub slope: f32,
    /// Сколько раз за кадр куча выравнивается (вода растекается быстрее).
    pub relax_passes: usize,
    /// Предельная высота кучи.
    pub max_pile: f32,
    /// Высота на краю окна, выше которой материал срывается вниз.
    pub edge_keep: f32,
    /// Убыль высоты в секунду (вода высыхает).
    pub evaporate: f32,

    // ---- внешний вид ----
    pub shape: Shape,
    /// Цвета частиц; каждая берёт случайный.
    pub colors: &'static [[f32; 3]],
    /// Общая прозрачность частиц.
    pub alpha: f32,
    pub pile_top: [f32; 4],
    pub pile_base: [f32; 4],
    /// Прозрачность тени над кромкой кучи.
    pub pile_shadow: f32,
    /// Пёстрая неровная куча (опавшие листья) вместо гладкой.
    pub pile_mottled: bool,
    /// Дымка на весь экран: цвет и наибольшая прозрачность (0 — дымки нет).
    /// Прозрачность медленно «дышит». Окна не затемняются: дымка за ними.
    pub haze: [f32; 4],
    /// Украшения на кромке окон.
    pub decor: Decor,
    /// Что получается, если сгрести кучу курсором.
    pub gather: Gather,
}

const SNOW: Material = Material {
    label: "Снег (зима)",
    scale: 1.0,
    density: 1.0,
    rate: 1.0,
    land_share: 0.7,
    fall_base: 28.0,
    fall_size: 20.0,
    drag: 3.0,
    wind: 1.0,
    sway: 14.0,
    sway_freq: 1.7,
    flutter: 0.0,
    spin: 0.0,
    debris_fall: 460.0,
    debris_drag: 2.2,
    debris_sway: 0.0,
    chunk: 5.0,
    max_chunks: 3,
    splash: false,
    slope: 1.4,
    relax_passes: 2,
    max_pile: 18.0,
    edge_keep: 2.0,
    evaporate: 0.0,
    shape: Shape::Dot,
    colors: &[[1.0, 1.0, 1.0]],
    alpha: 1.0,
    pile_top: [1.0, 1.0, 1.0, 1.0],
    pile_base: [0.84, 0.90, 1.0, 1.0],
    pile_shadow: 0.28,
    pile_mottled: false,
    wind_bias: 0.0,
    streak: 0.0,
    side_spawn: 0.0,
    haze: [0.0; 4],
    decor: Decor::Fir,
    gather: Gather::Ball,
};

/// Капли быстрые и тонкие; вода растекается ровным слоем, стекает с краёв и высыхает.
const RAIN: Material = Material {
    label: "Дождь (весна)",
    scale: 0.6,
    density: 1.6,
    rate: 4.0,
    land_share: 0.8,
    fall_base: 520.0,
    fall_size: 120.0,
    drag: 6.0,
    wind: 0.6,
    sway: 0.0,
    sway_freq: 0.0,
    flutter: 0.0,
    spin: 0.0,
    debris_fall: 700.0,
    debris_drag: 3.0,
    debris_sway: 0.0,
    chunk: 2.5,
    max_chunks: 3,
    splash: true,
    slope: 0.05,
    relax_passes: 6,
    max_pile: 4.0,
    edge_keep: 0.15,
    evaporate: 0.25,
    shape: Shape::Streak,
    colors: &[[0.74, 0.86, 1.0]],
    alpha: 0.6,
    pile_top: [0.70, 0.84, 1.0, 0.62],
    pile_base: [0.50, 0.70, 0.96, 0.62],
    pile_shadow: 0.0,
    pile_mottled: false,
    wind_bias: 0.0,
    streak: 0.007,
    side_spawn: 0.0,
    haze: [0.0; 4],
    decor: Decor::Leafy,
    gather: Gather::None,
};

/// Песчинки мелкие и тяжёлые, их сильно несёт ветром; куча держит крутой склон.
const SAND: Material = Material {
    label: "Песок (лето)",
    scale: 0.35,
    density: 3.0,
    rate: 3.5,
    land_share: 0.75,
    fall_base: 120.0,
    fall_size: 40.0,
    drag: 4.0,
    wind: 2.6,
    sway: 6.0,
    sway_freq: 3.0,
    flutter: 0.0,
    spin: 0.0,
    debris_fall: 520.0,
    debris_drag: 2.5,
    debris_sway: 0.0,
    chunk: 1.5,
    max_chunks: 6,
    splash: false,
    slope: 1.2,
    relax_passes: 2,
    max_pile: 24.0,
    edge_keep: 1.0,
    evaporate: 0.0,
    shape: Shape::Dot,
    colors: &[[0.93, 0.80, 0.55], [0.87, 0.72, 0.45], [0.96, 0.86, 0.64]],
    alpha: 1.0,
    pile_top: [0.94, 0.82, 0.58, 1.0],
    pile_base: [0.80, 0.64, 0.40, 1.0],
    pile_shadow: 0.25,
    pile_mottled: false,
    wind_bias: 0.0,
    streak: 0.0,
    side_spawn: 0.0,
    haze: [0.0; 4],
    decor: Decor::Cactus,
    gather: Gather::Heap,
};

/// Листья крупные и лёгкие: падают медленно, порхают и кружатся, лежат рыхлым слоем.
const LEAVES: Material = Material {
    label: "Листья (осень)",
    scale: 2.5,
    density: 0.12,
    rate: 0.3,
    land_share: 0.75,
    fall_base: 24.0,
    fall_size: 10.0,
    drag: 2.0,
    wind: 1.8,
    sway: 55.0,
    sway_freq: 1.1,
    flutter: 0.45,
    spin: 2.2,
    debris_fall: 120.0,
    debris_drag: 1.6,
    debris_sway: 35.0,
    chunk: 6.0,
    max_chunks: 2,
    splash: false,
    slope: 2.2,
    relax_passes: 2,
    max_pile: 12.0,
    edge_keep: 3.0,
    evaporate: 0.0,
    shape: Shape::Leaf,
    colors: &[
        [0.85, 0.33, 0.10],
        [0.93, 0.60, 0.12],
        [0.76, 0.20, 0.10],
        [0.90, 0.75, 0.20],
        [0.55, 0.33, 0.14],
    ],
    alpha: 1.0,
    pile_top: [0.90, 0.55, 0.15, 1.0],
    pile_base: [0.62, 0.34, 0.12, 1.0],
    pile_shadow: 0.25,
    pile_mottled: true,
    wind_bias: 0.0,
    streak: 0.0,
    side_spawn: 0.0,
    haze: [0.0; 4],
    decor: Decor::Bare,
    gather: Gather::Heap,
};

/// Пыль Марса: мелкая, быстрая и всегда боком. Ветер почти постоянный и порывистый,
/// пыль летит горизонтальными штрихами и залетает с левого края на любой высоте.
/// Оседает тонким рыжим слоем, дымка красит весь рабочий стол в оранжевый.
const DUST: Material = Material {
    label: "Пыльная буря (Марс)",
    scale: 0.55,
    density: 2.5,
    rate: 9.0,
    land_share: 0.7,
    fall_base: 22.0,
    fall_size: 14.0,
    drag: 2.5,
    wind: 5.0,
    sway: 45.0,
    sway_freq: 2.6,
    flutter: 0.0,
    spin: 0.0,
    debris_fall: 300.0,
    debris_drag: 2.0,
    debris_sway: 25.0,
    chunk: 1.5,
    max_chunks: 5,
    splash: false,
    slope: 0.8,
    relax_passes: 3,
    max_pile: 10.0,
    edge_keep: 0.8,
    evaporate: 0.0,
    shape: Shape::Streak,
    colors: &[[0.88, 0.48, 0.26], [0.80, 0.40, 0.22], [0.93, 0.58, 0.34], [0.72, 0.34, 0.20]],
    alpha: 0.85,
    pile_top: [0.84, 0.44, 0.24, 1.0],
    pile_base: [0.56, 0.26, 0.15, 1.0],
    pile_shadow: 0.2,
    pile_mottled: false,
    wind_bias: 45.0,
    streak: 0.035,
    side_spawn: 0.7,
    haze: [0.85, 0.42, 0.22, 0.18],
    decor: Decor::Rocks,
    gather: Gather::Heap,
};

impl Kind {
    pub fn material(self) -> &'static Material {
        match self {
            Kind::Snow => &SNOW,
            Kind::Rain => &RAIN,
            Kind::Sand => &SAND,
            Kind::Leaves => &LEAVES,
            Kind::Dust => &DUST,
        }
    }

    /// Осадки по времени года (северное полушарие), `month` — от 1 до 12.
    pub fn for_month(month: u32) -> Kind {
        match month {
            3..=5 => Kind::Rain,
            6..=8 => Kind::Sand,
            9..=11 => Kind::Leaves,
            _ => Kind::Snow,
        }
    }

    /// Осадки для сегодняшней даты по системным часам.
    pub fn for_today() -> Kind {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        Kind::for_month(month_of_day((secs / 86_400) as i64))
    }
}

/// Месяц (1–12) по числу дней от 1 января 1970 года.
pub(crate) fn month_of_day(days: i64) -> u32 {
    // алгоритм civil_from_days (Говард Хиннант)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let _ = era;
    (if mp < 10 { mp + 3 } else { mp - 9 }) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn months_from_days() {
        assert_eq!(month_of_day(0), 1); // 1 января 1970
        assert_eq!(month_of_day(58), 2); // 28 февраля 1970
        assert_eq!(month_of_day(59), 3); // 1 марта 1970
        assert_eq!(month_of_day(11_016), 2); // 29 февраля 2000
        assert_eq!(month_of_day(20_769), 11); // 12 ноября 2026
        assert_eq!(month_of_day(20_818), 12); // 31 декабря 2026
    }

    #[test]
    fn seasons() {
        assert_eq!(Kind::for_month(1), Kind::Snow);
        assert_eq!(Kind::for_month(4), Kind::Rain);
        assert_eq!(Kind::for_month(7), Kind::Sand);
        assert_eq!(Kind::for_month(10), Kind::Leaves);
        assert_eq!(Kind::for_month(12), Kind::Snow);
        // пыльная буря по календарю не выбирается ни в одном месяце
        assert!((1..=12).all(|m| Kind::for_month(m) != Kind::Dust));
    }

    #[test]
    fn sizes_are_ordered_relative_to_snow() {
        let s = |k: Kind| k.material().scale;
        assert!(s(Kind::Sand) < s(Kind::Rain));
        assert!(s(Kind::Rain) < s(Kind::Snow));
        assert!(s(Kind::Snow) < s(Kind::Leaves));
        // пыль бури крупнее песчинки, но мельче капли
        assert!(s(Kind::Sand) < s(Kind::Dust) && s(Kind::Dust) < s(Kind::Rain));
        assert_eq!(s(Kind::Snow), 1.0);
    }
}
