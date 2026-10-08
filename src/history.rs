//! History is loaded in pieces: a small first chunk per series so the chart paints at once, then older
//! chunks only when something needs them (the view scrolled or zoomed back, the indicators' warm-up,
//! the level map's month). One request in flight per series; an empty answer means no older history.

use crate::feed::Command;
use crate::model::{Store, Timeframe};
use std::collections::HashMap;

/// Bars per older chunk the view asks for while scrolling back.
pub const CHUNK: u32 = 2000;
/// Bars per chunk of background warm-up (fewer round trips, still far from loading everything).
pub const WARM_CHUNK: u32 = 5000;

/// What a series must cover.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Need {
    /// One more chunk, whatever is loaded (the view reached the oldest bar).
    More,
    /// Cover an absolute server time, for navigation to a calendar date.
    At { time: i64 },
    /// At least this many bars, including the forming bar, regardless of session gaps.
    #[cfg_attr(not(has_preset), allow(dead_code))] // Optional presets request exact bar counts.
    Bars { count: usize },
    /// From `span` seconds before the newest bar, or at least `cap` bars, whichever comes first.
    Since { span: i64, cap: usize },
}

#[derive(Default)]
struct State {
    /// `Some(before)` while a request is pending (`Some(None)` = the first, newest chunk).
    in_flight: Option<Option<i64>>,
    exhausted: bool,
}

#[derive(Default)]
pub struct Loader {
    states: HashMap<(String, Timeframe), State>,
}

impl Loader {
    pub fn clear(&mut self) {
        self.states.clear();
    }

    /// First request of a series: its newest `count` bars. None if it is already loading or loaded.
    pub fn first(&mut self, store: &Store, symbol: &str, tf: Timeframe, count: u32) -> Option<Command> {
        let st = self.states.entry((symbol.to_string(), tf)).or_default();
        if st.in_flight.is_some() || store.get(symbol, tf).is_some_and(|s| !s.bars.is_empty()) {
            return None;
        }
        st.in_flight = Some(None);
        Some(Command::History { symbol: symbol.into(), tf, count, before: None })
    }

    fn satisfied(store: &Store, symbol: &str, tf: Timeframe, need: Need) -> bool {
        let bars = store.bars(symbol, tf);
        match need {
            Need::More => false,
            Need::At { time } => bars.first().is_some_and(|b| b.time <= time),
            Need::Bars { count } => bars.len() >= count,
            Need::Since { span, cap } => match (bars.first(), bars.last()) {
                (Some(first), Some(last)) => bars.len() >= cap || first.time <= last.time - span,
                _ => false,
            },
        }
    }

    /// Request older bars if `need` is not met and nothing is pending for the series.
    pub fn older(&mut self, store: &Store, symbol: &str, tf: Timeframe, need: Need) -> Option<Command> {
        let first = store.bars(symbol, tf).first()?.time;
        let st = self.states.entry((symbol.to_string(), tf)).or_default();
        if st.exhausted || st.in_flight.is_some() || Self::satisfied(store, symbol, tf, need) {
            return None;
        }
        st.in_flight = Some(Some(first));
        let count = match need {
            Need::More => CHUNK,
            Need::At { time } => ((first - time).div_euclid(tf.seconds()) + 1)
                .clamp(1, WARM_CHUNK as i64) as u32,
            Need::Bars { count } => count
                .saturating_sub(store.bars(symbol, tf).len())
                .min(WARM_CHUNK as usize) as u32,
            Need::Since { .. } => WARM_CHUNK };
        Some(Command::History { symbol: symbol.into(), tf, count, before: Some(first) })
    }

    /// An answer arrived. Resyncs of the newest bars don't touch an older request in flight.
    pub fn on_bars(&mut self, symbol: &str, tf: Timeframe, before: Option<i64>, empty: bool) {
        let st = self.states.entry((symbol.to_string(), tf)).or_default();
        if st.in_flight == Some(before) {
            st.in_flight = None;
        }
        if before.is_some() && empty {
            st.exhausted = true;
        }
    }

    /// The series covers `need`, or no older history exists.
    pub fn ready(&self, store: &Store, symbol: &str, tf: Timeframe, need: Need) -> bool {
        let st = self.states.get(&(symbol.to_string(), tf));
        let loaded = store.get(symbol, tf).is_some_and(|s| !s.bars.is_empty());
        loaded && (st.is_some_and(|s| s.exhausted) || Self::satisfied(store, symbol, tf, need))
    }

    pub fn loading(&self) -> bool {
        self.states.values().any(|s| s.in_flight.is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Bar;

    fn bars(from: i64, n: i64) -> Vec<Bar> {
        (0..n).map(|i| Bar { time: (from + i) * 60, open: 1.0, high: 1.0, low: 1.0, close: 1.0, volume: 1.0 }).collect()
    }

    #[test]
    fn bar_count_requests_only_missing_history_across_session_gaps() {
        let (mut store, mut loader) = (Store::default(), Loader::default());
        store.put("X", Timeframe::D1, bars(1000, 300), 2);
        let need = Need::Bars { count: 301 };
        assert!(!loader.ready(&store, "X", Timeframe::D1, need));
        let Some(Command::History { count, before, .. }) =
            loader.older(&store, "X", Timeframe::D1, need)
        else {
            panic!()
        };
        assert_eq!(count, 1);
        assert!(loader.older(&store, "X", Timeframe::D1, need).is_none());
        store.put("X", Timeframe::D1, bars(900, 1), 2);
        loader.on_bars("X", Timeframe::D1, before, false);
        assert!(loader.ready(&store, "X", Timeframe::D1, need));
        assert!(loader.older(&store, "X", Timeframe::D1, need).is_none());
    }

    #[test]
    fn pages_back_until_covered_or_exhausted() {
        let (mut store, mut l) = (Store::default(), Loader::default());
        let tf = Timeframe::M1;
        assert!(matches!(l.first(&store, "X", tf, 100), Some(Command::History { before: None, .. })));
        assert!(l.first(&store, "X", tf, 100).is_none(), "one request in flight");
        store.put("X", tf, bars(1000, 100), 2);
        l.on_bars("X", tf, None, false);

        let need = Need::Since { span: 150 * 60, cap: 10_000 };
        assert!(!l.ready(&store, "X", tf, need));
        let Some(Command::History { before: Some(b), count, .. }) = l.older(&store, "X", tf, need) else { panic!() };
        assert_eq!((b, count), (1000 * 60, WARM_CHUNK));
        assert!(l.older(&store, "X", tf, need).is_none());
        store.put("X", tf, bars(900, 100), 2);
        l.on_bars("X", tf, Some(b), false);
        assert!(l.ready(&store, "X", tf, need));
        assert!(l.older(&store, "X", tf, need).is_none());

        // the view keeps asking until the source has nothing older
        let Some(Command::History { before: Some(b), .. }) = l.older(&store, "X", tf, Need::More) else { panic!() };
        l.on_bars("X", tf, Some(b), true);
        assert!(l.older(&store, "X", tf, Need::More).is_none());
        assert!(l.ready(&store, "X", tf, Need::Since { span: i64::MAX / 2, cap: usize::MAX }));
    }
}
