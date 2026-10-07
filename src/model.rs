//! Candles, timeframes and the series the chart draws.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Timeframe {
    M1,
    M5,
    M15,
    M30,
    H1,
    H4,
    D1,
    W1,
}

impl Timeframe {
    pub const ALL: [Timeframe; 8] = [
        Timeframe::M1,
        Timeframe::M5,
        Timeframe::M15,
        Timeframe::M30,
        Timeframe::H1,
        Timeframe::H4,
        Timeframe::D1,
        Timeframe::W1,
    ];

    pub fn seconds(self) -> i64 {
        match self {
            Timeframe::M1 => 60,
            Timeframe::M5 => 300,
            Timeframe::M15 => 900,
            Timeframe::M30 => 1800,
            Timeframe::H1 => 3600,
            Timeframe::H4 => 14_400,
            Timeframe::D1 => 86_400,
            Timeframe::W1 => 604_800,
        }
    }

    /// "M5", "h1"…: case doesn't matter.
    pub fn parse(s: &str) -> Option<Timeframe> {
        Timeframe::ALL.into_iter().find(|t| t.label().eq_ignore_ascii_case(s))
    }

    pub fn label(self) -> &'static str {
        match self {
            Timeframe::M1 => "M1",
            Timeframe::M5 => "M5",
            Timeframe::M15 => "M15",
            Timeframe::M30 => "M30",
            Timeframe::H1 => "H1",
            Timeframe::H4 => "H4",
            Timeframe::D1 => "D1",
            Timeframe::W1 => "W1",
        }
    }

    /// Open time of the bar that contains `time` (server time, seconds).
    pub fn bar_open(self, time: i64) -> i64 {
        match self {
            // MT5 weeks open on Sunday; 1970-01-01 was a Thursday
            Timeframe::W1 => time - (time + 4 * 86_400).rem_euclid(604_800),
            _ => time - time.rem_euclid(self.seconds()),
        }
    }
}

/// One candle; `time` is the open time in MT5 server time (seconds).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bar {
    pub time: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

#[derive(Clone, Debug, Default)]
pub struct Series {
    pub bars: Vec<Bar>,
    /// Last bid seen, for the price line even before the bar closes.
    pub last: Option<f64>,
    pub digits: u32,
}

impl Series {
    pub fn new(bars: Vec<Bar>, digits: u32) -> Self {
        let last = bars.last().map(|b| b.close);
        Self { bars, last, digits }
    }

    /// Fold a tick into the series: extends the forming bar or opens a new one.
    pub fn apply_tick(&mut self, tf: Timeframe, time: i64, price: f64, volume: f64) {
        self.last = Some(price);
        let open = tf.bar_open(time);
        match self.bars.last_mut() {
            Some(bar) if bar.time == open => {
                bar.high = bar.high.max(price);
                bar.low = bar.low.min(price);
                bar.close = price;
                bar.volume += volume;
            }
            Some(bar) if bar.time > open => {} // late tick for a closed bar
            _ => self.bars.push(Bar { time: open, open: price, high: price, low: price, close: price, volume }),
        }
    }

    /// Replace or append bars that came from the source (history or a bar update).
    pub fn merge(&mut self, bars: &[Bar]) {
        // older history (the chart scrolled back): one splice instead of an insert per bar
        if let (Some(first), Some(last_new)) = (self.bars.first(), bars.last())
            && last_new.time < first.time
        {
            self.bars.splice(0..0, bars.iter().copied());
            return;
        }
        for bar in bars {
            match self.bars.binary_search_by_key(&bar.time, |b| b.time) {
                Ok(i) => self.bars[i] = *bar,
                Err(i) => self.bars.insert(i, *bar),
            }
        }
        if self.last.is_none() {
            self.last = self.bars.last().map(|b| b.close);
        }
    }
}

/// Buy/sell volume per bar by the tick rule over the mid price ((bid+ask)/2): up = buy, down = sell,
/// unchanged = the previous side; each tick counts 1; the rule restarts at every bar; ticks before
/// the bar's first move have no side and count nowhere. With a `row` it also keeps the bar's POC:
/// counted ticks binned by `floor(bid / row)`, the first level to strictly pass the previous POC.
/// Closed bars come exact from the EA; the forming bar is built here from the streamed ticks.
#[derive(Default)]
pub struct Deltas {
    /// bar open time -> (buy, sell, poc price or NaN)
    pub bars: std::collections::BTreeMap<i64, (f64, f64, f64)>,
    live: Option<LiveDelta>,
    /// Price level height of the POC (0 = no POC).
    pub row: f64,
    /// Bumped on every change, so readers recompute only when needed.
    pub version: u64,
    /// Bumped when anything but the forming bar changes (EA batch, a bar closing, the row): between
    /// two bumps, readers only refresh the forming bar.
    pub closed: u64,
}

#[derive(Clone)]
struct LiveDelta {
    time: i64,
    buy: f64,
    sell: f64,
    prev_mid: f64,
    dir: i8,
    levels: std::collections::HashMap<i64, f64>,
    poc: Option<i64>,
}

impl LiveDelta {
    fn poc_price(&self, row: f64) -> f64 {
        match self.poc {
            Some(k) if row > 0.0 => (k as f64 + 0.5) * row,
            _ => f64::NAN,
        }
    }
}

// read by presets (none compiled in: unused)
#[cfg_attr(not(has_preset), allow(dead_code))]
impl Deltas {
    /// buy − sell of the bar opening at `time` (the forming one included).
    pub fn value(&self, time: i64) -> Option<f64> {
        match &self.live {
            Some(l) if l.time == time => Some(l.buy - l.sell),
            _ => self.bars.get(&time).map(|(b, s, _)| b - s),
        }
    }

    /// POC price of the bar opening at `time` (the forming one included).
    pub fn poc(&self, time: i64) -> Option<f64> {
        let p = match &self.live {
            Some(l) if l.time == time => l.poc_price(self.row),
            _ => self.bars.get(&time)?.2,
        };
        p.is_finite().then_some(p)
    }

    fn tick(&mut self, bar: i64, bid: f64, ask: f64) {
        if bid <= 0.0 {
            return;
        }
        let row = self.row;
        let l = match &mut self.live {
            Some(l) if l.time == bar => l,
            other => {
                // the bar closed: keep the estimate until the EA sends the exact value
                if let Some(old) = other.take() {
                    let poc = old.poc_price(row);
                    self.bars.entry(old.time).or_insert((old.buy, old.sell, poc));
                    self.closed += 1;
                }
                self.live.insert(LiveDelta { time: bar, buy: 0.0, sell: 0.0, prev_mid: 0.0, dir: 0, levels: Default::default(), poc: None })
            }
        };
        let mid = if ask > 0.0 { (bid + ask) / 2.0 } else { bid };
        if l.prev_mid > 0.0 {
            if mid > l.prev_mid {
                l.dir = 1;
            } else if mid < l.prev_mid {
                l.dir = -1;
            }
        }
        l.prev_mid = mid;
        match l.dir {
            1 => l.buy += 1.0,
            -1 => l.sell += 1.0,
            _ => {}
        }
        if row > 0.0 && l.dir != 0 {
            let key = (bid / row + 1e-9).floor() as i64;
            let v = {
                let e = l.levels.entry(key).or_insert(0.0);
                *e += 1.0;
                *e
            };
            let best = l.poc.and_then(|k| l.levels.get(&k)).copied().unwrap_or(0.0);
            if l.poc.is_none() || v > best {
                l.poc = Some(key);
            }
        }
        self.version += 1;
    }
}

/// Every series the app keeps, by symbol and timeframe. Ticks extend all series of their symbol.
#[derive(Default)]
pub struct Store {
    series: std::collections::HashMap<(String, Timeframe), Series>,
    deltas: std::collections::HashMap<(String, Timeframe), Deltas>,
}

impl Store {
    pub fn get(&self, symbol: &str, tf: Timeframe) -> Option<&Series> {
        self.series.get(&(symbol.to_string(), tf))
    }

    pub fn bars(&self, symbol: &str, tf: Timeframe) -> &[Bar] {
        self.get(symbol, tf).map(|s| s.bars.as_slice()).unwrap_or(&[])
    }

    /// History from the source: replaces an empty series, otherwise merges (resync of recent bars).
    pub fn put(&mut self, symbol: &str, tf: Timeframe, bars: Vec<Bar>, digits: u32) {
        let s = self.series.entry((symbol.to_string(), tf)).or_default();
        if s.bars.is_empty() {
            *s = Series::new(bars, digits);
        } else {
            s.merge(&bars);
        }
        s.digits = digits;
    }

    /// Fold a tick into every loaded series of `symbol`.
    pub fn tick(&mut self, symbol: &str, time: i64, price: f64, volume: f64) {
        for ((s, tf), series) in self.series.iter_mut() {
            if s == symbol && !series.bars.is_empty() {
                series.apply_tick(*tf, time, price, volume);
            }
        }
    }

    /// Start keeping the delta (and the POC, with `row` > 0) of `symbol`/`tf`: from now on its ticks feed it.
    pub fn track_delta(&mut self, symbol: &str, tf: Timeframe, row: f64) {
        let d = self.deltas.entry((symbol.to_string(), tf)).or_default();
        d.row = row;
        d.closed += 1;
    }

    pub fn deltas(&self, symbol: &str, tf: Timeframe) -> Option<&Deltas> {
        self.deltas.get(&(symbol.to_string(), tf))
    }

    /// Exact closed bars from the EA (they replace the live estimates).
    pub fn put_delta(&mut self, symbol: &str, tf: Timeframe, bars: &[Vec<f64>]) {
        let d = self.deltas.entry((symbol.to_string(), tf)).or_default();
        for b in bars.iter().filter(|b| b.len() >= 3) {
            let poc = b.get(3).copied().filter(|p| *p > 0.0).unwrap_or(f64::NAN);
            d.bars.insert(b[0] as i64, (b[1], b[2], poc));
        }
        d.version += 1;
        d.closed += 1;
    }

    /// A tick with both sides of the quote, for the deltas of its symbol.
    pub fn tick_quote(&mut self, symbol: &str, time_msc: i64, bid: f64, ask: f64) {
        for ((s, tf), d) in self.deltas.iter_mut() {
            if s == symbol {
                d.tick(tf.bar_open(time_msc.div_euclid(1000)), bid, ask);
            }
        }
    }

    pub fn keys(&self) -> impl Iterator<Item = &(String, Timeframe)> {
        self.series.keys()
    }

    pub fn clear(&mut self) {
        self.series.clear();
        self.deltas.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn week_opens_on_sunday() {
        let mon = 1_791_158_400; // 2026-10-05 00:00 UTC, Monday
        assert_eq!(Timeframe::W1.bar_open(mon + 3600), mon - 86_400);
        assert_eq!(Timeframe::W1.bar_open(mon - 86_400), mon - 86_400);
        assert_eq!(Timeframe::W1.bar_open(mon - 86_401), mon - 8 * 86_400);
    }

    #[test]
    fn ticks_build_bars() {
        let mut s = Series::default();
        s.apply_tick(Timeframe::M5, 1000, 10.0, 1.0);
        s.apply_tick(Timeframe::M5, 1100, 12.0, 1.0);
        s.apply_tick(Timeframe::M5, 1150, 9.0, 1.0);
        s.apply_tick(Timeframe::M5, 1200, 11.0, 1.0);
        assert_eq!(s.bars.len(), 2);
        assert_eq!(s.bars[0], Bar { time: 900, open: 10.0, high: 12.0, low: 9.0, close: 9.0, volume: 3.0 });
        assert_eq!(s.bars[1].time, 1200);
        assert_eq!(s.last, Some(11.0));
    }

    #[test]
    fn delta_by_tick_rule() {
        let mut st = Store::default();
        st.track_delta("X", Timeframe::M1, 0.5);
        // first tick only sets the mid; up, same (keeps buying), down, down
        for (ms, bid) in [(0, 10.0), (1000, 10.5), (2000, 10.5), (3000, 10.0), (4000, 9.5)] {
            st.tick_quote("X", ms, bid, bid + 0.5);
        }
        let d = st.deltas("X", Timeframe::M1).unwrap();
        assert_eq!(d.value(0), Some(2.0 - 2.0));
        // a new bar restarts the rule; the closed one keeps the estimate until the EA's value
        st.tick_quote("X", 61_000, 9.0, 9.5);
        let d = st.deltas("X", Timeframe::M1).unwrap();
        assert_eq!((d.value(0), d.value(60)), (Some(0.0), Some(0.0)));
        // POC of the closed bar from the ticks: counted bids 10.5, 10.5, 10.0, 9.5 -> level 21 (2 ticks)
        assert_eq!(st.deltas("X", Timeframe::M1).unwrap().poc(0), Some(10.75));
        st.put_delta("X", Timeframe::M1, &[vec![0.0, 7.0, 3.0, 9.25]]);
        let d = st.deltas("X", Timeframe::M1).unwrap();
        assert_eq!((d.value(0), d.poc(0)), (Some(4.0), Some(9.25)));
    }

    #[test]
    fn merge_keeps_order() {
        let b = |t| Bar { time: t, open: 1.0, high: 1.0, low: 1.0, close: 1.0, volume: 0.0 };
        let mut s = Series::new(vec![b(60), b(180)], 2);
        s.merge(&[b(120), b(180), b(240)]);
        assert_eq!(s.bars.iter().map(|x| x.time).collect::<Vec<_>>(), vec![60, 120, 180, 240]);
        s.merge(&[b(0), b(30)]);
        assert_eq!(s.bars.iter().map(|x| x.time).collect::<Vec<_>>(), vec![0, 30, 60, 120, 180, 240]);
    }
}
