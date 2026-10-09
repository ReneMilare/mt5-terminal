//! Charts side by side (`chart.layout`): each one has its own symbol, timeframe, view, indicators,
//! date and delta; the store, the history loader, the feed and the ticket are shared. The active
//! chart (under the resting pointer, or the last one clicked) is the one the top bar, the date bar, the ticket and the shortcuts use.

use crate::chart::ChartView;
use crate::model::Timeframe;
use crate::navigation::DateNavigation;
use crate::settings::{self, Grid, Settings};
use crate::studies::Studies;
use eframe::egui::{Pos2, Rect};

pub struct Pane {
    pub symbol: String,
    pub tf: Timeframe,
    pub view: ChartView,
    pub studies: Studies,
    pub navigation: DateNavigation,
    /// Newest chart bar seen, to finalize the delta of the bar that closed.
    pub delta_last_bar: i64,
    /// Closed bars of delta the studies want (asked once the POC row is known).
    pub delta_want: u32,
    pub delta: DeltaAsked,
}

/// How much of a chart's delta was asked of the EA: the bars on screen first (every chart), then the
/// rest. The EA answers one request at a time, so one chart's long history doesn't hold the others back.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DeltaAsked {
    Nothing,
    /// The newest `DELTA_FIRST` bars, at this moment.
    Screen(std::time::Instant),
    All,
}

/// Closed bars of delta asked first: more than a screen of M5.
pub const DELTA_FIRST: u32 = 300;
/// The rest of a chart's delta waits this long at most for the other charts' first part.
pub const DELTA_REST_AFTER: std::time::Duration = std::time::Duration::from_secs(2);

impl DeltaAsked {
    pub fn key(self) -> &'static str {
        match self {
            DeltaAsked::Nothing => "nothing",
            DeltaAsked::Screen(_) => "screen",
            DeltaAsked::All => "all",
        }
    }
}

impl Pane {
    pub fn new(symbol: String, tf: Timeframe, settings: &Settings) -> Self {
        let mut view = ChartView::default();
        view.set_auto_scale(settings.chart.auto_scale);
        Self {
            studies: Studies::new(&symbol, tf, &settings.fibonacci, &settings.studies),
            symbol,
            tf,
            view,
            navigation: DateNavigation::default(),
            delta_last_bar: 0,
            delta_want: 0,
            delta: DeltaAsked::Nothing,
        }
    }

    /// Forget what was drawn (new symbol, timeframe or data), keeping the zoom and the scale mode.
    pub fn clear(&mut self, settings: &Settings) {
        self.view.reset();
        self.navigation.clear();
        self.delta_last_bar = 0;
        self.studies = Studies::new(&self.symbol, self.tf, &settings.fibonacci, &settings.studies);
    }

    /// As in `chart.charts`: "SYMBOL TIMEFRAME".
    pub fn entry(&self) -> String {
        format!("{} {}", self.symbol, self.tf.label())
    }
}

/// Symbol and timeframe of chart `i` (0 = the first) as the config sets them; a chart without an entry
/// takes the first symbol of `chart.symbols` not in `open` (cycling when all are), in the first chart's
/// timeframe.
pub fn configured(settings: &Settings, i: usize, open: &[&str]) -> (String, Timeframe) {
    let c = &settings.chart;
    if i == 0 {
        return (c.symbol.clone(), c.timeframe);
    }
    if let Some(entry) = c.charts.get(i - 1).and_then(|t| settings::chart_entry(t)) {
        return entry;
    }
    let symbol = c.symbols.iter().find(|s| !open.contains(&s.as_str())).unwrap_or(&c.symbols[i % c.symbols.len()]);
    (symbol.clone(), c.timeframe)
}

/// The charts' rectangles inside `rect`, row by row, `gap` points apart (whole points, no blurry edges).
pub fn cells(grid: Grid, rect: Rect, gap: f32) -> Vec<Rect> {
    let (cols, rows) = grid.shape();
    let edges = |from: f32, to: f32, n: usize| -> Vec<(f32, f32)> {
        let size = (to - from - gap * (n - 1) as f32) / n as f32;
        (0..n)
            .map(|k| {
                let a = (from + k as f32 * (size + gap)).round();
                let b = if k + 1 == n { to } else { (from + k as f32 * (size + gap) + size).round() };
                (a, b)
            })
            .collect()
    };
    let xs = edges(rect.left(), rect.right(), cols);
    let ys = edges(rect.top(), rect.bottom(), rows);
    ys.iter()
        .flat_map(|&(top, bottom)| xs.iter().map(move |&(left, right)| Rect::from_min_max(Pos2::new(left, top), Pos2::new(right, bottom))))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cells_cover_the_area_row_by_row_without_overlap() {
        let rect = Rect::from_min_max(Pos2::new(0.0, 40.0), Pos2::new(1001.0, 840.0));
        for grid in Grid::ALL {
            let cells = cells(grid, rect, 2.0);
            assert_eq!(cells.len(), grid.count());
            assert_eq!(cells[0].min, rect.min);
            assert_eq!(cells.last().unwrap().max, rect.max);
            for (i, a) in cells.iter().enumerate() {
                for b in &cells[i + 1..] {
                    assert!(!a.intersects(*b) || a.intersect(*b).area() == 0.0, "{grid:?}: {a:?} {b:?}");
                }
            }
        }
        let four = cells(Grid::Four, rect, 2.0);
        assert!(four[1].left() > four[0].right() && four[2].top() > four[0].bottom(), "second is to the right, third below");
    }

    #[test]
    fn extra_charts_come_from_the_config_or_the_next_symbols() {
        let (mut s, _) = settings::parse("[chart]\nsymbols = ['A', 'B', 'C']\nsymbol = 'B'\ntimeframe = 'H1'").unwrap();
        assert_eq!(configured(&s, 0, &[]), ("B".into(), Timeframe::H1));
        assert_eq!(configured(&s, 1, &["B"]), ("A".into(), Timeframe::H1));
        assert_eq!(configured(&s, 2, &["B", "A"]), ("C".into(), Timeframe::H1));
        assert_eq!(configured(&s, 3, &["B", "A", "C"]).1, Timeframe::H1, "all on screen: any of them");
        s.chart.charts = vec!["C M1".into()];
        assert_eq!(configured(&s, 1, &["B"]), ("C".into(), Timeframe::M1));
    }
}
