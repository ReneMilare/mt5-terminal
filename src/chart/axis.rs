//! Axis math: "nice" price steps, time label steps and label text. No egui here, so it is testable.

use chrono::{DateTime, Datelike, NaiveDateTime, Timelike};

/// Smallest "nice" step (1, 2, 2.5, 5 × 10^k) not below `raw`.
pub fn nice_step(raw: f64) -> f64 {
    if !(raw.is_finite() && raw > 0.0) {
        return 1.0;
    }
    let mag = 10f64.powf(raw.log10().floor());
    for m in [1.0, 2.0, 2.5, 5.0, 10.0] {
        if m * mag >= raw * (1.0 - 1e-9) {
            return m * mag;
        }
    }
    10.0 * mag
}

/// Price ticks inside `[lo, hi]` for a step of `step`.
pub fn price_ticks(lo: f64, hi: f64, step: f64) -> impl Iterator<Item = f64> {
    let first = (lo / step).ceil() as i64;
    let last = (hi / step).floor() as i64;
    (first..=last).map(move |k| k as f64 * step)
}

/// Decimals needed to print `step` exactly, at least `digits`.
pub fn decimals_for(step: f64, digits: u32) -> usize {
    let mut d = 0usize;
    while d < 8 && ((step * 10f64.powi(d as i32)).round() - step * 10f64.powi(d as i32)).abs() > 1e-6 {
        d += 1;
    }
    d.max(digits.min(8) as usize).min(8)
}

/// Candidate spacings between time labels, in seconds.
const TIME_STEPS: [i64; 14] =
    [60, 300, 900, 1800, 3600, 7200, 10_800, 14_400, 21_600, 43_200, 86_400, 172_800, 604_800, 2_592_000];

/// Label step for an average of `secs_per_px` seconds per pixel, wanting ~`min_px` between labels.
pub fn time_step(secs_per_px: f64, min_px: f64) -> i64 {
    let want = secs_per_px * min_px;
    TIME_STEPS.iter().copied().find(|&s| s as f64 >= want).unwrap_or(2_592_000)
}

pub fn naive(time: i64) -> NaiveDateTime {
    DateTime::from_timestamp(time, 0).map(|d| d.naive_utc()).unwrap_or_default()
}

/// Bucket used to decide where a label goes: a label is placed at the first bar of each bucket.
/// Days and months follow the calendar; smaller steps follow the clock.
pub fn time_bucket(time: i64, step: i64) -> i64 {
    let t = naive(time);
    match step {
        s if s >= 2_592_000 => (t.year() as i64) * 12 + t.month0() as i64,
        s if s >= 604_800 => t.date().iso_week().week() as i64 + 100 * t.iso_week().year() as i64,
        s if s >= 86_400 => t.date().num_days_from_ce() as i64 / (s / 86_400),
        s => time.div_euclid(s),
    }
}

const MONTHS: [&str; 12] = ["Jan", "Fev", "Mar", "Abr", "Mai", "Jun", "Jul", "Ago", "Set", "Out", "Nov", "Dez"];
const WEEKDAYS: [&str; 7] = ["Seg", "Ter", "Qua", "Qui", "Sex", "Sáb", "Dom"];

/// Text for a time label; `major` when the day (or month) changed, which is drawn stronger.
pub fn time_label(time: i64, prev: Option<i64>, step: i64) -> (String, bool) {
    let t = naive(time);
    let new_day = prev.is_none_or(|p| naive(p).date() != t.date());
    let new_month = prev.is_none_or(|p| naive(p).month() != t.month());
    if step >= 2_592_000 || (step >= 86_400 && new_month) {
        if t.month0() == 0 {
            return (t.year().to_string(), true);
        }
        return (MONTHS[t.month0() as usize].to_string(), true);
    }
    if step >= 86_400 || new_day {
        return (format!("{:02} {}", t.day(), MONTHS[t.month0() as usize]), true);
    }
    (format!("{:02}:{:02}", t.hour(), t.minute()), false)
}

/// Full text for the crosshair time box.
pub fn crosshair_label(time: i64, intraday: bool) -> String {
    let t = naive(time);
    let wd = WEEKDAYS[t.weekday().num_days_from_monday() as usize];
    if intraday {
        format!("{wd} {:02} {} {:02}  {:02}:{:02}", t.day(), MONTHS[t.month0() as usize], t.year() % 100, t.hour(), t.minute())
    } else {
        format!("{wd} {:02} {} {:02}", t.day(), MONTHS[t.month0() as usize], t.year() % 100)
    }
}

/// "mm:ss" or "hh:mm:ss" until the bar closes.
pub fn countdown(secs: i64) -> String {
    let s = secs.max(0);
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{:02}:{:02}", s / 60, s % 60) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nice_steps() {
        assert_eq!(nice_step(0.8), 1.0);
        assert_eq!(nice_step(1.3), 2.0);
        assert_eq!(nice_step(2.2), 2.5);
        assert_eq!(nice_step(3.0), 5.0);
        assert_eq!(nice_step(7.0), 10.0);
        assert_eq!(nice_step(12.0), 20.0);
        assert_eq!(nice_step(0.02), 0.02);
    }

    #[test]
    fn ticks_and_decimals() {
        let t: Vec<f64> = price_ticks(30_995.0, 31_052.0, 25.0).collect();
        assert_eq!(t, vec![31_000.0, 31_025.0, 31_050.0]);
        assert_eq!(decimals_for(25.0, 2), 2);
        assert_eq!(decimals_for(0.25, 0), 2);
        assert_eq!(decimals_for(0.005, 2), 3);
    }

    #[test]
    fn time_labels() {
        // 2026-10-05 14:35 UTC (a Monday)
        let t = 1_791_211_700 - 1_791_211_700 % 300;
        assert_eq!(time_step(30.0, 90.0), 3600);
        let (txt, major) = time_label(t, Some(t - 300), 3600);
        assert!(!major && txt.contains(':'));
        let midnight = t - t % 86_400;
        let (txt, major) = time_label(midnight, Some(midnight - 300), 3600);
        assert!(major && txt.ends_with("Out"), "{txt}");
        assert!(crosshair_label(t, true).starts_with("Seg 05 Out 26"));
        assert_eq!(countdown(75), "01:15");
        assert_eq!(countdown(3725), "1:02:05");
    }

    #[test]
    fn buckets_follow_calendar() {
        let day = 86_400;
        let t = 1_791_211_700 - 1_791_211_700 % day;
        assert_ne!(time_bucket(t - 1, day), time_bucket(t, day));
        assert_eq!(time_bucket(t, day), time_bucket(t + 3600, day));
        assert_eq!(time_bucket(t, 3600), time_bucket(t + 1800, 3600));
    }
}
