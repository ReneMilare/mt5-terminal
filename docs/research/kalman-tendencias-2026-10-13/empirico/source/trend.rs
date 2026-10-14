//! `KalmanTrend` (level + slope in log price, state bands) and `SessionVWAP` in daily mode, with the
//! preset's inputs. Both are incremental: closed bars are processed once, in order, like MT5's
//! `prev_calculated`; the forming bar costs O(1).

use super::ctx;
use super::math::{log_return, variance, Kalman, Weighted};
use crate::model::Bar;

/// Shade of a Kalman segment.
pub const UP: u8 = 0;
pub const DOWN: u8 = 1;
pub const FLAT: u8 = 2;

const K_HISTORY: usize = 10_000;
const K_WARMUP: usize = 256;

/// How the filter's speed follows the market (`Off`: fixed, as in MT5).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Adaptive {
    Off,
    /// By its own innovations: same-sign errors in a row mean it lags (faster), alternating mean
    /// noise (slower). Speed = exp(gain × ρ), ρ the lag-1 autocorrelation of the normalized
    /// innovations over ~`span` bars.
    Innovation,
    /// By the efficiency of the move (net change / path over `span` bars) against its own average.
    Efficiency,
}

/// KalmanTrend's inputs; the default is the MT5 chart's (what `--verify` compares).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KalmanParams {
    pub window: usize,
    pub level_noise: f64,
    pub slope_noise: f64,
    pub measurement_noise: f64,
    pub band_sigma: f64,
    pub slope_threshold: f64,
    /// Hysteresis: a trend shade holds while the slope stays beyond this (≤ `slope_threshold`;
    /// equal = no hysteresis, as in MT5).
    pub slope_exit: f64,
    pub adaptive: Adaptive,
    pub adapt_gain: f64,
    pub adapt_span: usize,
    /// The speed stays within [1/cap, cap] times the fixed one.
    pub adapt_cap: f64,
}

impl Default for KalmanParams {
    fn default() -> Self {
        Self {
            window: 50,
            level_noise: 0.02,
            slope_noise: 0.002,
            measurement_noise: 1.0,
            band_sigma: 1.96,
            slope_threshold: 0.1,
            slope_exit: 0.1,
            adaptive: Adaptive::Off,
            adapt_gain: 3.0,
            adapt_span: 20,
            adapt_cap: 16.0,
        }
    }
}

/// State of the adaptive speed, carried bar to bar (the same incrementally or in a full pass).
#[derive(Clone, Copy, Debug, Default)]
struct Adapt {
    /// EWMAs of ν² and of ν·ν₋₁ (ν: normalized innovation), and the last ν.
    m0: f64,
    m1: f64,
    last: f64,
    /// EWMA of the efficiency ratio (its own reference level).
    er_mean: f64,
}

/// How `bars` relates to the series the values were computed on (first bar `first_time`, `len` bars):
/// `Some(0)` the same, `Some(k)` the same with `k` older bars loaded in front, `None` another series.
pub fn prepended(first_time: i64, len: usize, bars: &[Bar]) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let k = bars.partition_point(|b| b.time < first_time);
    (k < bars.len() && bars[k].time == first_time && bars.len() >= len + k).then_some(k)
}

/// Make room for `k` older bars: existing values keep their bars, the older ones stay empty (like
/// MT5, the indicators only cover their history limit, not everything scrolled into view).
pub fn shift_front<T: Copy>(v: &mut Vec<T>, k: usize, empty: T) {
    v.splice(0..0, std::iter::repeat_n(empty, k));
}

#[derive(Default)]
pub struct KalmanTrend {
    pub trend: Vec<f64>,
    pub upper: Vec<f64>,
    pub lower: Vec<f64>,
    pub shade: Vec<u8>,
    /// Speed factor applied at each bar (1 = fixed).
    pub speed: Vec<f64>,
    /// Slope in standard deviations of its estimate (what the shade thresholds compare).
    pub slope_t: Vec<f64>,
    pub params: KalmanParams,
    returns: Vec<f64>,
    state: Kalman,
    adapt: Adapt,
    ready: bool,
    /// Compute every bar (studies), not only MT5's history limit.
    unlimited: bool,
    /// Next closed bar to process.
    next: usize,
    first_time: i64,
}

impl KalmanTrend {
    pub fn new(params: KalmanParams) -> Self {
        Self { params, ..Self::default() }
    }

    /// Without MT5's history limit: every loaded bar (for studies over long history).
    #[cfg(test)]
    pub fn unlimited(params: KalmanParams) -> Self {
        Self { params, unlimited: true, ..Self::default() }
    }

    /// `bars` is the series these values belong to, maybe with newer bars or older ones in front
    /// (`update` then stays cheap: no full pass).
    pub fn follows(&self, bars: &[Bar]) -> bool {
        prepended(self.first_time, self.trend.len(), bars).is_some()
    }

    /// Closed bars processed so far (the forming one never is).
    pub fn closed(&self) -> usize {
        self.next
    }

    pub fn update(&mut self, bars: &[Bar]) {
        let n = bars.len();
        if n < 2 {
            *self = Self { unlimited: self.unlimited, ..Self::new(self.params) };
            return;
        }
        match prepended(self.first_time, self.trend.len(), bars) {
            Some(0) => {}
            Some(k) => {
                for v in [&mut self.trend, &mut self.upper, &mut self.lower, &mut self.returns, &mut self.speed, &mut self.slope_t] {
                    shift_front(v, k, f64::NAN);
                }
                shift_front(&mut self.shade, k, FLAT);
                self.next += k;
                self.first_time = bars[0].time;
            }
            None => {
                let history = if self.unlimited { n } else { K_HISTORY + self.params.window + K_WARMUP };
                *self = Self { first_time: bars[0].time, unlimited: self.unlimited, ..Self::new(self.params) };
                self.next = (n as isize - 1 - history as isize).max(1) as usize;
            }
        }
        self.trend.resize(n, f64::NAN);
        self.upper.resize(n, f64::NAN);
        self.lower.resize(n, f64::NAN);
        self.shade.resize(n, FLAT);
        self.returns.resize(n, f64::NAN);
        self.speed.resize(n, f64::NAN);
        self.slope_t.resize(n, f64::NAN);
        for i in self.next..n - 1 {
            self.step(bars, i);
        }
        self.next = self.next.max(n - 1);
    }

    fn step(&mut self, bars: &[Bar], i: usize) {
        let close = bars[i].close;
        let k = self.params;
        self.returns[i] = log_return(bars[i - 1].close, close);
        if i < k.window + 1 || close <= 0.0 {
            self.ready = false;
            return;
        }
        let Some(var) = variance(&self.returns[i - k.window..i]) else {
            self.ready = false;
            return;
        };
        let speed = self.speed_at(i);
        let (q0, q1, r) = (k.level_noise * var * speed, k.slope_noise * var * speed, k.measurement_noise * var);
        if !self.ready {
            self.state = Kalman::new(bars[i - 1].close.ln(), r);
            self.ready = true;
        }
        if !self.state.update(close.ln(), q0, q1, r) {
            self.ready = false;
            return;
        }
        self.speed[i] = speed;
        if k.adaptive == Adaptive::Innovation {
            let nu = self.state.innovation / self.state.innovation_var.sqrt();
            let a = 2.0 / (k.adapt_span as f64 + 1.0);
            let st = &mut self.adapt;
            st.m0 += a * (nu * nu - st.m0);
            st.m1 += a * (nu * st.last - st.m1);
            st.last = nu;
        }
        let s = &self.state;
        let sigma = s.p00.sqrt();
        self.trend[i] = s.level.exp();
        self.upper[i] = (s.level + k.band_sigma * sigma).exp();
        self.lower[i] = (s.level - k.band_sigma * sigma).exp();
        // slope in standard deviations; a trend shade holds until it falls back under the exit
        let t = s.slope / s.p11.sqrt();
        self.slope_t[i] = t;
        let held = match self.shade[i - 1] {
            UP if t > k.slope_exit => Some(UP),
            DOWN if t < -k.slope_exit => Some(DOWN),
            _ => None,
        };
        let threshold = k.slope_threshold * s.p11.sqrt();
        self.shade[i] = held.unwrap_or(if s.slope > threshold { UP } else if s.slope < -threshold { DOWN } else { FLAT });
    }

    /// Speed factor for bar `i` from what was known before it (1 = the fixed filter).
    fn speed_at(&mut self, i: usize) -> f64 {
        let k = self.params;
        let raw = match k.adaptive {
            Adaptive::Off => return 1.0,
            Adaptive::Innovation => {
                let st = self.adapt;
                if st.m0 > 0.0 { (k.adapt_gain * (st.m1 / st.m0).clamp(-1.0, 1.0)).exp() } else { 1.0 }
            }
            Adaptive::Efficiency => {
                let span = k.adapt_span.max(2);
                if i <= span {
                    return 1.0;
                }
                let window = &self.returns[i + 1 - span..=i];
                let path: f64 = window.iter().filter(|r| r.is_finite()).map(|r| r.abs()).sum();
                let net: f64 = window.iter().filter(|r| r.is_finite()).sum::<f64>().abs();
                if path <= 0.0 {
                    return 1.0;
                }
                let er = net / path;
                let st = &mut self.adapt;
                st.er_mean = if st.er_mean == 0.0 { er } else { st.er_mean + (er - st.er_mean) / (10.0 * span as f64) };
                (k.adapt_gain * (er - st.er_mean)).exp()
            }
        };
        raw.clamp(1.0 / k.adapt_cap, k.adapt_cap)
    }
}

const V_MAX_BARS: usize = 10_000;

/// Accumulator of the current period, as SessionVWAP keeps it before the forming bar.
#[derive(Clone, Copy, Default)]
struct VwapState {
    acc: Weighted,
    key: Option<i64>,
    /// A closed bar without volume invalidates the rest of the period.
    invalid: bool,
}

/// VWAP of the server day, HLC3 weighted by tick volume (SessionVWAP `InpMode=2`, no bands).
#[derive(Default)]
pub struct DailyVwap {
    pub values: Vec<f64>,
    base: VwapState,
    floor: usize,
    next: usize,
    first_time: i64,
}

impl DailyVwap {
    pub fn update(&mut self, bars: &[Bar]) {
        let n = bars.len();
        if n < 2 {
            *self = Self::default();
            return;
        }
        match prepended(self.first_time, self.values.len(), bars) {
            Some(0) => {}
            Some(k) => {
                shift_front(&mut self.values, k, f64::NAN);
                self.floor += k;
                self.next += k;
                self.first_time = bars[0].time;
            }
            None => {
                // the history limit never cuts the current day
                let mut floor = n.saturating_sub(V_MAX_BARS);
                let need = ctx::day(bars[n - 1].time);
                if bars[floor].time > need {
                    floor = bars[..floor].partition_point(|b| b.time < need);
                }
                *self = Self { first_time: bars[0].time, floor, next: floor, ..Self::default() };
            }
        }
        self.values.resize(n, f64::NAN);
        let mut st = self.base;
        for i in self.next..n - 1 {
            self.values[i] = self.step(bars, i, &mut st, false);
        }
        self.base = st;
        self.next = self.next.max(n - 1);
        self.values[n - 1] = self.step(bars, n - 1, &mut st, true);
    }

    fn step(&self, bars: &[Bar], i: usize, st: &mut VwapState, forming: bool) -> f64 {
        let mut key = Some(ctx::day(bars[i].time));
        // publish only days whose first bar is inside the loaded range
        if let Some(k) = key
            && bars[self.floor].time > k
            && (self.floor == 0 || bars[self.floor - 1].time >= k)
        {
            key = None;
        }
        if key.is_none() || key != st.key {
            *st = VwapState { key, ..VwapState::default() };
        }
        if key.is_none() {
            return f64::NAN;
        }
        let b = &bars[i];
        if b.volume <= 0.0 && !forming {
            st.invalid = true;
            return f64::NAN;
        }
        if st.invalid {
            return f64::NAN;
        }
        st.acc.add((b.high + b.low + b.close) / 3.0, b.volume);
        if st.acc.total > 0.0 { st.acc.mean } else { f64::NAN }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bars(n: usize, step: i64) -> Vec<Bar> {
        (0..n)
            .map(|i| {
                let p = 100.0 + (i as f64 * 0.1).sin() * 2.0 + i as f64 * 0.01;
                Bar { time: 1_789_992_000 + i as i64 * step, open: p, high: p + 0.3, low: p - 0.3, close: p + 0.1, volume: 1.0 + (i % 7) as f64 }
            })
            .collect()
    }

    #[test]
    fn kalman_incremental_equals_full() {
        let all = bars(600, 300);
        let mut full = KalmanTrend::default();
        full.update(&all);
        let mut inc = KalmanTrend::default();
        inc.update(&all[..400]);
        for n in 401..=600 {
            inc.update(&all[..n]);
        }
        assert_eq!(full.next, inc.next);
        for i in 0..599 {
            assert!(full.trend[i] == inc.trend[i] || (full.trend[i].is_nan() && inc.trend[i].is_nan()), "bar {i}");
        }
        assert!(full.trend[598].is_finite() && full.trend[599].is_nan());
        assert!(full.upper[598] > full.trend[598] && full.lower[598] < full.trend[598]);
    }

    #[test]
    fn adaptive_and_hysteresis_stay_incremental() {
        let all = bars(900, 300);
        for adaptive in [Adaptive::Innovation, Adaptive::Efficiency] {
            let p = KalmanParams { adaptive, adapt_gain: 4.0, slope_threshold: 1.0, slope_exit: 0.0, ..KalmanParams::default() };
            let mut full = KalmanTrend::new(p);
            full.update(&all);
            let mut inc = KalmanTrend::new(p);
            inc.update(&all[..500]);
            for n in 501..=900 {
                inc.update(&all[..n]);
            }
            for i in 0..899 {
                assert!(full.trend[i] == inc.trend[i] || (full.trend[i].is_nan() && inc.trend[i].is_nan()), "{adaptive:?} bar {i}");
                assert_eq!(full.shade[i], inc.shade[i], "{adaptive:?} bar {i}");
            }
            let speeds: Vec<f64> = full.speed.iter().copied().filter(|x| x.is_finite()).collect();
            assert!(speeds.iter().any(|&x| (x - 1.0).abs() > 0.05), "{adaptive:?} changes the speed");
            assert!(speeds.iter().all(|&x| x >= 1.0 / p.adapt_cap && x <= p.adapt_cap));
        }
    }

    #[test]
    fn hysteresis_holds_the_trend_until_the_exit() {
        let all = bars(900, 300);
        let entry = KalmanParams { slope_threshold: 1.0, slope_exit: 1.0, ..KalmanParams::default() };
        let held = KalmanParams { slope_exit: 0.0, ..entry };
        let (mut a, mut b) = (KalmanTrend::new(entry), KalmanTrend::new(held));
        a.update(&all);
        b.update(&all);
        let mut longer = 0;
        for i in 1..899 {
            let t = b.slope_t[i];
            if !t.is_finite() {
                continue;
            }
            // with the exit at 0, an up shade never ends while the slope is still positive
            if b.shade[i - 1] == UP && t > 0.0 {
                assert_eq!(b.shade[i], UP, "bar {i}");
            }
            // it only adds trend bars: entries are the same
            if a.shade[i] == UP {
                assert_eq!(b.shade[i], UP, "bar {i}");
            }
            longer += (b.shade[i] != a.shade[i]) as usize;
        }
        assert!(longer > 0, "the hysteresis keeps some trends longer");
    }

    #[test]
    fn vwap_resets_each_day() {
        let all = bars(600, 300); // ~2 days of M5
        let mut v = DailyVwap::default();
        v.update(&all);
        // first day starts before the loaded range: not published
        let first_full = all.iter().position(|b| b.time == ctx::day(b.time)).unwrap();
        assert!(v.values[..first_full].iter().all(|x| x.is_nan()));
        let b = &all[first_full];
        assert!((v.values[first_full] - (b.high + b.low + b.close) / 3.0).abs() < 1e-12);
        // incremental with a changing forming bar gives the same as full
        let mut inc = DailyVwap::default();
        inc.update(&all[..300]);
        for n in 301..=600 {
            inc.update(&all[..n]);
        }
        for i in 0..600 {
            assert!(v.values[i] == inc.values[i] || (v.values[i].is_nan() && inc.values[i].is_nan()));
        }
    }

    #[test]
    fn older_bars_shift_without_recompute() {
        let all = bars(600, 300);
        let mut k = KalmanTrend::default();
        k.update(&all[100..]);
        let before = k.trend.clone();
        k.update(&all);
        assert_eq!(k.trend.len(), 600);
        assert!(k.trend[..100].iter().all(|x| x.is_nan()));
        for (a, b) in before.iter().zip(&k.trend[100..]) {
            assert!(a == b || (a.is_nan() && b.is_nan()));
        }
        let mut v = DailyVwap::default();
        v.update(&all[100..]);
        let before = v.values.clone();
        v.update(&all);
        // closed bars keep their values; the forming bar is recomputed every update
        for (a, b) in before[..499].iter().zip(&v.values[100..599]) {
            assert!(a == b || (a.is_nan() && b.is_nan()));
        }
    }
}
