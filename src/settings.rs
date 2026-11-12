//! Настройки пользователя: простой текстовый файл `ключ=значение`.

use std::fs;
use std::path::PathBuf;

/// Интенсивности снегопада: название и число снежинок в секунду на 1000 точек ширины.
pub const RATES: [(&str, f32); 3] = [("Лёгкий снежок", 15.0), ("Снегопад", 40.0), ("Метель", 110.0)];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    /// Индекс в `RATES`.
    pub rate: usize,
    /// Режим «не засыпать содержимое окон».
    pub solid: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { rate: 1, solid: false }
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
                ("solid", v) => s.solid = v == "1" || v == "true",
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
        let _ = fs::write(p, format!("rate={}\nsolid={}\n", self.rate, self.solid as u8));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_ignores_garbage() {
        assert_eq!(Settings::parse("rate=2\nsolid=1\n"), Settings { rate: 2, solid: true });
        assert_eq!(Settings::parse("rate=99\nчто-то\nsolid=0"), Settings { rate: 2, solid: false });
        assert_eq!(Settings::parse(""), Settings::default());
    }
}
