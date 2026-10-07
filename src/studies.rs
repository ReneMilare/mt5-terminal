//! Indicators drawn over the chart. The app itself ships none: a preset in `preset/` (a separate,
//! private repository, ignored by this one) is compiled in when the directory exists — `build.rs`
//! sets `cfg(has_preset)`. Any preset implements the API of `none::Studies` below.
//!
//! Rules for a preset: incremental (a tick without a new bar is O(1)), nothing heavy on the UI thread,
//! and it asks for history through [`Needs`] instead of loading everything (see `history`).

use crate::history::Need;
use crate::model::Timeframe;

/// What the studies read besides the chart's own series.
#[derive(Default)]
pub struct Needs {
    /// Symbols whose ticks they read (the chart symbol is always subscribed).
    pub symbols: Vec<String>,
    /// First chunk of each extra series: (symbol, timeframe, bars).
    pub first: Vec<(String, Timeframe, u32)>,
    /// How far back extra series must reach: (symbol, timeframe, need).
    pub older: Vec<(String, Timeframe, Need)>,
    /// Closed bars of the chart whose volume delta (and POC, see `delta_row`) they read (0 = none).
    pub delta_bars: u32,
}

#[cfg(has_preset)]
pub use crate::preset::Studies;
#[cfg(not(has_preset))]
pub use none::Studies;

/// No preset: nothing to compute, nothing to draw.
#[cfg(not(has_preset))]
mod none {
    use super::Needs;
    use crate::chart::{MapLevel, Marks, Overlay, Pane, VolumeBand};
    use crate::history::Need;
    use crate::model::{Store, Timeframe};

    pub struct Studies;

    impl Studies {
        pub fn new(_symbol: &str, _tf: Timeframe, _fibonacci: &crate::settings::Fibonacci) -> Self {
            Studies
        }

        pub fn configure_fibonacci(&mut self, _settings: &crate::settings::Fibonacci) -> bool {
            false
        }

        pub fn fibonacci_state(&self) -> serde_json::Value {
            serde_json::json!({"enabled": false, "timeframes": []})
        }

        pub fn is_for(&self, _symbol: &str, _tf: Timeframe) -> bool {
            true
        }

        /// History behind the newest chart bar needed before the first full pass.
        pub fn warmup(_tf: Timeframe) -> Need {
            Need::Since { span: 0, cap: 0 }
        }

        pub fn needs(&self) -> Needs {
            Needs::default()
        }

        /// New history arrived (any series).
        pub fn data_arrived(&mut self) {}

        /// Bring everything up to date. `tick`: price step of the chart symbol. `warm`: the chart covers
        /// `warmup`. `now`: server time. `quote(symbol)`: last (bid, tick time in server seconds).
        #[allow(clippy::too_many_arguments)]
        pub fn update(&mut self, _store: &Store, _digits: u32, _tick: f64, _warm: bool, _now: Option<i64>, _quote: impl Fn(&str) -> Option<(f64, i64)>) {}

        /// Price level height for the per-bar POC once known (the delta is requested with it).
        pub fn delta_row(&self) -> Option<f64> {
            None
        }

        /// One mark per bar over the candles (e.g. POC).
        pub fn marks(&self) -> Option<Marks<'_>> {
            None
        }

        pub fn overlays(&self) -> Vec<Overlay<'_>> {
            Vec::new()
        }

        pub fn map_levels(&self) -> Vec<MapLevel> {
            Vec::new()
        }

        pub fn pane(&self) -> Option<Pane<'_>> {
            None
        }

        /// Replaces the volume band (None: the plain volume).
        pub fn volume(&self) -> Option<VolumeBand<'_>> {
            None
        }
    }
}
