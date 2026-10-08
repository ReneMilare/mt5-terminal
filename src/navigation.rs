//! Calendar navigation uses the same server dates as the chart, without a timezone conversion.

use chrono::NaiveDate;

use crate::history::{Loader, Need};
use crate::model::{Store, Timeframe};

#[derive(Default)]
pub struct DateNavigation {
    pub input: String,
    pub target: Option<i64>,
    pub message: Option<String>,
}

impl DateNavigation {
    pub fn clear(&mut self) {
        self.target = None;
        self.message = None;
    }

    pub fn request(&mut self, today: NaiveDate) {
        let text = self.input.trim();
        let date = ["%d/%m/%Y", "%Y-%m-%d"].into_iter().find_map(|fmt| {
            NaiveDate::parse_from_str(text, fmt).ok().filter(|d| d.format(fmt).to_string() == text)
        });
        self.target = None;
        self.message = None;
        let Some(date) = date else {
            self.message = Some("Data inválida. Use DD/MM/AAAA.".into());
            return;
        };
        if date > today {
            self.message = Some("Escolha hoje ou uma data anterior.".into());
            return;
        }
        self.target = Some(date.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp());
    }

    /// Called each frame: returns a bar only once history covers the date or is exhausted.
    pub fn resolve(&mut self, store: &Store, loader: &Loader, symbol: &str, tf: Timeframe) -> Option<i64> {
        let time = self.target?;
        if !loader.ready(store, symbol, tf, Need::At { time }) {
            return None;
        }
        let bars = store.bars(symbol, tf);
        let next = bars.partition_point(|b| b.time < time);
        // A weekly candle can contain the chosen day even though it opens earlier.
        let index = if tf == Timeframe::W1 && next > 0 && time < bars[next - 1].time + tf.seconds() {
            next - 1
        } else {
            next.min(bars.len() - 1)
        };
        let bar_time = bars[index].time;
        let chosen = chrono::DateTime::from_timestamp(time, 0).unwrap().date_naive();
        let shown = chrono::DateTime::from_timestamp(bar_time, 0)?.date_naive();
        let covered = if tf == Timeframe::W1 {
            (bar_time..bar_time + tf.seconds()).contains(&time)
        } else {
            chosen == shown
        };
        self.message = Some(if covered {
            format!("{}", chosen.format("%d/%m/%Y"))
        } else {
            format!("Sem candles em {}. Exibindo {}.", chosen.format("%d/%m/%Y"), shown.format("%d/%m/%Y"))
        });
        self.target = None;
        Some(bar_time)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feed::Command;
    use crate::model::Bar;

    fn date(text: &str) -> NaiveDate {
        NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap()
    }

    fn time(text: &str) -> i64 {
        date(text).and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp()
    }

    fn put(store: &mut Store, tf: Timeframe, dates: &[&str]) {
        store.put("X", tf, dates.iter().map(|d| Bar {
            time: time(d), open: 1.0, high: 2.0, low: 0.5, close: 1.0, volume: 1.0,
        }).collect(), 2);
    }

    #[test]
    fn validates_calendar_dates_and_accepts_brazilian_and_iso_formats() {
        let mut nav = DateNavigation::default();
        for input in ["29/02/2024", " 2024-02-29 "] {
            nav.input = input.into();
            nav.request(date("2026-10-07"));
            assert_eq!(nav.target, Some(time("2024-02-29")));
            assert!(nav.message.is_none());
        }
        for input in ["", "29/02/2025", "31/04/2026", "07/10/26", "08/10/2026", "2026-13-01"] {
            nav.input = input.into();
            nav.request(date("2026-10-07"));
            assert!(nav.target.is_none() && nav.message.is_some(), "{input}");
        }
    }

    #[test]
    fn pages_until_target_then_resolves_once_and_can_cancel() {
        let (mut store, mut loader) = (Store::default(), Loader::default());
        let mut nav = DateNavigation { input: "01/10/2026".into(), ..Default::default() };
        nav.request(date("2026-10-07"));
        assert!(nav.resolve(&store, &loader, "X", Timeframe::M1).is_none());
        put(&mut store, Timeframe::M1, &["2026-10-05", "2026-10-06"]);
        let need = Need::At { time: nav.target.unwrap() };
        let Some(Command::History { before, .. }) = loader.older(&store, "X", Timeframe::M1, need) else { panic!() };
        assert!(loader.older(&store, "X", Timeframe::M1, need).is_none());
        assert!(nav.resolve(&store, &loader, "X", Timeframe::M1).is_none());
        put(&mut store, Timeframe::M1, &["2026-09-30", "2026-10-01"]);
        loader.on_bars("X", Timeframe::M1, before, false);
        assert_eq!(nav.resolve(&store, &loader, "X", Timeframe::M1), Some(time("2026-10-01")));
        assert!(nav.resolve(&store, &loader, "X", Timeframe::M1).is_none());
        nav.request(date("2026-10-07"));
        nav.clear();
        assert!(nav.resolve(&store, &loader, "X", Timeframe::M1).is_none());
    }

    #[test]
    fn missing_session_and_exhausted_history_show_available_bars() {
        let (mut store, mut loader) = (Store::default(), Loader::default());
        put(&mut store, Timeframe::D1, &["2026-10-02", "2026-10-05"]);
        let mut nav = DateNavigation { input: "04/10/2026".into(), ..Default::default() };
        nav.request(date("2026-10-07"));
        assert_eq!(nav.resolve(&store, &loader, "X", Timeframe::D1), Some(time("2026-10-05")));
        assert!(nav.message.as_ref().unwrap().contains("Sem candles"));
        nav.input = "01/01/2000".into();
        nav.request(date("2026-10-07"));
        assert!(nav.resolve(&store, &loader, "X", Timeframe::D1).is_none());
        loader.on_bars("X", Timeframe::D1, Some(time("2026-10-02")), true);
        assert_eq!(nav.resolve(&store, &loader, "X", Timeframe::D1), Some(time("2026-10-02")));
        nav.input = "07/10/2026".into();
        nav.request(date("2026-10-07"));
        assert_eq!(nav.resolve(&store, &loader, "X", Timeframe::D1), Some(time("2026-10-05")));
    }

    #[test]
    fn weekly_chart_selects_the_candle_containing_the_day() {
        let (mut store, loader) = (Store::default(), Loader::default());
        put(&mut store, Timeframe::W1, &["2026-09-28", "2026-10-05"]);
        let mut nav = DateNavigation { input: "01/10/2026".into(), ..Default::default() };
        nav.request(date("2026-10-07"));
        assert_eq!(nav.resolve(&store, &loader, "X", Timeframe::W1), Some(time("2026-09-28")));
        assert_eq!(nav.message.as_deref(), Some("01/10/2026"));
    }
}
