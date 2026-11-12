//! Местное время. Стандартная библиотека знает только UTC, поэтому часовой
//! пояс берётся у системы.

use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug)]
pub struct Clock {
    /// Местное время суток в часах: 0.0 ≤ hour < 24.0.
    pub hour: f32,
    /// Положение в году в месяцах: 0.0 — 1 января, 11.97 — 31 декабря.
    pub season: f32,
    /// Секунды с 1 января 1970 года по UTC (для фазы луны).
    pub unix: f64,
}

fn unix_now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

/// Запасной вариант, если система не сказала местное время: считаем по UTC.
#[cfg(unix)]
fn utc() -> Clock {
    let unix = unix_now();
    let day = (unix / 86_400.0).floor() as i64;
    let month = crate::materials::month_of_day(day);
    Clock { hour: ((unix % 86_400.0) / 3_600.0) as f32, season: (month - 1) as f32 + 0.5, unix }
}

#[cfg(unix)]
pub fn now() -> Clock {
    // SAFETY: `time` принимает нулевой указатель; `localtime_r` пишет только в
    // переданную нами структуру, которая живёт до конца вызова.
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&t, &mut tm).is_null() {
            return utc();
        }
        Clock {
            hour: tm.tm_hour as f32 + tm.tm_min as f32 / 60.0 + tm.tm_sec as f32 / 3_600.0,
            season: tm.tm_mon as f32 + (tm.tm_mday - 1) as f32 / 31.0,
            unix: unix_now(),
        }
    }
}

#[cfg(windows)]
pub fn now() -> Clock {
    use windows_sys::Win32::Foundation::SYSTEMTIME;
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;
    // SAFETY: функция пишет только в переданную структуру.
    let st = unsafe {
        let mut st: SYSTEMTIME = std::mem::zeroed();
        GetLocalTime(&mut st);
        st
    };
    Clock {
        hour: st.wHour as f32 + st.wMinute as f32 / 60.0 + st.wSecond as f32 / 3_600.0,
        season: (st.wMonth.max(1) - 1) as f32 + (st.wDay.max(1) - 1) as f32 / 31.0,
        unix: unix_now(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_is_in_range() {
        let c = now();
        assert!((0.0..24.0).contains(&c.hour), "hour = {}", c.hour);
        assert!((0.0..12.0).contains(&c.season), "season = {}", c.season);
        assert!(c.unix > 1.7e9);
    }
}
