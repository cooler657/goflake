//! Настройки пользователя: простой текстовый файл `ключ=значение`.

use std::fs;
use std::path::PathBuf;

use crate::materials::{KINDS, Kind};

/// Интенсивности: название и число снежинок в секунду на 1000 точек ширины
/// (для других материалов умножается на `Material::rate`).
pub const RATES: [(&str, f32); 3] = [("Слабо", 15.0), ("Умеренно", 40.0), ("Сильно", 110.0)];

/// Название пункта «выбирать осадки по календарю».
pub const SCENE_AUTO: &str = "По времени года";
/// Число вариантов в меню осадков: «по времени года» и по одному на материал.
pub const SCENES: usize = KINDS.len() + 1;

/// Название варианта в меню осадков.
pub fn scene_label(scene: usize) -> &'static str {
    if scene == 0 { SCENE_AUTO } else { KINDS[(scene - 1).min(KINDS.len() - 1)].material().label }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    /// Индекс в `RATES`.
    pub rate: usize,
    /// Что падает: 0 — по времени года, дальше — номер материала в `KINDS` плюс один.
    pub scene: usize,
    /// Режим «не засыпать содержимое окон».
    pub solid: bool,
    /// Аппаратное ускорение: рисовать на видеокарте, а не программно.
    pub gpu: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { rate: 1, scene: 0, solid: false, gpu: true }
    }
}

fn path() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("snowdrift").join("settings.conf"))
}

impl Settings {
    pub fn parse(text: &str) -> Self {
        let mut s = Self::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else { continue };
            match (key.trim(), value.trim()) {
                ("rate", v) => {
                    if let Ok(i) = v.parse::<usize>() {
                        s.rate = i.min(RATES.len() - 1);
                    }
                }
                ("scene", v) => {
                    if let Ok(i) = v.parse::<usize>() {
                        s.scene = i.min(SCENES - 1);
                    }
                }
                ("solid", v) => s.solid = v == "1" || v == "true",
                ("gpu", v) => s.gpu = v != "0" && v != "false",
                _ => {}
            }
        }
        s
    }

    pub fn load() -> Self {
        path().and_then(|p| fs::read_to_string(p).ok()).map(|t| Self::parse(&t)).unwrap_or_default()
    }

    pub fn save(&self) {
        let Some(p) = path() else { return };
        if let Some(dir) = p.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let text =
            format!("rate={}\nscene={}\nsolid={}\ngpu={}\n", self.rate, self.scene, self.solid as u8, self.gpu as u8);
        let _ = fs::write(p, text);
    }

    /// Материал, который должен падать сейчас.
    pub fn kind(&self) -> Kind {
        if self.scene == 0 { Kind::for_today() } else { KINDS[(self.scene - 1).min(KINDS.len() - 1)] }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_ignores_garbage() {
        assert_eq!(
            Settings::parse("rate=2\nscene=3\nsolid=1\ngpu=0\n"),
            Settings { rate: 2, scene: 3, solid: true, gpu: false }
        );
        assert_eq!(
            Settings::parse("rate=99\nчто-то\nscene=99\nsolid=0"),
            Settings { rate: 2, scene: 4, solid: false, gpu: true }
        );
        assert_eq!(Settings::parse(""), Settings::default());
    }

    #[test]
    fn scene_selects_material() {
        let s = |scene| Settings { scene, ..Settings::default() }.kind();
        assert_eq!(s(1), Kind::Snow);
        assert_eq!(s(2), Kind::Rain);
        assert_eq!(s(3), Kind::Sand);
        assert_eq!(s(4), Kind::Leaves);
        assert_eq!(s(0), Kind::for_today());
        assert_eq!(scene_label(0), SCENE_AUTO);
        assert_eq!(scene_label(4), Kind::Leaves.material().label);
    }
}
