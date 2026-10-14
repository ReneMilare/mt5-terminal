//! Calendar and clocks shared by the indicators: server time (ActivTrades: UTC+1 standard, EU daylight
//! saving), New York time, NYSE sessions and holidays. Port of `ContextMath.mqh`/`ContextRuntime.mqh`
//! with the preset's inputs (`InpServerDST=1`, `InpServerStandardMinutes=60`, no overrides).
//! All times are seconds; "day" values are midnights of the clock they belong to.

use chrono::{DateTime, Datelike, NaiveDate};

pub const DAY: i64 = 86_400;
pub const WEEK: i64 = 7 * DAY;

/// Server daylight rule: 1 = EU (last Sunday of March to last Sunday of October).
const SERVER_DST: u8 = 1;
const SERVER_STANDARD: i64 = 60 * 60;
const US: u8 = 2;
const NY_STANDARD: i64 = -18_000;

pub fn date(y: i32, m: u32, d: u32) -> i64 {
    NaiveDate::from_ymd_opt(y, m, d).map(|d| d.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp()).unwrap_or(0)
}

pub fn day(t: i64) -> i64 {
    t - t.rem_euclid(DAY)
}

/// 0 = Sunday … 6 = Saturday (1970-01-01 was a Thursday).
pub fn weekday(t: i64) -> i64 {
    (t.div_euclid(DAY) + 4).rem_euclid(7)
}

pub fn ymd(t: i64) -> (i32, u32, u32) {
    let d = DateTime::from_timestamp(t, 0).unwrap_or_default().date_naive();
    (d.year(), d.month(), d.day())
}

/// n-th `weekday` of a month; n = 5 means the last one.
fn nth(y: i32, m: u32, wd: i64, n: i64) -> i64 {
    let first = date(y, m, 1);
    let mut t = first + (wd - weekday(first) + 7) % 7 * DAY;
    if n < 5 {
        return t + (n - 1) * WEEK;
    }
    while ymd(t + WEEK).1 == m {
        t += WEEK;
    }
    t
}

fn dst(utc: i64, rule: u8, standard: i64) -> bool {
    let y = ymd(utc).0;
    match rule {
        1 => utc >= nth(y, 3, 0, 5) + 3600 && utc < nth(y, 10, 0, 5) + 3600,
        2 => utc >= nth(y, 3, 0, 2) + 7200 - standard && utc < nth(y, 11, 0, 1) + 3600 - standard,
        _ => false,
    }
}

fn observed(d: i64) -> i64 {
    match weekday(d) {
        6 => d - DAY,
        0 => d + DAY,
        _ => d,
    }
}

fn easter(y: i32) -> i64 {
    let (a, b, c) = (y % 19, y / 100, y % 100);
    let (d, e, f) = (b / 4, b % 4, (b + 8) / 25);
    let g = (b - f + 1) / 3;
    let h = (19 * a + b - d - g + 15) % 30;
    let (i, k) = (c / 4, c % 4);
    let l = (32 + 2 * e + 2 * i - h - k) % 7;
    let m = (a + 11 * h + 22 * l) / 451;
    let v = h + l - 7 * m + 114;
    date(y, (v / 31) as u32, (v % 31 + 1) as u32)
}

/// NYSE close in minutes after New York midnight for a New York day; 0 = closed. Valid 2024–2028.
pub fn session_close(nyday: i64) -> i64 {
    let (y, mon, dd) = ymd(nyday);
    let wd = weekday(nyday);
    if wd == 0 || wd == 6 {
        return 0;
    }
    let newyear = date(y, 1, 1);
    let holiday = nyday == newyear
        || (weekday(newyear) == 0 && nyday == newyear + DAY)
        || nyday == nth(y, 1, 1, 3)
        || nyday == nth(y, 2, 1, 3)
        || nyday == easter(y) - 2 * DAY
        || nyday == nth(y, 5, 1, 5)
        || (y >= 2022 && nyday == observed(date(y, 6, 19)))
        || nyday == observed(date(y, 7, 4))
        || nyday == nth(y, 9, 1, 1)
        || nyday == nth(y, 11, 4, 4)
        || nyday == observed(date(y, 12, 25))
        || nyday == date(2025, 1, 9)
        || nyday == date(2018, 12, 5)
        || nyday == date(2012, 10, 29)
        || nyday == date(2012, 10, 30);
    if holiday {
        return 0;
    }
    if nyday == nth(y, 11, 4, 4) + DAY || (mon == 12 && dd == 24) || (mon == 7 && dd == 3) {
        return 780;
    }
    960
}

pub fn utc_of_server(server: i64) -> i64 {
    let utc = server - SERVER_STANDARD;
    utc - if dst(utc - 3600, SERVER_DST, SERVER_STANDARD) { 3600 } else { 0 }
}

pub fn server_of_utc(utc: i64) -> i64 {
    utc + SERVER_STANDARD + if dst(utc, SERVER_DST, SERVER_STANDARD) { 3600 } else { 0 }
}

/// New York wall clock of a server time.
pub fn ny(server: i64) -> i64 {
    let utc = utc_of_server(server);
    utc - 18_000 + if dst(utc, US, NY_STANDARD) { 3600 } else { 0 }
}

/// Server time of a New York wall clock (never the ambiguous 01:00 of transition Sundays).
pub fn ny_server(ny: i64) -> i64 {
    let mut utc = ny + 18_000;
    if dst(utc, US, NY_STANDARD) {
        utc -= 3600;
    }
    server_of_utc(utc)
}

/// Previous New York day with a session.
pub fn previous_session(mut nyday: i64) -> i64 {
    for _ in 0..14 {
        nyday -= DAY;
        if session_close(nyday) > 0 {
            return nyday;
        }
    }
    0
}

/// Server minus UTC, as the GEX exporter's `GexServerUtcOffset` (whole quarter hours).
pub fn server_utc_offset(utc: i64) -> i64 {
    server_of_utc(utc) - utc
}

/// Monday that starts the server week of `d` (Sunday belongs to the next week).
pub fn week_start(d: i64) -> i64 {
    let wd = weekday(d);
    if wd == 0 { d + DAY } else { d - (wd - 1) * DAY }
}

pub fn month_start(d: i64) -> i64 {
    let (y, m, _) = ymd(d);
    date(y, m, 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calendar() {
        assert_eq!(weekday(date(2026, 10, 5)), 1); // Monday
        assert_eq!(nth(2026, 3, 0, 5), date(2026, 3, 29)); // last Sunday of March
        assert_eq!(nth(2026, 11, 4, 4), date(2026, 11, 26)); // Thanksgiving
        assert_eq!(easter(2026), date(2026, 4, 5));
        assert_eq!(session_close(date(2026, 4, 3)), 0); // Good Friday
        assert_eq!(session_close(date(2026, 11, 27)), 780);
        assert_eq!(session_close(date(2026, 10, 5)), 960);
        assert_eq!(previous_session(date(2026, 10, 5)), date(2026, 10, 2));
    }

    #[test]
    fn clocks() {
        // 2026-10-05 13:30 UTC: server (EU summer) 16:30, New York (EDT) 09:30
        let utc = date(2026, 10, 5) + 13 * 3600 + 1800;
        let server = server_of_utc(utc);
        assert_eq!(server - utc, 7200);
        assert_eq!(utc_of_server(server), utc);
        assert_eq!(ny(server), date(2026, 10, 5) + 9 * 3600 + 1800);
        assert_eq!(ny_server(date(2026, 10, 5) + 570 * 60), server);
        // January: server UTC+1, New York UTC-5 -> 6 h apart
        let jan = date(2026, 1, 15) + 15 * 3600;
        assert_eq!(server_of_utc(jan) - jan, 3600);
        assert_eq!(ny(server_of_utc(jan)), jan - 5 * 3600);
        assert_eq!(week_start(date(2026, 10, 7)), date(2026, 10, 5));
        assert_eq!(week_start(date(2026, 10, 4)), date(2026, 10, 5));
        assert_eq!(month_start(date(2026, 10, 7) + 5000), date(2026, 10, 1));
    }
}
