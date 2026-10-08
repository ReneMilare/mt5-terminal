//! Candlestick chart widget: our own engine on top of egui's painter (tessellated by wgpu).
//!
//! The x axis is the bar index, not time, so weekends and session gaps take no space (like MT5 and
//! TradingView). `right` is the (fractional) bar index under the right edge of the plot.

pub mod axis;

use crate::feed::{OrderKind, Side};
use crate::model::{Series, Timeframe};
use crate::theme::Palette;
use eframe::egui::{
    self, Align2, Color32, CornerRadius, CursorIcon, FontId, Mesh, Pos2, Rect, Sense, Shape, Stroke, Ui, Vec2,
};

const PRICE_AXIS_W: f32 = 76.0;
const TIME_AXIS_H: f32 = 26.0;
/// Height of the indicator pane under the price plot, and of its minimized strip.
const PANE_H: f32 = 170.0;
const PANE_MIN_H: f32 = 20.0;
/// Rows of the ribbon at the bottom of the plot.
const RIBBON_ROW_H: f32 = 9.0;
const RIBBON_GAP: f32 = 2.0;
/// Empty bars kept to the right of the last bar when following the market.
const RIGHT_PAD: f64 = 6.0;
const MIN_BAR_PX: f32 = 1.0;
const MAX_BAR_PX: f32 = 120.0;
/// Vertical drag (px) before a pan of the plot turns auto-scale off.
const UNLOCK_Y_PX: f32 = 14.0;
/// How close (px) the pointer must be to a trade line for Delete.
const GRAB_PX: f32 = 5.0;
/// With Alt held, how close (px) to grab a trade line and drag it (wider: Alt makes the intent clear).
const ALT_GRAB_PX: f32 = 10.0;
/// The × that removes an order/stop/target: a square left of the line's tag.
const REMOVE_X: f32 = 48.0;
const REMOVE_W: f32 = 18.0;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Zone {
    Plot,
    PriceAxis,
    TimeAxis,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CursorMode {
    Arrow,
    #[default]
    Hand,
    Cross,
}

impl CursorMode {
    pub const ALL: [Self; 3] = [Self::Arrow, Self::Hand, Self::Cross];

    pub fn key(self) -> &'static str {
        match self { Self::Arrow => "arrow", Self::Hand => "hand", Self::Cross => "cross" }
    }

    pub fn label(self) -> &'static str {
        match self { Self::Arrow => "Seta", Self::Hand => "Mão", Self::Cross => "Cruz" }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Self::Arrow => "Seta: apontar e selecionar, sem arrastar o gráfico",
            Self::Hand => "Mão: clique e arraste para mover o gráfico",
            Self::Cross => {
                "Cruz: clique e arraste para medir porcentagem e barras. Esc limpa a medição"
            }
        }
    }

    /// Vector icons stay sharp at any display scale and do not depend on font glyphs.
    pub fn button(self, ui: &mut Ui, selected: bool) -> egui::Response {
        let response = ui.add(egui::Button::selectable(selected, "").min_size(Vec2::splat(28.0)));
        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::Button,
                ui.is_enabled(),
                selected,
                self.label(),
            )
        });
        if ui.is_rect_visible(response.rect) {
            let color = ui
                .style()
                .interact_selectable(&response, selected)
                .fg_stroke
                .color;
            let stroke = Stroke::new(1.5, color);
            let at = |x: f32, y: f32| response.rect.center() + Vec2::new(x - 10.0, y - 10.0);
            match self {
                Self::Arrow => {
                    let points = [
                        (3.0, 2.0),
                        (3.0, 17.0),
                        (7.0, 13.0),
                        (10.0, 19.0),
                        (13.0, 17.5),
                        (10.0, 11.5),
                        (16.0, 11.5),
                    ];
                    ui.painter().add(Shape::closed_line(
                        points.into_iter().map(|(x, y)| at(x, y)).collect(),
                        stroke,
                    ));
                }
                Self::Hand => {
                    let points = [
                        (6.0, 12.0),
                        (6.0, 5.0),
                        (7.0, 4.0),
                        (8.0, 5.0),
                        (8.0, 10.0),
                        (8.0, 2.0),
                        (9.0, 1.0),
                        (10.0, 2.0),
                        (10.0, 10.0),
                        (10.0, 3.0),
                        (11.0, 2.0),
                        (12.0, 3.0),
                        (12.0, 10.0),
                        (12.0, 5.0),
                        (13.0, 4.0),
                        (14.0, 5.0),
                        (14.0, 13.0),
                        (13.0, 17.0),
                        (11.0, 19.0),
                        (7.0, 19.0),
                        (5.0, 17.0),
                        (2.0, 12.0),
                        (2.0, 10.0),
                        (3.0, 9.0),
                        (4.0, 10.0),
                    ];
                    ui.painter().add(Shape::closed_line(
                        points.into_iter().map(|(x, y)| at(x, y)).collect(),
                        stroke,
                    ));
                }
                Self::Cross => {
                    for (a, b) in [
                        ((10.0, 1.0), (10.0, 6.0)),
                        ((10.0, 14.0), (10.0, 19.0)),
                        ((1.0, 10.0), (6.0, 10.0)),
                        ((14.0, 10.0), (19.0, 10.0)),
                    ] {
                        ui.painter()
                            .line_segment([at(a.0, a.1), at(b.0, b.1)], stroke);
                    }
                    ui.painter().circle_stroke(at(10.0, 10.0), 3.0, stroke);
                }
            }
        }
        response.on_hover_text(self.hint())
    }

    fn icon(self) -> CursorIcon {
        match self { Self::Arrow => CursorIcon::Default, Self::Hand => CursorIcon::Grab, Self::Cross => CursorIcon::Crosshair }
    }
}

#[derive(Clone, Copy, Debug)]
struct MeasurePoint {
    bar: usize,
    price: f64,
}

#[derive(Clone, Copy, Debug)]
struct Measurement {
    start: MeasurePoint,
    end: MeasurePoint,
}

impl Measurement {
    fn percent(self) -> Option<f64> {
        let percent = (self.end.price - self.start.price) / self.start.price * 100.0;
        percent.is_finite().then_some(percent)
    }

    fn bars(self) -> usize {
        self.end.bar.abs_diff(self.start.bar)
    }
}

pub struct ChartView {
    right: f64,
    bar_px: f32,
    auto_y: bool,
    y_lo: f64,
    y_hi: f64,
    drag: Option<Zone>,
    drag_dy: f32,
    cursor: CursorMode,
    measurement: Option<Measurement>,
    measuring: bool,
    follow: bool,
    known_len: usize,
    /// Time of the oldest bar seen, to keep the view still when older history is loaded in front.
    known_first: i64,
    /// Center this bar after reconciling newly prepended history and measuring the plot.
    target_time: Option<i64>,
    /// The view reaches (or is within a screen of) the oldest loaded bar: more history is wanted.
    pub wants_older: bool,
    /// Trade line being dragged and where it is now.
    level_drag: Option<(Handle, f64)>,
    /// Price under the pointer when the context menu opened.
    menu_price: Option<f64>,
    /// Where the context menu opened, and the indicator found there.
    menu_at: Option<Pos2>,
    menu_study: Option<&'static str>,
    /// What the user did to trades on the chart this frame (drained by the app).
    pub actions: Vec<ChartAction>,
}

/// What a draggable trade line moves.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Handle {
    /// Entry of a pending order.
    Order(u64),
    /// Entry of a position: dragging it out creates a stop or a target.
    Position(u64),
    /// Stop of a position or pending order.
    Sl(u64),
    /// Target of a position or pending order.
    Tp(u64),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ChartAction {
    /// A trade line was dropped at `price`.
    Move { handle: Handle, price: f64 },
    /// The × of a line (or Delete over it): close the position, cancel the order, or drop that stop/target.
    Remove { handle: Handle },
    /// Minimize or restore the indicator pane.
    TogglePane,
    /// New pending order from the context menu.
    Order { side: Side, kind: OrderKind, price: f64 },
    /// Open the options of an indicator (its key).
    EditStudy(&'static str),
}

impl Default for ChartView {
    fn default() -> Self {
        Self {
            right: 0.0,
            bar_px: 8.0,
            auto_y: true,
            y_lo: 0.0,
            y_hi: 1.0,
            drag: None,
            drag_dy: 0.0,
            cursor: CursorMode::default(),
            measurement: None,
            measuring: false,
            follow: true,
            known_len: 0,
            known_first: 0,
            target_time: None,
            wants_older: false,
            level_drag: None,
            menu_price: None,
            menu_at: None,
            menu_study: None,
            actions: Vec::new(),
        }
    }
}

/// What the chart needs to draw a frame.
pub struct ChartData<'a> {
    pub series: &'a Series,
    pub symbol: &'a str,
    pub tf: Timeframe,
    /// Current server time (seconds, fractional) when known, for the bar countdown.
    pub server_now: Option<f64>,
    /// Positions and pending orders of the symbol, drawn as horizontal lines.
    pub levels: &'a [Level],
    /// Indicator lines over the candles (one value per bar, NaN = gap).
    pub overlays: &'a [Overlay<'a>],
    /// Support/resistance lines from a bar time to the right edge.
    pub map_levels: &'a [MapLevel],
    /// Indicator pane under the price plot.
    pub pane: Option<&'a Pane<'a>>,
    /// The pane is shown (false: a thin strip that restores it).
    pub pane_open: bool,
    /// Drawing order of the plot's layers, back to front.
    pub layers: &'a [Layer],
    /// Live bid/ask of the symbol, also shown while trading is locked.
    pub quote: Option<(f64, f64)>,
    /// Order previews and the context menu require the same permission as the ticket.
    pub can_trade: bool,
    pub cursor: CursorMode,
    /// Price step of the symbol (the order following the pointer shows the price it will get).
    pub tick: f64,
    /// Volume of the ticket, shown on the order following the pointer.
    pub order_volume: &'a str,
    /// Stop and target of the next order, drawn with the order following the pointer.
    pub bracket: Option<crate::trading::Bracket>,
    /// Replaces the volume band at the bottom of the plot (e.g. volume delta).
    pub volume: Option<VolumeBand<'a>>,
    /// A small mark per bar at a price (e.g. the bar's POC), drawn over the candles.
    pub marks: Option<Marks<'a>>,
    /// Tint behind the candles, per bar.
    pub shading: Option<Shading<'a>>,
    /// Thin rows at the bottom of the plot (the volume sits on top of them).
    pub ribbon: Option<Ribbon<'a>>,
    /// The indicators' line under the legend.
    pub legend: &'a [crate::studies::LegendItem],
    /// Editable indicators (key, name), listed in the context menu.
    pub studies: &'a [(&'static str, &'static str)],
}

/// Thin rows at the bottom of the plot, a class per bar each (e.g. one state per timeframe).
pub struct Ribbon<'a> {
    pub study: &'static str,
    /// Top to bottom.
    pub rows: Vec<RibbonRow<'a>>,
}

pub struct RibbonRow<'a> {
    pub label: &'static str,
    /// Index into `palette` per bar (out of it = nothing).
    pub classes: &'a [u8],
    pub palette: &'a [Color32],
}

impl Ribbon<'_> {
    fn height(&self) -> f32 {
        if self.rows.is_empty() { 0.0 } else { self.rows.len() as f32 * (RIBBON_ROW_H + RIBBON_GAP) + RIBBON_GAP }
    }
}

/// A tint behind the candles: a class per bar (`u8::MAX` = none) and the color of each class.
pub struct Shading<'a> {
    pub study: &'static str,
    pub classes: &'a [u8],
    pub palette: &'a [Color32],
}

/// One discreet mark per bar: a short horizontal dash at `prices[i]` (NaN = none), sized by the zoom.
pub struct Marks<'a> {
    pub study: &'static str,
    pub prices: &'a [f64],
    pub color: Color32,
}

/// Values drawn in the volume band instead of the volume, the same way: bars up from the bottom,
/// height |value| on the visible maximum, color by sign (`palette`: positive, negative, zero).
pub struct VolumeBand<'a> {
    pub study: &'static str,
    /// One value per bar (NaN = none).
    pub values: &'a [f64],
    pub palette: [Color32; 3],
}

/// Lines with a × to remove them: every trade line (a position's × closes it).
fn removable(_h: Handle) -> bool {
    true
}

/// What the price plot draws, in an order the user picks (the last one is in front).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Layer {
    /// Candles and the bid/ask lines and price tags.
    Price,
    /// Indicator lines over the candles.
    Indicators,
    /// Support/resistance map.
    Levels,
    /// Positions and pending orders.
    Trades,
}

impl Layer {
    /// Default order, back to front: the price in front of everything.
    pub const DEFAULT: [Layer; 4] = [Layer::Levels, Layer::Indicators, Layer::Trades, Layer::Price];

    pub const ALL: [Layer; 4] = Layer::DEFAULT;

    /// Name in the config file and in `ctl`.
    pub fn key(self) -> &'static str {
        match self {
            Layer::Price => "price",
            Layer::Indicators => "indicators",
            Layer::Levels => "levels",
            Layer::Trades => "trades",
        }
    }

    pub fn from_key(key: &str) -> Option<Layer> {
        Layer::ALL.into_iter().find(|l| l.key() == key)
    }

    pub fn label(self) -> &'static str {
        match self {
            Layer::Price => "Preço (candles)",
            Layer::Indicators => "Indicadores",
            Layer::Levels => "Níveis",
            Layer::Trades => "Posições e ordens",
        }
    }
}

/// A line over the candles. The segment that ends at bar i takes `palette[shade[i]]` (MT5's
/// DRAW_COLOR_LINE), or `palette[0]` without shades.
pub struct Overlay<'a> {
    /// Key of the indicator it belongs to (right click edits it).
    pub study: &'static str,
    pub values: &'a [f64],
    pub shade: Option<&'a [u8]>,
    pub palette: &'a [Color32],
    pub width: f32,
    pub dotted: bool,
}

pub struct MapLevel {
    pub study: &'static str,
    pub price: f64,
    /// Bar time where the line starts.
    pub from: i64,
    pub color: Color32,
    pub width: f32,
    pub dashed: bool,
    pub text: String,
    /// Complete sources, even when the on-chart label is shortened.
    pub details: String,
}

/// One strip of the pane: a class per bar (`u8::MAX` = nothing) and the color of each class.
pub struct Strip<'a> {
    pub classes: &'a [u8],
    pub palette: &'a [Color32],
    /// Height on the pane's 0.4–7.6 scale (like an MT5 indicator subwindow).
    pub level: f32,
}

pub struct Pane<'a> {
    pub study: &'static str,
    pub strips: [Strip<'a>; 3],
    pub lines: &'a [String],
    /// Second text box, with a color per row.
    pub boxed: &'a [(String, Color32)],
}

/// A labelled horizontal line: an open position (solid) or a pending order (dashed).
pub struct Level {
    pub price: f64,
    pub color: Color32,
    pub label: String,
    pub dashed: bool,
    /// Set when the line can be dragged (trading allowed).
    pub handle: Option<Handle>,
    /// Side of a position's entry line: dragging it out previews a target or a stop.
    pub side: Option<Side>,
}

struct Frame {
    plot: Rect,
    right: f64,
    bar_px: f32,
    lo: f64,
    hi: f64,
    ppp: f32,
}

impl Frame {
    fn x(&self, i: f64) -> f32 {
        self.plot.right() - ((self.right - i) as f32) * self.bar_px
    }
    fn index_at(&self, x: f32) -> f64 {
        self.right - ((self.plot.right() - x) / self.bar_px) as f64
    }
    fn y(&self, price: f64) -> f32 {
        self.plot.bottom() - ((price - self.lo) / (self.hi - self.lo)) as f32 * self.plot.height()
    }
    fn price_at(&self, y: f32) -> f64 {
        self.lo + ((self.plot.bottom() - y) / self.plot.height()) as f64 * (self.hi - self.lo)
    }
    /// Snap to the physical pixel grid; `center` puts 1px lines on pixel centers.
    fn snap(&self, v: f32, center: bool) -> f32 {
        let half = if center { 0.5 } else { 0.0 };
        ((v * self.ppp).floor() + half) / self.ppp
    }
}

impl ChartView {
    /// Forget the view (new symbol or timeframe): follow the last bar with auto-scale.
    pub fn reset(&mut self) {
        *self = Self { bar_px: self.bar_px, cursor: self.cursor, ..Self::default() };
    }

    /// Center a loaded bar, keeping the zoom and restoring price auto-scale.
    pub fn go_to(&mut self, time: i64) {
        self.target_time = Some(time);
        self.follow = false;
        self.auto_y = true;
        self.measurement = None;
        self.measuring = false;
        self.drag = None;
        self.level_drag = None;
        self.menu_price = None;
        self.menu_at = None;
    }

    fn set_cursor(&mut self, cursor: CursorMode) {
        if self.cursor != cursor {
            self.cursor = cursor;
            self.measurement = None;
            self.measuring = false;
            self.drag = None;
            self.level_drag = None;
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, data: &ChartData, pal: &Palette) {
        self.set_cursor(data.cursor);
        let rect = ui.available_rect_before_wrap();
        let response = ui.allocate_rect(rect, Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        let ppp = ui.ctx().pixels_per_point();
        painter.rect_filled(rect, 0.0, pal.chart_bg);

        let pane_h = match (data.pane.is_some(), data.pane_open) {
            (false, _) => 0.0,
            (true, true) => PANE_H.min(rect.height() * 0.4),
            (true, false) => PANE_MIN_H,
        };
        let plot = Rect::from_min_max(rect.min, Pos2::new(rect.right() - PRICE_AXIS_W, rect.bottom() - TIME_AXIS_H - pane_h));
        let pane_rect = Rect::from_min_max(Pos2::new(rect.left(), plot.bottom()), Pos2::new(plot.right(), plot.bottom() + pane_h));
        let price_axis = Rect::from_min_max(Pos2::new(plot.right(), rect.top()), Pos2::new(rect.right(), plot.bottom()));
        let time_axis = Rect::from_min_max(Pos2::new(rect.left(), rect.bottom() - TIME_AXIS_H), rect.max);

        let bars = &data.series.bars;
        let quote = valid_quote(data.quote);
        let trade_quote = quote.filter(|_| data.can_trade);
        let n = bars.len();
        if n == 0 {
            painter.text(plot.center(), Align2::CENTER_CENTER, "Aguardando dados…", FontId::proportional(15.0), pal.text_dim);
            return;
        }

        // --- older bars loaded in front: the same bars stay on screen -----------------------------
        if self.known_len > 0 && bars[0].time < self.known_first {
            let added = bars.partition_point(|b| b.time < self.known_first);
            self.right += added as f64;
            self.known_len += added;
            if let Some(m) = &mut self.measurement {
                m.start.bar += added;
                m.end.bar += added;
            }
        }
        self.known_first = bars[0].time;

        // --- keep following the market when new bars arrive -------------------------------------
        if self.known_len == 0 {
            self.right = n as f64 - 1.0 + RIGHT_PAD;
        } else if n > self.known_len && self.follow && !self.measuring {
            self.right += (n - self.known_len) as f64;
        }
        self.known_len = n;
        if let Some(time) = self.target_time.take() {
            let index = bars.partition_point(|b| b.time < time).min(n - 1);
            self.right = index as f64 + (plot.width() / self.bar_px) as f64 / 2.0;
        }

        // --- input --------------------------------------------------------------------------------
        let hover = response.hover_pos();
        let zone_at = |p: Pos2| {
            if price_axis.contains(p) {
                Zone::PriceAxis
            } else if time_axis.contains(p) {
                Zone::TimeAxis
            } else {
                Zone::Plot
            }
        };
        self.actions.clear();
        // price under a y, with the range the user sees at the start of this frame
        let (lo0, hi0) = (self.y_lo, self.y_hi);
        let price_at = |y: f32| lo0 + ((plot.bottom() - y) / plot.height()) as f64 * (hi0 - lo0);
        let y_at = |price: f64| plot.bottom() - ((price - lo0) / (hi0 - lo0)) as f32 * plot.height();
        let grab_within = |p: Pos2, px: f32| -> Option<Handle> {
            if !plot.contains(p) {
                return None;
            }
            data.levels
                .iter()
                .filter_map(|l| l.handle.map(|h| (h, (y_at(l.price) - p.y).abs())))
                .filter(|(_, d)| *d <= px)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(h, _)| h)
        };
        let grab = |p: Pos2| grab_within(p, GRAB_PX);
        // Alt dragging edits trades; otherwise the selected cursor controls plot gestures.
        let alt = ui.input(|i| i.modifiers.alt);
        let alt_grab = |p: Pos2| {
            if alt { grab_within(p, ALT_GRAB_PX) } else { None
            }
        };
        // the × of a removable line under the pointer
        let remove_at = |p: Pos2| -> Option<Handle> {
            if p.x < plot.left() + REMOVE_X || p.x > plot.left() + REMOVE_X + REMOVE_W {
                return None;
            }
            data.levels
                .iter()
                .filter_map(|l| {
                    l.handle.filter(|h| removable(*h)).map(|h| (h, (y_at(l.price) - p.y).abs()))
                })
                .filter(|(_, d)| *d <= REMOVE_W / 2.0)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(h, _)| h)
        };
        // Shift (buy) or Ctrl (sell) held: the order follows the pointer until the click places it
        let mods = ui.input(|i| i.modifiers);
        let ghost_side = match (mods.shift, mods.ctrl) {
            (true, false) => Some(Side::Buy),
            (false, true) => Some(Side::Sell),
            _ => None,
        };
        let snap_price = |price: f64| {
            if data.tick > 0.0 { (price / data.tick).round() * data.tick } else { price
            }
        };
        let ghost = match (ghost_side, hover.filter(|p| plot.contains(*p)), trade_quote) {
            (Some(side), Some(p), Some((bid, ask))) if self.level_drag.is_none() && self.drag.is_none() && !self.measuring => {
                let price = snap_price(price_at(p.y));
                let kind = match side {
                    Side::Buy if price < ask => OrderKind::Limit,
                    Side::Sell if price > bid => OrderKind::Limit,
                    _ => OrderKind::Stop,
                };
                Some((side, kind, price))
            }
            _ => None,
        };
        if let Some(h) = hover.and_then(grab).filter(|h| removable(*h))
            && ui.input(|i| i.key_pressed(egui::Key::Delete))
        {
            self.actions.push(ChartAction::Remove { handle: h });
        }
        if let Some(p) = hover {
            ui.ctx().set_cursor_icon(match zone_at(p) {
                _ if self.level_drag.is_some() => CursorIcon::Grabbing,
                _ if remove_at(p).is_some() && self.cursor != CursorMode::Cross => {
                    CursorIcon::PointingHand
                }
                _ if ghost.is_some() => CursorIcon::Crosshair,
                Zone::Plot if self.drag.is_none() && alt_grab(p).is_some() => {
                    CursorIcon::ResizeVertical
                }
                Zone::PriceAxis => CursorIcon::ResizeVertical,
                Zone::TimeAxis => CursorIcon::ResizeHorizontal,
                Zone::Plot if self.drag == Some(Zone::Plot) => CursorIcon::Grabbing,
                Zone::Plot => self.cursor.icon(),
            });
        }

        // y range before this frame's input, needed to unlock auto-scale smoothly
        let (auto_lo, auto_hi) = self.auto_range(bars, data.series.last, quote, plot.width());
        if self.auto_y && !self.measuring {
            (self.y_lo, self.y_hi) = (auto_lo, auto_hi);
        }

        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.measurement = None;
            self.measuring = false;
        }
        let measure_point = |p: Pos2| MeasurePoint {
            bar: (self.right - ((plot.right() - p.x.clamp(plot.left(), plot.right())) / self.bar_px) as f64)
                .round().clamp(0.0, (n - 1) as f64) as usize,
            price: snap_price(price_at(p.y.clamp(plot.top(), plot.bottom()))),
        };
        if response.drag_started_by(egui::PointerButton::Primary) {
            // where the button went down: egui reports a drag only after the pointer has moved a few
            // pixels, by then off a thin line (a fast hand would pan instead of grabbing it)
            let at = ui.input(|i| i.pointer.press_origin()).or_else(|| response.interact_pointer_pos());
            match at.and_then(|p| alt_grab(p).map(|h| (h, price_at(p.y)))) {
                Some(grabbed) => self.level_drag = Some(grabbed),
                None => match at.map(zone_at) {
                    Some(Zone::Plot) => {
                        if ghost.is_none() && let Some(at) = at.filter(|p| plot.contains(*p)) {
                            match self.cursor {
                                CursorMode::Hand => self.drag = Some(Zone::Plot),
                                CursorMode::Cross => {
                                    let start = measure_point(at);
                                    self.measurement = Some(Measurement { start, end: start });
                                    self.measuring = true;
                                }
                                CursorMode::Arrow => {}
                            }
                        }
                    }
                    zone => self.drag = zone,
                },
            }
            self.drag_dy = 0.0;
        }
        if self.measuring {
            if let Some(p) = response.interact_pointer_pos()
                && let Some(m) = &mut self.measurement
            {
                m.end = measure_point(p);
            }
            if response.drag_stopped_by(egui::PointerButton::Primary) {
                self.measuring = false;
            }
        }
        if let Some((h, _)) = self.level_drag {
            if let Some(p) = response.interact_pointer_pos().or(hover) {
                self.level_drag = Some((h, price_at(p.y)));
            }
            if response.drag_stopped()
                && let Some((handle, price)) = self.level_drag.take()
            {
                self.actions.push(ChartAction::Move { handle, price });
            }
        }
        // a click on a × removes; with Shift/Ctrl held it places the order following the pointer
        // (ProfitChart: Shift buys, Ctrl sells; limit on the favorable side of the quote, stop beyond)
        if response.clicked() && self.cursor == CursorMode::Cross && ghost.is_none() && !mods.alt {
            if response.interact_pointer_pos().is_some_and(|p| plot.contains(p)) {
                self.measurement = None;
            }
        } else if response.clicked() {
            match response.interact_pointer_pos().and_then(remove_at) {
                Some(handle) => self.actions.push(ChartAction::Remove { handle }),
                None => {
                    if let Some((side, kind, price)) = ghost {
                        self.actions.push(ChartAction::Order { side, kind, price });
                    }
                }
            }
        }
        // right click: the indicator under the pointer, and a pending order at that price
        if response.secondary_clicked() {
            self.menu_at = response.interact_pointer_pos();
            self.menu_price = self.menu_at.filter(|p| plot.contains(*p)).map(|p| price_at(p.y));
        }
        if response.dragged() && self.level_drag.is_none() {
            let d = response.drag_delta();
            match self.drag {
                Some(Zone::Plot) => {
                    self.right -= (d.x / self.bar_px) as f64;
                    self.drag_dy += d.y;
                    if self.auto_y && self.drag_dy.abs() > UNLOCK_Y_PX {
                        self.auto_y = false;
                    }
                    if !self.auto_y {
                        let per_px = (self.y_hi - self.y_lo) / plot.height() as f64;
                        self.y_lo += d.y as f64 * per_px;
                        self.y_hi += d.y as f64 * per_px;
                    }
                }
                Some(Zone::PriceAxis) => {
                    // drag down compresses, up stretches, around the middle of the range
                    let f = (d.y as f64 * 0.006).exp();
                    let mid = (self.y_lo + self.y_hi) / 2.0;
                    let half = (self.y_hi - self.y_lo) / 2.0 * f;
                    self.auto_y = false;
                    (self.y_lo, self.y_hi) = (mid - half, mid + half);
                }
                Some(Zone::TimeAxis) => {
                    // stretch horizontally, anchored at the right edge (like TradingView)
                    self.bar_px = (self.bar_px * (d.x * 0.006).exp()).clamp(MIN_BAR_PX, MAX_BAR_PX);
                }
                None => {}
            }
        }
        if response.drag_stopped() {
            self.drag = None;
        }
        if response.double_clicked() {
            match response.interact_pointer_pos().map(zone_at) {
                Some(Zone::PriceAxis) => self.auto_y = true,
                Some(Zone::TimeAxis) => {
                    self.bar_px = 8.0;
                    self.right = n as f64 - 1.0 + RIGHT_PAD;
                }
                _ => {}
            }
        }
        if let Some(p) = hover {
            let scroll = ui.input(|i| i.smooth_scroll_delta);
            if scroll != Vec2::ZERO {
                let shift = ui.input(|i| i.modifiers.shift);
                if shift || scroll.x.abs() > scroll.y.abs() {
                    // horizontal scroll pans
                    let dx = if shift { scroll.y } else { scroll.x };
                    self.right -= (dx / self.bar_px) as f64;
                } else if zone_at(p) == Zone::PriceAxis {
                    let f = (-scroll.y as f64 * 0.003).exp();
                    let mid = (self.y_lo + self.y_hi) / 2.0;
                    let half = (self.y_hi - self.y_lo) / 2.0 * f;
                    self.auto_y = false;
                    (self.y_lo, self.y_hi) = (mid - half, mid + half);
                } else {
                    // zoom around the bar under the cursor
                    let anchor_x = if zone_at(p) == Zone::Plot { p.x } else { plot.right() };
                    let idx = self.right - ((plot.right() - anchor_x) / self.bar_px) as f64;
                    self.bar_px = (self.bar_px * (scroll.y * 0.0025).exp()).clamp(MIN_BAR_PX, MAX_BAR_PX);
                    self.right = idx + ((plot.right() - anchor_x) / self.bar_px) as f64;
                }
            }
        }

        // keyboard: arrows pan a tenth of the screen, +/- zoom at the right edge (not while typing)
        if !ui.ctx().egui_wants_keyboard_input()
            && (response.hovered() || ui.ctx().memory(|m| m.focused().is_none()))
        {
            let (back, fwd, zin, zout) = ui.input(|i| {
                (
                    i.key_pressed(egui::Key::ArrowLeft),
                    i.key_pressed(egui::Key::ArrowRight),
                    i.key_pressed(egui::Key::Plus) || i.key_pressed(egui::Key::Equals),
                    i.key_pressed(egui::Key::Minus),
                )
            });
            let step = (plot.width() / self.bar_px) as f64 * 0.1;
            if back {
                self.right -= step;
            }
            if fwd {
                self.right += step;
            }
            if zin || zout {
                self.bar_px = (self.bar_px * if zin { 1.25 } else { 0.8 }).clamp(MIN_BAR_PX, MAX_BAR_PX);
            }
        }

        // keep at least a few bars on screen
        let visible = (plot.width() / self.bar_px) as f64;
        self.right = self.right.clamp(4.0_f64.min(n as f64 - 1.0), n as f64 - 1.0 + visible * 0.85);
        self.follow = self.right >= n as f64 - 1.0;
        // left edge within a screen of the oldest bar (scrolled or zoomed back)
        self.wants_older = self.right - 2.0 * visible < 0.0;
        if self.auto_y && !self.measuring {
            let (lo, hi) = self.auto_range(bars, data.series.last, quote, plot.width());
            (self.y_lo, self.y_hi) = (lo, hi);
        }
        if !(self.y_hi - self.y_lo).is_normal() || self.y_hi <= self.y_lo {
            self.y_hi = self.y_lo + 1.0;
        }

        let f = Frame { plot, right: self.right, bar_px: self.bar_px, lo: self.y_lo, hi: self.y_hi, ppp };
        let i0 = (f.index_at(plot.left()).floor() as isize).max(0) as usize;
        let i1 = (f.right.ceil() as isize).clamp(0, n as isize - 1) as usize;
        let digits = data.series.digits;
        if let Some(p) = self.menu_at.take() {
            let base = plot.bottom() - data.ribbon.as_ref().map(|r| r.height()).unwrap_or(0.0);
            let vol_top = base - plot.height() * 0.16;
            self.menu_study = study_at(&f, data, p, (data.pane.is_some() && data.pane_open).then_some(pane_rect), vol_top, base);
        }

        // --- grid + axes --------------------------------------------------------------------------
        let step = axis::nice_step((f.hi - f.lo) / (plot.height() as f64 / 56.0).max(1.0));
        let decimals = axis::decimals_for(step, digits);
        let label_font = FontId::proportional(11.5);
        for price in axis::price_ticks(f.lo, f.hi, step) {
            let y = f.snap(f.y(price), true);
            if y < plot.top() + 4.0 {
                continue;
            }
            painter.hline(plot.x_range(), y, Stroke::new(1.0 / ppp, pal.grid));
            painter.text(
                Pos2::new(price_axis.left() + 8.0, y),
                Align2::LEFT_CENTER,
                format!("{price:.decimals$}"),
                label_font.clone(),
                pal.text_dim,
            );
        }

        let secs_per_px = data.tf.seconds() as f64 / self.bar_px as f64;
        let tstep = axis::time_step(secs_per_px, 96.0);
        let mut last_label_x = f32::NEG_INFINITY;
        for i in i0.max(1)..=i1 {
            let (t, prev) = (bars[i].time, bars[i - 1].time);
            if axis::time_bucket(t, tstep) == axis::time_bucket(prev, tstep) {
                continue;
            }
            let x = f.snap(f.x(i as f64), true);
            if x - last_label_x < 70.0 || x < plot.left() + 28.0 {
                continue;
            }
            last_label_x = x;
            let (text, major) = axis::time_label(t, Some(prev), tstep);
            painter.vline(x, plot.y_range(), Stroke::new(1.0 / ppp, pal.grid));
            painter.text(
                Pos2::new(x, time_axis.center().y),
                Align2::CENTER_CENTER,
                text,
                if major { FontId::proportional(11.5) } else { label_font.clone() },
                if major { pal.text } else { pal.text_dim },
            );
        }
        painter.vline(f.snap(plot.right(), true), rect.y_range(), Stroke::new(1.0 / ppp, pal.border));
        painter.hline(rect.x_range(), f.snap(plot.bottom(), true), Stroke::new(1.0 / ppp, pal.border));

        // --- volume + candles (one mesh each) -----------------------------------------------------
        let plot_painter = painter.with_clip_rect(plot);
        // tint behind everything: one rect per run of bars of the same class
        if let Some(sh) = &data.shading {
            let mut mesh = Mesh::default();
            let last = i1.min(sh.classes.len().saturating_sub(1));
            let mut i = i0;
            while i <= last && !sh.classes.is_empty() {
                let c = sh.classes[i];
                let mut j = i;
                while j < last && sh.classes[j + 1] == c {
                    j += 1;
                }
                if let Some(&color) = sh.palette.get(c as usize) {
                    let (xa, xb) = (f.x(i as f64 - 0.5), f.x(j as f64 + 0.5));
                    mesh.add_colored_rect(Rect::from_min_max(Pos2::new(xa, plot.top()), Pos2::new(xb, plot.bottom())), color);
                }
                i = j + 1;
            }
            plot_painter.add(Shape::mesh(mesh));
        }
        let max_vol = if data.volume.is_some() { 0.0 } else { bars[i0..=i1].iter().map(|b| b.volume).fold(0.0, f64::max) };
        let vol_h = plot.height() * 0.16;
        // the volume stands on the ribbon, if any
        let base = plot.bottom() - data.ribbon.as_ref().map(|r| r.height()).unwrap_or(0.0);
        let body_w = if self.bar_px >= 3.0 { (self.bar_px * 0.72).max(1.0) } else { 0.0 };
        let wick_w = (1.0 / ppp).max((self.bar_px * 0.08).min(2.0));
        let mut vol_mesh = Mesh::default();
        let mut mesh = Mesh::default();
        mesh.reserve_vertices((i1 + 1 - i0) * 8);
        for (i, b) in bars.iter().enumerate().take(i1 + 1).skip(i0) {
            let up = b.close >= b.open;
            let (body, wick) = if up { (pal.candle_up, pal.wick_up) } else { (pal.candle_down, pal.wick_down) };
            let cx = f.x(i as f64);
            if max_vol > 0.0 {
                let h = (b.volume / max_vol) as f32 * vol_h;
                let w = (self.bar_px * 0.72).max(1.0 / ppp);
                vol_mesh.add_colored_rect(
                    Rect::from_min_max(
                        Pos2::new(f.snap(cx - w / 2.0, false), base - h),
                        Pos2::new(f.snap(cx + w / 2.0, false).max(f.snap(cx - w / 2.0, false) + 1.0 / ppp), base),
                    ),
                    if up { pal.up_vol } else { pal.down_vol },
                );
            }
            let (yh, yl) = (f.y(b.high), f.y(b.low));
            let wx = f.snap(cx - wick_w / 2.0, false);
            mesh.add_colored_rect(Rect::from_min_max(Pos2::new(wx, yh), Pos2::new(wx + wick_w, yl.max(yh + 1.0 / ppp))), wick);
            if body_w > 0.0 {
                let (yo, yc) = (f.y(b.open), f.y(b.close));
                let (top, bottom) = (yo.min(yc), yo.max(yc).max(yo.min(yc) + 1.0 / ppp));
                let left = f.snap(cx - body_w / 2.0, false);
                let right = f.snap(cx + body_w / 2.0, false).max(left + 1.0 / ppp);
                let rect = Rect::from_min_max(Pos2::new(left, top), Pos2::new(right, bottom));
                // body unlike its outline (MT5 bar color): an outline, then the body inside it
                let edge = 1.0 / ppp;
                if body != wick && rect.width() > 2.0 * edge && rect.height() > 2.0 * edge {
                    mesh.add_colored_rect(rect, wick);
                    mesh.add_colored_rect(rect.shrink(edge), body);
                } else {
                    mesh.add_colored_rect(rect, body);
                }
            }
        }
        // a band in place of the volume (e.g. delta): drawn like the volume, up from the bottom with
        // the same scale and transparency; the size is |value|, the color its sign
        if let Some(band) = &data.volume {
            let last = i1.min(band.values.len().saturating_sub(1));
            let top = band.values.get(i0..=last).unwrap_or(&[]).iter().filter(|x| x.is_finite()).fold(0.0f64, |m, x| m.max(x.abs()));
            if top > 0.0 {
                let w = (self.bar_px * 0.72).max(1.0 / ppp);
                let faded = band.palette.map(|c| Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), 60));
                for i in i0..=last {
                    let v = band.values[i];
                    if !v.is_finite() {
                        continue;
                    }
                    let h = (v.abs() / top) as f32 * vol_h;
                    let cx = f.x(i as f64);
                    let left = f.snap(cx - w / 2.0, false);
                    let right = f.snap(cx + w / 2.0, false).max(left + 1.0 / ppp);
                    let c = faded[if v > 0.0 { 0 } else if v < 0.0 { 1 } else { 2 }];
                    vol_mesh.add_colored_rect(Rect::from_min_max(Pos2::new(left, base - h), Pos2::new(right, base)), c);
                }
            }
        }
        // volume is background context: always behind every layer
        plot_painter.add(Shape::mesh(vol_mesh));

        // --- layers, back to front -----------------------------------------------------------------
        let mut candles = Some(mesh);
        for layer in data.layers {
            match layer {
                Layer::Price => {
                    if let Some(mesh) = candles.take() {
                        plot_painter.add(Shape::mesh(mesh));
                    }
                    // marks on the candles (POC): a dash that follows the bar width, never wider
                    // than a body nor thicker than a few pixels, so neighbors don't merge
                    if let Some(m) = &data.marks {
                        let w = (self.bar_px * 0.75).clamp(2.0, 10.0);
                        let h = (self.bar_px * 0.2).clamp(1.0, 2.5);
                        let mut mesh = Mesh::default();
                        for i in i0..=i1.min(m.prices.len().saturating_sub(1)) {
                            let p = m.prices[i];
                            if !p.is_finite() {
                                continue;
                            }
                            let (cx, cy) = (f.x(i as f64), f.y(p));
                            mesh.add_colored_rect(Rect::from_center_size(Pos2::new(cx, cy), Vec2::new(w, h)), m.color);
                        }
                        plot_painter.add(Shape::mesh(mesh));
                    }
                    draw_prices(&painter, &f, price_axis, data, pal);
                }
                Layer::Indicators => {
                    for o in data.overlays {
                        draw_overlay(&plot_painter, &f, o, i0.saturating_sub(1), i1.min(o.values.len().saturating_sub(1)));
                    }
                }
                Layer::Levels => {
                    for (index, m) in data.map_levels.iter().enumerate() {
                        let y = f.snap(f.y(m.price), true);
                        if !plot.y_range().contains(y) {
                            continue;
                        }
                        let start = bars.partition_point(|b| b.time < m.from);
                        let x0 = f.x(start as f64).max(plot.left());
                        let stroke = Stroke::new(m.width, m.color);
                        let line = [Pos2::new(x0, y), Pos2::new(plot.right(), y)];
                        if m.dashed {
                            plot_painter.extend(Shape::dashed_line(&line, stroke, 7.0, 4.0));
                        } else {
                            plot_painter.line_segment(line, stroke);
                        }
                        let at = Pos2::new(plot.left() + 4.0, y - 2.0);
                        let label = plot_painter.text(at, Align2::LEFT_BOTTOM, &m.text, FontId::proportional(11.0), m.color);
                        ui.interact(
                            label.intersect(plot),
                            ui.id().with(("map-level", index)),
                            Sense::hover(),
                        )
                        .on_hover_text(&m.details);
                    }
                }
                Layer::Trades => {
                    for level in data.levels {
                        let dragged = self.level_drag.filter(|(h, _)| Some(*h) == level.handle).map(|(_, p)| p);
                        // a position stays put: dragging out of it previews the target or stop it creates
                        if let (Some(price), Some(side)) = (dragged, level.side) {
                            let gain = if side == Side::Buy { price > level.price } else { price < level.price };
                            let (color, name) = if gain { (pal.ok, "Alvo") } else { (pal.danger, "Stop") };
                            let y = f.snap(f.y(price), true);
                            plot_painter.extend(Shape::dashed_line(&[Pos2::new(plot.left(), y), Pos2::new(plot.right(), y)], Stroke::new(1.5, color), 8.0, 4.0));
                            let text = format!("{name} @ {price:.d$}  (solte para posicionar)", d = digits as usize);
                            let galley = painter.layout_no_wrap(text, FontId::proportional(11.5), Color32::WHITE);
                            let tag = Rect::from_min_size(Pos2::new(plot.left() + 70.0, y - 9.0), Vec2::new(galley.size().x + 12.0, 18.0));
                            plot_painter.rect_filled(tag, CornerRadius::same(3), color);
                            plot_painter.galley(tag.center() - galley.size() / 2.0, galley, Color32::WHITE);
                            let ptag = Rect::from_min_size(Pos2::new(price_axis.left() + 1.0, y - 9.5), Vec2::new(PRICE_AXIS_W - 2.0, 19.0));
                            painter.rect_filled(ptag, CornerRadius::same(3), color);
                            painter.text(Pos2::new(ptag.left() + 7.0, y), Align2::LEFT_CENTER, format!("{price:.d$}", d = digits as usize), FontId::proportional(11.5), Color32::WHITE);
                        }
                        let dragged = dragged.filter(|_| level.side.is_none());
                        if dragged.is_some() {
                            let y = f.snap(f.y(level.price), true);
                            plot_painter.extend(Shape::dashed_line(
                                &[Pos2::new(plot.left(), y), Pos2::new(plot.right(), y)],
                                Stroke::new(1.0, level.color.gamma_multiply(0.35)),
                                4.0,
                                4.0,
                            ));
                        }
                        let price = dragged.unwrap_or(level.price);
                        let y = f.snap(f.y(price), true);
                        if !plot.y_range().contains(y) {
                            continue;
                        }
                        let stroke = Stroke::new(1.0, level.color);
                        let line = [Pos2::new(plot.left(), y), Pos2::new(plot.right(), y)];
                        if level.dashed {
                            plot_painter.extend(Shape::dashed_line(&line, stroke, 6.0, 4.0));
                        } else {
                            plot_painter.line_segment(line, stroke);
                        }
                        let galley = painter.layout_no_wrap(level.label.clone(), FontId::proportional(11.0), Color32::WHITE);
                        let tag = Rect::from_min_size(Pos2::new(plot.left() + 70.0, y - 9.0), Vec2::new(galley.size().x + 12.0, 18.0));
                        plot_painter.rect_filled(tag, CornerRadius::same(3), level.color);
                        plot_painter.galley(tag.center() - galley.size() / 2.0, galley, Color32::WHITE);
                        if level.handle.is_some_and(removable) && dragged.is_none() {
                            let x = Rect::from_min_size(Pos2::new(plot.left() + REMOVE_X, y - REMOVE_W / 2.0), Vec2::splat(REMOVE_W));
                            let hot = hover.is_some_and(|p| x.contains(p));
                            plot_painter.rect_filled(x, CornerRadius::same(3), if hot { pal.danger } else { level.color });
                            let (a, b) = (x.shrink(5.0).left_top(), x.shrink(5.0).right_bottom());
                            let stroke = Stroke::new(1.6, Color32::WHITE);
                            plot_painter.line_segment([a, b], stroke);
                            plot_painter.line_segment([Pos2::new(a.x, b.y), Pos2::new(b.x, a.y)], stroke);
                        }
                        let ptag = Rect::from_min_size(Pos2::new(price_axis.left() + 1.0, y - 9.5), Vec2::new(PRICE_AXIS_W - 2.0, 19.0));
                        painter.rect_filled(ptag, CornerRadius::same(3), level.color);
                        painter.text(
                            Pos2::new(ptag.left() + 7.0, y),
                            Align2::LEFT_CENTER,
                            format!("{:.prec$}", price, prec = digits as usize),
                            FontId::proportional(11.5),
                            Color32::WHITE,
                        );
                    }
                }
            }
        }
        // a layer list without the price still shows the candles
        if let Some(mesh) = candles.take() {
            plot_painter.add(Shape::mesh(mesh));
        }
        // the ribbon: its own strip under everything drawn in the plot, one mesh, runs of one class
        if let Some(r) = data.ribbon.as_ref().filter(|r| !r.rows.is_empty()) {
            let strip = Rect::from_min_max(Pos2::new(plot.left(), base), plot.max);
            plot_painter.rect_filled(strip, 0.0, pal.chart_bg);
            let mut mesh = Mesh::default();
            for (k, row) in r.rows.iter().enumerate() {
                let top = base + RIBBON_GAP + k as f32 * (RIBBON_ROW_H + RIBBON_GAP);
                let last = i1.min(row.classes.len().saturating_sub(1));
                let mut i = i0;
                while i <= last && !row.classes.is_empty() {
                    let c = row.classes[i];
                    let mut j = i;
                    while j < last && row.classes[j + 1] == c {
                        j += 1;
                    }
                    if let Some(&color) = row.palette.get(c as usize) {
                        let (xa, xb) = (f.x(i as f64 - 0.5), f.x(j as f64 + 0.5));
                        mesh.add_colored_rect(Rect::from_min_max(Pos2::new(xa, top), Pos2::new(xb, top + RIBBON_ROW_H)), color);
                    }
                    i = j + 1;
                }
            }
            plot_painter.add(Shape::mesh(mesh));
            for (k, row) in r.rows.iter().enumerate() {
                let y = base + RIBBON_GAP + k as f32 * (RIBBON_ROW_H + RIBBON_GAP) + RIBBON_ROW_H / 2.0;
                let galley = plot_painter.layout_no_wrap(row.label.to_string(), FontId::proportional(9.0), pal.text);
                let tag = Rect::from_min_size(Pos2::new(plot.left() + 2.0, y - galley.size().y / 2.0), galley.size() + Vec2::new(4.0, 0.0));
                plot_painter.rect_filled(tag, 0.0, pal.chart_bg);
                plot_painter.galley(tag.min + Vec2::new(2.0, 0.0), galley, pal.text_dim);
            }
        }
        // the order following the pointer, on top of everything
        if let Some((side, kind, price)) = ghost {
            let color = if side == Side::Buy { pal.up } else { pal.down };
            let y = f.snap(f.y(price), true);
            plot_painter.extend(Shape::dashed_line(
                &[Pos2::new(plot.left(), y), Pos2::new(plot.right(), y)],
                Stroke::new(1.5, color),
                8.0,
                4.0,
            ));
            let text = format!(
                "{} {} {} @ {:.d$}  (clique para posicionar)",
                if side == Side::Buy { "Compra" } else { "Venda" },
                if kind == OrderKind::Limit { "limite" } else { "stop" },
                data.order_volume,
                price,
                d = digits as usize
            );
            let galley = painter.layout_no_wrap(text, FontId::proportional(11.5), Color32::WHITE);
            let tag = Rect::from_min_size(Pos2::new(plot.left() + 70.0, y - 20.0), Vec2::new(galley.size().x + 12.0, 18.0));
            plot_painter.rect_filled(tag, CornerRadius::same(3), color);
            plot_painter.galley(tag.center() - galley.size() / 2.0, galley, Color32::WHITE);
            let ptag = Rect::from_min_size(Pos2::new(price_axis.left() + 1.0, y - 9.5), Vec2::new(PRICE_AXIS_W - 2.0, 19.0));
            painter.rect_filled(ptag, CornerRadius::same(3), color);
            painter.text(Pos2::new(ptag.left() + 7.0, y), Align2::LEFT_CENTER, format!("{price:.d$}", d = digits as usize), FontId::proportional(11.5), Color32::WHITE);
            // its stop and target, where they will be placed
            if let Some(b) = data.bracket {
                let (sl, tp) = b.levels(side, price, data.tick);
                for (level, offset, sign, name, c) in [(sl, b.stop, '-', "Stop", pal.danger), (tp, b.target, '+', "Alvo", pal.ok)] {
                    if level <= 0.0 {
                        continue;
                    }
                    let y = f.snap(f.y(level), true);
                    plot_painter.extend(Shape::dashed_line(&[Pos2::new(plot.left(), y), Pos2::new(plot.right(), y)], Stroke::new(1.2, c), 5.0, 4.0));
                    let text = format!("{name} {} @ {level:.d$}", offset.label(sign), d = digits as usize);
                    let galley = painter.layout_no_wrap(text, FontId::proportional(11.0), Color32::WHITE);
                    let tag = Rect::from_min_size(Pos2::new(plot.left() + 70.0, y - 9.0), Vec2::new(galley.size().x + 12.0, 18.0));
                    plot_painter.rect_filled(tag, CornerRadius::same(3), c);
                    plot_painter.galley(tag.center() - galley.size() / 2.0, galley, Color32::WHITE);
                    let ptag = Rect::from_min_size(Pos2::new(price_axis.left() + 1.0, y - 9.5), Vec2::new(PRICE_AXIS_W - 2.0, 19.0));
                    painter.rect_filled(ptag, CornerRadius::same(3), c);
                    painter.text(Pos2::new(ptag.left() + 7.0, y), Align2::LEFT_CENTER, format!("{level:.d$}", d = digits as usize), FontId::proportional(11.5), Color32::WHITE);
                }
            }
        }
        if let Some(pane) = data.pane {
            // the pane's corner button: "minimizar" when open, the whole strip restores it
            let pp = painter.with_clip_rect(pane_rect);
            let button = if data.pane_open {
                self.draw_pane(&pp, pane_rect, &f, pane, i0, i1, pal);
                let r = Rect::from_min_size(Pos2::new(pane_rect.right() - 86.0, pane_rect.top() + 4.0), Vec2::new(80.0, 18.0));
                let hot = hover.is_some_and(|p| r.contains(p));
                pp.rect_filled(r, CornerRadius::same(3), if hot { pal.tag_bg } else { pal.panel_bg });
                pp.text(r.center(), Align2::CENTER_CENTER, "minimizar", FontId::proportional(11.0), if hot { pal.text } else { pal.text_dim });
                r
            } else {
                let hot = hover.is_some_and(|p| pane_rect.contains(p));
                pp.rect_filled(pane_rect, 0.0, if hot { pal.tag_bg } else { pal.panel_bg });
                pp.hline(pane_rect.x_range(), f.snap(pane_rect.top(), true), Stroke::new(1.0 / ppp, pal.border));
                pp.text(Pos2::new(pane_rect.left() + 12.0, pane_rect.center().y), Align2::LEFT_CENTER, "Painel minimizado · abrir painel", FontId::proportional(11.0), if hot { pal.text } else { pal.text_dim });
                pane_rect
            };
            if hover.is_some_and(|p| button.contains(p)) {
                ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
                if response.clicked() {
                    self.actions.push(ChartAction::TogglePane);
                }
            }
        }

        // --- crosshair + legend -------------------------------------------------------------------
        let hovered_bar = hover.filter(|p| plot.contains(*p)).map(|p| {
            let i = f.index_at(p.x).round().clamp(0.0, (n - 1) as f64) as usize;
            (p, i)
        });
        if let Some((p, i)) = hovered_bar.filter(|_| self.cursor == CursorMode::Cross && ghost.is_none()) {
            let x = f.snap(f.x(i as f64), true);
            let y = f.snap(p.y, true);
            let stroke = Stroke::new(1.0 / ppp, pal.crosshair);
            plot_painter.extend(Shape::dashed_line(&[Pos2::new(x, plot.top()), Pos2::new(x, plot.bottom())], stroke, 4.0, 4.0));
            if pane_h > 0.0 {
                let line = [Pos2::new(x, pane_rect.top()), Pos2::new(x, pane_rect.bottom())];
                painter.with_clip_rect(pane_rect).extend(Shape::dashed_line(&line, stroke, 4.0, 4.0));
            }
            plot_painter.extend(Shape::dashed_line(&[Pos2::new(plot.left(), y), Pos2::new(plot.right(), y)], stroke, 4.0, 4.0));

            let price_tag = Rect::from_min_size(Pos2::new(price_axis.left() + 1.0, y - 9.5), Vec2::new(PRICE_AXIS_W - 2.0, 19.0));
            painter.rect_filled(price_tag, CornerRadius::same(3), pal.tag_bg);
            painter.text(
                Pos2::new(price_tag.left() + 7.0, y),
                Align2::LEFT_CENTER,
                format!("{:.prec$}", f.price_at(p.y), prec = decimals.max(digits as usize)),
                FontId::proportional(11.5),
                pal.text,
            );
            let label = axis::crosshair_label(bars[i].time, data.tf != Timeframe::D1);
            let galley = painter.layout_no_wrap(label, FontId::proportional(11.5), pal.text);
            let w = galley.size().x + 16.0;
            let tx = (x - w / 2.0).clamp(rect.left(), plot.right() - w);
            let time_tag = Rect::from_min_size(Pos2::new(tx, time_axis.top() + 2.0), Vec2::new(w, TIME_AXIS_H - 4.0));
            painter.rect_filled(time_tag, CornerRadius::same(3), pal.tag_bg);
            painter.galley(time_tag.center() - galley.size() / 2.0, galley, pal.text);
        }

        let li = hovered_bar.map(|(_, i)| i).unwrap_or(n - 1);
        self.legend(&painter, plot, data, li, pal);
        if let Some(m) = self.measurement {
            draw_measurement(&plot_painter, &f, m, digits, pal);
        }

        // --- back to the market -------------------------------------------------------------------
        if !self.follow {
            let btn = Rect::from_center_size(Pos2::new(plot.right() - 26.0, plot.bottom() - 22.0), Vec2::splat(26.0));
            let hovered = hover.is_some_and(|p| btn.contains(p));
            painter.rect_filled(btn, CornerRadius::same(13), if hovered { pal.accent } else { pal.tag_bg });
            painter.text(btn.center(), Align2::CENTER_CENTER, "»", FontId::proportional(16.0), pal.text);
            if hovered {
                ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
                if response.clicked() {
                    self.right = n as f64 - 1.0 + RIGHT_PAD;
                }
            }
        }

        self.context_menu(&response, data, trade_quote);

        if data.server_now.is_some() {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
        }
    }

    /// Right click: edit the indicator under the pointer (or any, from the list) and, when trading is
    /// allowed, a pending order at that price.
    fn context_menu(&mut self, response: &egui::Response, data: &ChartData, trade_quote: Option<(f64, f64)>) {
        let order = self.menu_price.zip(trade_quote);
        if data.studies.is_empty() && order.is_none() {
            return;
        }
        let digits = data.series.digits as usize;
        response.context_menu(|ui| {
            if let Some(key) = self.menu_study
                && let Some((_, name)) = data.studies.iter().find(|(k, _)| *k == key)
            {
                if ui.button(format!("Editar {name}…")).clicked() {
                    self.actions.push(ChartAction::EditStudy(key));
                    ui.close();
                }
                ui.separator();
            }
            if let Some((price, (bid, ask))) = order {
                let buy = if price < ask { OrderKind::Limit } else { OrderKind::Stop };
                let sell = if price > bid { OrderKind::Limit } else { OrderKind::Stop };
                let name = |k: OrderKind| {
                    if k == OrderKind::Limit { "limite" } else { "stop"
                    }
                };
                for (side, kind, title) in [(Side::Buy, buy, "Compra"), (Side::Sell, sell, "Venda")] {
                    if ui.button(format!("{title} {} @ {price:.digits$}", name(kind))).clicked() {
                        self.actions.push(ChartAction::Order { side, kind, price });
                        ui.close();
                    }
                }
            }
            if !data.studies.is_empty() {
                ui.menu_button("Indicadores", |ui| {
                    for (key, name) in data.studies {
                        if ui.button(format!("{name}…")).clicked() {
                            self.actions.push(ChartAction::EditStudy(key));
                            ui.close();
                        }
                    }
                });
            }
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_pane(&self, painter: &egui::Painter, rect: Rect, f: &Frame, pane: &Pane, i0: usize, i1: usize, pal: &Palette) {
        painter.rect_filled(rect, 0.0, pal.chart_bg);
        painter.hline(rect.x_range(), f.snap(rect.top(), true), Stroke::new(1.0, pal.border));
        let y_of = |v: f32| rect.top() + (7.6 - v) / 7.2 * rect.height();
        let half = 4.0;
        let mut mesh = Mesh::default();
        for strip in &pane.strips {
            let y = y_of(strip.level);
            for i in i0..=i1.min(strip.classes.len().saturating_sub(1)) {
                let c = strip.classes[i];
                if c == u8::MAX || i == 0 || strip.classes[i - 1] == u8::MAX {
                    continue;
                }
                // the segment from the previous bar, colored by this one (MT5 color line)
                let (xa, xb) = (f.x(i as f64 - 1.0), f.x(i as f64));
                if let Some(&color) = strip.palette.get(c as usize) {
                    mesh.add_colored_rect(Rect::from_min_max(Pos2::new(xa, y - half), Pos2::new(xb.max(xa + 1.0), y + half)), color);
                }
            }
        }
        painter.add(Shape::mesh(mesh));
        let font = FontId::monospace(11.5);
        for (k, line) in pane.lines.iter().enumerate() {
            painter.text(Pos2::new(rect.left() + 12.0, rect.top() + 10.0 + 17.0 * k as f32), Align2::LEFT_TOP, line, font.clone(), pal.text_dim);
        }
        if !pane.boxed.is_empty() {
            let font = FontId::monospace(10.5);
            let w = pane.boxed.iter().map(|(t, _)| t.chars().count()).max().unwrap_or(0) as f32 * 6.6 + 14.0;
            let left = (rect.left() + 470.0).min(rect.right() - w - 8.0);
            let bg = Rect::from_min_size(Pos2::new(left, rect.top() + 6.0), Vec2::new(w, 15.0 * pane.boxed.len() as f32 + 8.0));
            painter.rect_filled(bg, CornerRadius::same(3), pal.panel_bg);
            for (k, (line, color)) in pane.boxed.iter().enumerate() {
                painter.text(Pos2::new(bg.left() + 7.0, bg.top() + 4.0 + 15.0 * k as f32), Align2::LEFT_TOP, line, font.clone(), *color);
            }
        }
    }

    /// Auto-scale range: visible bars and the live quote when the newest bar is visible.
    fn auto_range(&self, bars: &[crate::model::Bar], last: Option<f64>, quote: Option<(f64, f64)>, plot_w: f32) -> (f64, f64) {
        let n = bars.len();
        let left = self.right - (plot_w / self.bar_px) as f64;
        let i0 = (left.floor().max(0.0) as usize).min(n.saturating_sub(1));
        let i1 = (self.right.ceil().max(0.0) as usize).min(n.saturating_sub(1));
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        for b in &bars[i0..=i1] {
            lo = lo.min(b.low);
            hi = hi.max(b.high);
        }
        if let Some(p) = last.filter(|_| i1 + 1 >= n) {
            lo = lo.min(p);
            hi = hi.max(p);
        }
        if let Some((bid, ask)) = quote.filter(|_| i1 + 1 >= n) {
            lo = lo.min(bid);
            hi = hi.max(ask);
        }
        if !lo.is_finite() {
            return (self.y_lo, self.y_hi);
        }
        let pad = ((hi - lo) * 0.08).max(hi.abs() * 1e-6).max(1e-9);
        // leave room at the bottom for the volume bars
        (lo - pad - (hi - lo) * 0.18, hi + pad)
    }

    fn legend(&self, painter: &egui::Painter, plot: Rect, data: &ChartData, i: usize, pal: &Palette) {
        let bars = &data.series.bars;
        let b = bars[i];
        let prev_close = if i > 0 { bars[i - 1].close } else { b.open };
        let change = if prev_close != 0.0 { (b.close - prev_close) / prev_close * 100.0 } else { 0.0 };
        let color = if b.close >= b.open { pal.up } else { pal.down };
        let d = data.series.digits as usize;
        let mut pos = Pos2::new(plot.left() + 12.0, plot.top() + 10.0);
        let mut put = |text: String, font: FontId, c: Color32, gap: f32| {
            let r = painter.text(pos, Align2::LEFT_TOP, text, font, c);
            pos.x = r.right() + gap;
        };
        put(format!("{} · {}", data.symbol, data.tf.label()), FontId::proportional(14.0), pal.text, 14.0);
        for (k, v) in [("A", b.open), ("M", b.high), ("m", b.low), ("F", b.close)] {
            put(k.to_string(), FontId::proportional(12.0), pal.text_dim, 3.0);
            put(format!("{v:.d$}"), FontId::proportional(12.0), color, 10.0);
        }
        put(format!("{change:+.2}%"), FontId::proportional(12.0), color, 0.0);
        if let Some((bid, ask)) = valid_quote(data.quote) {
            let text = format!("BID {bid:.d$}   ASK {ask:.d$}   Spread {:.d$}", ask - bid);
            let painter = painter.with_clip_rect(plot);
            let galley = painter.layout_no_wrap(text, FontId::proportional(12.0), pal.text);
            let pos = Pos2::new(plot.left() + 12.0, plot.top() + 30.0);
            painter.rect_filled(Rect::from_min_size(pos, galley.size()).expand(3.0), CornerRadius::same(3), pal.chart_bg);
            painter.galley(pos, galley, pal.text);
        }
        // the indicators' line: plain text or filled tags
        let mut x = plot.left() + 12.0;
        let y = plot.top() + 50.0;
        let painter = painter.with_clip_rect(plot);
        for item in data.legend {
            let font = FontId::proportional(11.5);
            if item.tag {
                let galley = painter.layout_no_wrap(item.text.clone(), font, Color32::WHITE);
                let r = Rect::from_min_size(Pos2::new(x, y), galley.size() + Vec2::new(10.0, 4.0));
                painter.rect_filled(r, CornerRadius::same(3), item.color);
                painter.galley(r.center() - galley.size() / 2.0, galley, Color32::WHITE);
                x = r.right() + 4.0;
            } else {
                let galley = painter.layout_no_wrap(item.text.clone(), font, item.color);
                let r = Rect::from_min_size(Pos2::new(x, y + 2.0), galley.size());
                painter.rect_filled(r.expand(2.0), CornerRadius::same(3), pal.chart_bg);
                painter.galley(r.min, galley, item.color);
                x = r.right() + 8.0;
            }
        }
    }
}

fn draw_measurement(painter: &egui::Painter, f: &Frame, m: Measurement, digits: u32, pal: &Palette) {
    let start = Pos2::new(f.x(m.start.bar as f64), f.y(m.start.price));
    let end = Pos2::new(f.x(m.end.bar as f64), f.y(m.end.price));
    let area = Rect::from_two_pos(start, end);
    painter.rect_filled(area, 0.0, pal.accent.gamma_multiply(0.10));
    let stroke = Stroke::new(1.0 / f.ppp, pal.accent);
    let corner = Pos2::new(end.x, start.y);
    painter.extend(Shape::dashed_line(&[start, corner, end], stroke, 4.0, 4.0));
    painter.line_segment([start, end], Stroke::new(1.5, pal.accent));
    for point in [start, end] {
        painter.circle_filled(point, 3.0, pal.accent);
    }
    let percent = m.percent().map(|p| format!("{p:+.2}%")).unwrap_or_else(|| "—".into());
    let bars = m.bars();
    let text = format!("{percent} · {bars} {}\n{:.d$} até {:.d$}",
        if bars == 1 { "barra" } else { "barras" }, m.start.price, m.end.price, d = digits as usize);
    let galley = painter.layout_no_wrap(text, FontId::proportional(12.0), pal.text);
    let size = galley.size() + Vec2::new(16.0, 12.0);
    let x = (end.x + 12.0).clamp(f.plot.left() + 4.0, (f.plot.right() - size.x - 4.0).max(f.plot.left() + 4.0));
    let y = (end.y - size.y - 12.0).clamp(f.plot.top() + 4.0, (f.plot.bottom() - size.y - 4.0).max(f.plot.top() + 4.0));
    let tag = Rect::from_min_size(Pos2::new(x, y), size);
    painter.rect_filled(tag, CornerRadius::same(4), pal.tag_bg);
    painter.rect_stroke(tag, CornerRadius::same(4), stroke, egui::StrokeKind::Inside);
    painter.galley(tag.min + Vec2::new(8.0, 6.0), galley, pal.text);
}

fn valid_quote(quote: Option<(f64, f64)>) -> Option<(f64, f64)> {
    quote.filter(|&(bid, ask)| bid.is_finite() && ask.is_finite() && bid > 0.0 && ask >= bid)
}

/// Keep the two quote tags apart, even at zero spread; only labels move, never the price lines.
fn quote_tag_tops(axis: Rect, ask_y: f32, bid_y: f32, bid_h: f32, trade_ys: &[f32]) -> [f32; 2] {
    let clamp = |top: f32, h: f32| top.clamp(axis.top(), (axis.bottom() - h).max(axis.top()));
    let ask_top = clamp(ask_y - 24.5, 34.0);
    let bid_top = clamp(bid_y - 24.5, bid_h);
    let tops = if bid_top >= ask_top + 36.0 {
        [ask_top, bid_top]
    } else {
        let ask_top = clamp((ask_y + bid_y) * 0.5 - (36.0 + bid_h) * 0.5, 36.0 + bid_h);
        [ask_top, ask_top + 36.0]
    };
    let overlaps_trade = |top: f32, h: f32| {
        trade_ys.iter().any(|y| top < y + 11.5 && top + h > y - 11.5)
    };
    if !overlaps_trade(tops[0], 34.0) && !overlaps_trade(tops[1], bid_h) {
        return tops;
    }
    // If a trade tag is nearby, find the nearest free space for the pair. Operations retain their
    // exact axis positions and remain in front; the connectors still point at the true bid/ask.
    let h = 36.0 + bid_h;
    let ideal = clamp((ask_y + bid_y) * 0.5 - h * 0.5, h);
    let candidates = [ideal, axis.top(), clamp(axis.bottom() - h, h)].into_iter()
        .chain(trade_ys.iter().flat_map(|y| [clamp(y - 11.5 - h, h), clamp(y + 11.5, h)]));
    let top = candidates.filter(|&top| !overlaps_trade(top, h))
        .min_by(|a, b| (a - ideal).abs().total_cmp(&(b - ideal).abs()));
    top.map(|top| [top, top + 36.0]).unwrap_or(tops)
}

/// Draw in the price layer so operation lines and tags can stay in front of the live quote.
fn draw_prices(painter: &egui::Painter, f: &Frame, price_axis: Rect, data: &ChartData, pal: &Palette) {
    let plot_painter = painter.with_clip_rect(f.plot);
    let axis_painter = painter.with_clip_rect(price_axis);
    let b = data.series.bars.last().unwrap();
    let countdown = data.server_now
        .map(|now| axis::countdown(b.time + data.tf.seconds() - now.floor() as i64))
        .filter(|_| data.tf != Timeframe::D1);
    let d = data.series.digits as usize;
    if let Some((bid, ask)) = valid_quote(data.quote) {
        let bid_h = if countdown.is_some() { 49.0 } else { 34.0 };
        let trade_ys: Vec<f32> = data.levels.iter().map(|l| f.snap(f.y(l.price), true))
            .filter(|y| f.plot.y_range().contains(*y)).collect();
        let tops = quote_tag_tops(price_axis, f.y(ask), f.y(bid), bid_h, &trade_ys);
        for (name, price, color, top, h, cd) in [
            ("ASK", ask, pal.down, tops[0], 34.0, None),
            ("BID", bid, pal.accent, tops[1], bid_h, countdown.as_deref()),
        ] {
            let y = f.snap(f.y(price), true);
            if !f.plot.y_range().contains(y) {
                continue;
            }
            plot_painter.extend(Shape::dashed_line(
                &[Pos2::new(f.plot.left(), y), Pos2::new(f.plot.right(), y)],
                Stroke::new(1.0 / f.ppp, color), 3.0, 3.0,
            ));
            let tag = Rect::from_min_size(Pos2::new(price_axis.left() + 5.0, top), Vec2::new(PRICE_AXIS_W - 6.0, h));
            axis_painter.line_segment([Pos2::new(price_axis.left(), y), Pos2::new(tag.left(), top + 24.5)], Stroke::new(1.0 / f.ppp, color));
            axis_painter.rect_filled(tag, CornerRadius::same(3), color);
            axis_painter.text(Pos2::new(tag.left() + 5.0, top + 8.0), Align2::LEFT_CENTER, name, FontId::proportional(10.0), Color32::WHITE);
            axis_painter.text(Pos2::new(tag.left() + 5.0, top + 24.5), Align2::LEFT_CENTER, format!("{price:.d$}"), FontId::proportional(11.5), Color32::WHITE);
            if let Some(cd) = cd {
                axis_painter.text(Pos2::new(tag.left() + 5.0, top + 40.0), Align2::LEFT_CENTER, cd, FontId::proportional(10.5), Color32::from_white_alpha(210));
            }
        }
    } else if let Some(last) = data.series.last {
        // Historical close until the first live quote arrives; do not invent an ask or spread.
        let color = if last >= b.open { pal.up } else { pal.down };
        let y = f.snap(f.y(last), true);
        if f.plot.y_range().contains(y) {
            plot_painter.extend(Shape::dashed_line(
                &[Pos2::new(f.plot.left(), y), Pos2::new(f.plot.right(), y)],
                Stroke::new(1.0 / f.ppp, color.gamma_multiply(0.8)), 3.0, 3.0,
            ));
        }
        let h = if countdown.is_some() { 34.0 } else { 19.0 };
        let top = (y - 9.5).clamp(price_axis.top(), (price_axis.bottom() - h).max(price_axis.top()));
        let tag = Rect::from_min_size(Pos2::new(price_axis.left() + 1.0, top), Vec2::new(PRICE_AXIS_W - 2.0, h));
        axis_painter.rect_filled(tag, CornerRadius::same(3), color);
        axis_painter.text(Pos2::new(tag.left() + 7.0, top + 9.5), Align2::LEFT_CENTER, format!("{last:.d$}"), FontId::proportional(11.5), Color32::WHITE);
        if let Some(cd) = countdown {
            axis_painter.text(Pos2::new(tag.left() + 7.0, top + 25.0), Align2::LEFT_CENTER, cd, FontId::proportional(10.5), Color32::from_white_alpha(210));
        }
    }
}

/// The indicator drawn closest to `p` (lines, levels, marks), else the pane, the volume band or the
/// tint behind the bar under it.
fn study_at(f: &Frame, data: &ChartData, p: Pos2, pane: Option<Rect>, vol_top: f32, ribbon_top: f32) -> Option<&'static str> {
    if let (Some(rect), Some(pane)) = (pane, data.pane)
        && rect.contains(p)
    {
        return Some(pane.study);
    }
    if !f.plot.contains(p) {
        return None;
    }
    if let Some(r) = &data.ribbon
        && p.y >= ribbon_top
    {
        return Some(r.study);
    }
    let n = data.series.bars.len();
    let at = f.index_at(p.x);
    let i = at.round().clamp(0.0, n.saturating_sub(1) as f64) as usize;
    let mut best: Option<(&'static str, f32)> = None;
    let mut consider = |study: &'static str, d: f32, within: f32| {
        if d <= within && best.is_none_or(|(_, b)| d < b) {
            best = Some((study, d));
        }
    };
    for o in data.overlays {
        // the line between the two bars around the pointer, or the nearest point at a gap
        let a = at.floor().max(0.0) as usize;
        let value = |k: usize| o.values.get(k).copied().filter(|v| v.is_finite());
        let y = match (value(a), value(a + 1)) {
            (Some(va), Some(vb)) => Some(f.y(va + (vb - va) * (at - a as f64).clamp(0.0, 1.0))),
            _ => value(i).map(|v| f.y(v)),
        };
        if let Some(y) = y {
            consider(o.study, (y - p.y).abs(), o.width.max(1.0) + 5.0);
        }
    }
    for m in data.map_levels {
        let start = data.series.bars.partition_point(|b| b.time < m.from);
        if p.x >= f.x(start as f64) {
            consider(m.study, (f.y(m.price) - p.y).abs(), m.width + 4.0);
        }
    }
    if let Some(m) = &data.marks
        && let Some(&price) = m.prices.get(i).filter(|v| v.is_finite())
    {
        consider(m.study, (f.y(price) - p.y).abs(), 4.0);
    }
    if let Some((study, _)) = best {
        return Some(study);
    }
    if let Some(v) = &data.volume
        && p.y >= vol_top
    {
        return Some(v.study);
    }
    data.shading.as_ref().filter(|s| s.classes.get(i).is_some_and(|c| (*c as usize) < s.palette.len())).map(|s| s.study)
}

/// Polyline runs of one color between valid points, from bar `i0` to `i1`.
fn draw_overlay(painter: &egui::Painter, f: &Frame, o: &Overlay, i0: usize, i1: usize) {
    if o.values.is_empty() || i1 <= i0 {
        return;
    }
    let color_of = |i: usize| {
        let k = o.shade.and_then(|s| s.get(i)).copied().unwrap_or(0) as usize;
        o.palette.get(k).or(o.palette.first()).copied().unwrap_or(Color32::WHITE)
    };
    let flush = |run: &mut Vec<Pos2>, color: Color32| {
        if run.len() >= 2 {
            let stroke = Stroke::new(o.width, color);
            if o.dotted {
                painter.extend(Shape::dashed_line(run, stroke, 2.0, 2.0));
            } else {
                painter.add(Shape::line(std::mem::take(run), stroke));
            }
        }
        run.clear();
    };
    let mut run: Vec<Pos2> = Vec::new();
    let mut run_color = Color32::TRANSPARENT;
    for i in i0..=i1 {
        let v = o.values[i];
        if !v.is_finite() {
            flush(&mut run, run_color);
            continue;
        }
        let p = Pos2::new(f.x(i as f64), f.y(v));
        let c = color_of(i);
        if run.is_empty() {
            run.push(p);
            run_color = c;
            continue;
        }
        if c != run_color {
            // the segment into bar i takes bar i's color: start a new run at the previous point
            let prev = *run.last().unwrap();
            flush(&mut run, run_color);
            run.push(prev);
            run_color = c;
        }
        run.push(p);
    }
    flush(&mut run, run_color);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Bar;

    #[test]
    fn measurement_counts_intervals_and_signed_percentage_from_start() {
        let mut m = Measurement { start: MeasurePoint { bar: 20, price: 100.0 }, end: MeasurePoint { bar: 30, price: 110.0 } };
        assert_eq!(m.bars(), 10);
        assert_eq!(m.percent(), Some(10.0));
        m.end = MeasurePoint { bar: 5, price: 90.0 };
        assert_eq!(m.bars(), 15);
        assert_eq!(m.percent(), Some(-10.0));
        m.end = m.start;
        assert_eq!(m.bars(), 0);
        assert_eq!(m.percent(), Some(0.0));
        m.start.price = 0.0;
        assert_eq!(m.percent(), None);
    }

    fn chart_frame(ctx: &egui::Context, view: &mut ChartView, data: &ChartData, events: Vec<egui::Event>) -> egui::FullOutput {
        let mut output = ctx.run_ui(egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0))),
            events, ..Default::default()
        }, |ui| {
            egui::CentralPanel::no_frame().show(ui, |ui| view.ui(ui, data, &Palette::default()));
        });
        output.textures_delta.clear();
        output
    }

    fn pointer_button(pos: Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() }
    }

    #[test]
    fn date_jump_centers_after_history_prepend_and_today_resumes_following() {
        let mut series = Series::new((100..500).map(|i| Bar {
            time: i * 300, open: 100.0, high: 101.0, low: 99.0, close: 100.0, volume: 1.0,
        }).collect(), 2);
        fn data(series: &Series) -> ChartData<'_> {
            ChartData {
                series, symbol: "X", tf: Timeframe::M5, server_now: None,
                levels: &[], overlays: &[], map_levels: &[], pane: None, pane_open: false,
                layers: &Layer::DEFAULT, quote: None, can_trade: false, cursor: CursorMode::Hand,
                tick: 0.0, order_volume: "1", bracket: None, volume: None, marks: None, shading: None, ribbon: None, legend: &[], studies: &[],
            }
        }
        let ctx = egui::Context::default();
        let mut view = ChartView { bar_px: 10.0, ..Default::default() };
        chart_frame(&ctx, &mut view, &data(&series), vec![]);
        // Navigation resolves in the frame in which older history arrives.
        series.merge(&(0..100).map(|i| Bar { time: i * 300, ..series.bars[0] }).collect::<Vec<_>>());
        view.auto_y = false;
        view.go_to(150 * 300);
        chart_frame(&ctx, &mut view, &data(&series), vec![]);
        let plot_width = 800.0 - PRICE_AXIS_W;
        assert_eq!(view.right, 150.0 + (plot_width / view.bar_px) as f64 / 2.0);
        assert!(view.auto_y && !view.follow);
        assert_eq!(view.bar_px, 10.0);
        let right = view.right;
        series.apply_tick(Timeframe::M5, 500 * 300, 100.0, 1.0);
        chart_frame(&ctx, &mut view, &data(&series), vec![]);
        assert_eq!(view.right, right, "historical date stays put on live bars");
        view.go_to(10 * 300);
        view.reset(); // Hoje cancels any queued jump, even before a chart frame.
        chart_frame(&ctx, &mut view, &data(&series), vec![]);
        assert_eq!(view.right, 500.0 + RIGHT_PAD);
        assert!(view.follow && view.auto_y);
        assert_eq!(view.bar_px, 10.0);
        series.apply_tick(Timeframe::M5, 501 * 300, 100.0, 1.0);
        chart_frame(&ctx, &mut view, &data(&series), vec![]);
        assert_eq!(view.right, 501.0 + RIGHT_PAD);
    }

    #[test]
    fn only_hand_pans_cross_measures_and_arrow_stays_still() {
        let series = Series::new((0..120).map(|i| Bar {
            time: i * 300, open: 100.0, high: 101.0, low: 99.0, close: 100.0, volume: 1.0,
        }).collect(), 2);
        for cursor in CursorMode::ALL {
            let data = ChartData {
                series: &series, symbol: "X", tf: Timeframe::M5, server_now: None,
                levels: &[], overlays: &[], map_levels: &[], pane: None, pane_open: false,
                layers: &Layer::DEFAULT, quote: None, can_trade: false, cursor, tick: 0.0,
                order_volume: "1", bracket: None, volume: None, marks: None, shading: None, ribbon: None, legend: &[], studies: &[],
            };
            let ctx = egui::Context::default();
            let mut view = ChartView::default();
            chart_frame(&ctx, &mut view, &data, vec![]);
            let right = view.right;
            let range = (view.y_lo, view.y_hi);
            let start = Pos2::new(300.0, 400.0);
            let end = Pos2::new(380.0, 200.0);
            chart_frame(&ctx, &mut view, &data, vec![egui::Event::PointerMoved(start), pointer_button(start, true)]);
            chart_frame(&ctx, &mut view, &data, vec![egui::Event::PointerMoved(end)]);
            let output = chart_frame(&ctx, &mut view, &data, vec![pointer_button(end, false)]);
            assert!(view.actions.is_empty());
            match cursor {
                CursorMode::Hand => {
                    assert_eq!(view.right, right - 10.0);
                    assert_ne!((view.y_lo, view.y_hi), range);
                    assert!(view.measurement.is_none());
                }
                CursorMode::Arrow => {
                    assert_eq!(view.right, right);
                    assert_eq!((view.y_lo, view.y_hi), range);
                    assert!(view.measurement.is_none());
                }
                CursorMode::Cross => {
                    assert_eq!(view.right, right);
                    assert_eq!((view.y_lo, view.y_hi), range);
                    let m = view.measurement.unwrap();
                    assert_eq!(m.bars(), 10);
                    assert!(m.percent().unwrap() > 0.0);
                    assert!(!view.measuring);
                    assert!(output.shapes.iter().any(|s| match &s.shape {
                        Shape::Text(t) => t.galley.job.text.contains("% · 10 barras"),
                        _ => false,
                    }));
                    chart_frame(&ctx, &mut view, &data, vec![egui::Event::Key {
                        key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default(),
                    }]);
                    assert!(view.measurement.is_none());
                }
            }
        }
    }

    #[test]
    fn changing_cursor_and_reset_clear_measurement() {
        let m = Measurement { start: MeasurePoint { bar: 0, price: 100.0 }, end: MeasurePoint { bar: 5, price: 101.0 } };
        let mut view = ChartView { cursor: CursorMode::Cross, measurement: Some(m), measuring: true, ..ChartView::default() };
        view.set_cursor(CursorMode::Arrow);
        assert!(view.measurement.is_none() && !view.measuring);
        view.set_cursor(CursorMode::Cross);
        view.measurement = Some(m);
        view.reset();
        assert_eq!(view.cursor, CursorMode::Cross);
        assert!(view.measurement.is_none());
    }

    #[test]
    fn quote_tags_remain_separate_at_zero_spread_and_axis_edges() {
        let axis = Rect::from_min_max(Pos2::new(600.0, 20.0), Pos2::new(676.0, 620.0));
        for bid_h in [34.0, 49.0] {
            for ask_y in [20.0_f32, 21.0, 300.0, 600.0, 620.0] {
                for gap in [0.0, 0.1, 2.0, 40.0, 100.0] {
                    let bid_y = (ask_y + gap).min(axis.bottom());
                    let [ask_top, bid_top] = quote_tag_tops(axis, ask_y, bid_y, bid_h, &[]);
                    assert!(ask_top >= axis.top());
                    assert!(ask_top + 34.0 <= bid_top);
                    assert!(bid_top + bid_h <= axis.bottom());
                }
            }
        }
    }

    #[test]
    fn quote_tags_avoid_nearby_operation_prices() {
        let axis = Rect::from_min_max(Pos2::new(600.0, 20.0), Pos2::new(676.0, 620.0));
        for trade_ys in [&[300.0][..], &[299.0, 320.0][..], &[20.0, 45.0][..], &[600.0, 620.0][..]] {
            let [ask_top, bid_top] = quote_tag_tops(axis, trade_ys[0], trade_ys[0] + 1.0, 49.0, trade_ys);
            assert!(ask_top >= axis.top() && bid_top + 49.0 <= axis.bottom());
            for y in trade_ys {
                assert!(ask_top + 34.0 <= y - 11.5 || ask_top >= y + 11.5);
                assert!(bid_top + 49.0 <= y - 11.5 || bid_top >= y + 11.5);
            }
        }
    }

    #[test]
    fn live_ask_is_in_auto_range_only_when_latest_bar_is_visible() {
        let bars = vec![Bar { time: 0, open: 100.0, high: 101.0, low: 99.0, close: 100.0, volume: 1.0 }; 100];
        let mut view = ChartView { right: 105.0, ..ChartView::default() };
        let (lo, hi) = view.auto_range(&bars, Some(100.0), Some((100.0, 110.0)), 400.0);
        assert!(lo < 100.0 && hi > 110.0);
        view.right = 50.0;
        let (_, hi) = view.auto_range(&bars, Some(100.0), Some((100.0, 110.0)), 400.0);
        assert!(hi < 110.0);
    }

    #[test]
    fn locked_chart_shows_quote_without_order_preview_and_trades_draw_on_top() {
        let series = Series::new(vec![Bar { time: 0, open: 100.0, high: 101.0, low: 99.0, close: 100.0, volume: 1.0 }], 2);
        let levels = [Level { price: 100.0, color: Color32::RED, label: "operation".into(), dashed: false, handle: None, side: None }];
        let data = ChartData {
            series: &series, symbol: "X", tf: Timeframe::M5, server_now: Some(1.0),
            levels: &levels, overlays: &[], map_levels: &[], pane: None, pane_open: false,
            layers: &[Layer::Levels, Layer::Indicators, Layer::Price, Layer::Trades],
            quote: Some((100.0, 100.25)), can_trade: false, cursor: CursorMode::Hand, tick: 0.25, order_volume: "1",
            bracket: None, volume: None, marks: None, shading: None, ribbon: None, legend: &[], studies: &[],
        };
        let ctx = egui::Context::default();
        let mut view = ChartView::default();
        let mut output = ctx.run_ui(egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0))),
            events: vec![
                egui::Event::ModifiersChanged(egui::Modifiers { shift: true, ..Default::default() }),
                egui::Event::PointerMoved(Pos2::new(300.0, 300.0)),
            ],
            ..Default::default()
        }, |ui| {
            egui::CentralPanel::no_frame().show(ui, |ui| view.ui(ui, &data, &Palette::default()));
        });
        output.textures_delta.clear(); // No GPU backend in this render test.
        let texts: Vec<&str> = output.shapes.iter().filter_map(|s| match &s.shape {
            Shape::Text(t) => Some(t.galley.job.text.as_str()),
            _ => None,
        }).collect();
        assert!(texts.contains(&"ASK") && texts.contains(&"BID"));
        assert!(texts.contains(&"100.25"));
        assert!(texts.contains(&"BID 100.00   ASK 100.25   Spread 0.25"));
        assert!(!texts.iter().any(|t| t.contains("clique para posicionar")));
        let bid_index = texts.iter().position(|t| *t == "BID").unwrap();
        let operation_index = texts.iter().position(|t| *t == "operation").unwrap();
        assert!(bid_index < operation_index, "operations must paint after price tags");
        assert!(view.actions.is_empty());
    }

    #[test]
    fn right_click_finds_the_indicator_under_the_pointer() {
        let series = Series::new((0..100).map(|i| Bar { time: i * 300, open: 100.0, high: 101.0, low: 99.0, close: 100.0, volume: 1.0 }).collect(), 2);
        let line: Vec<f64> = (0..100).map(|i| if i < 10 { f64::NAN } else { 100.0 }).collect();
        let overlays = [Overlay { study: "line", values: &line, shade: None, palette: &[Color32::WHITE], width: 2.0, dotted: false }];
        let map_levels = [MapLevel { study: "map", price: 105.0, from: 50 * 300, color: Color32::RED, width: 1.0, dashed: false, text: String::new(), details: String::new() }];
        let classes: Vec<u8> = (0..100).map(|i| if i >= 80 { 0 } else { u8::MAX }).collect();
        let strip = || Strip { classes: &[], palette: &[], level: 1.0 };
        let pane = Pane { study: "pane", strips: [strip(), strip(), strip()], lines: &[], boxed: &[] };
        let data = ChartData {
            series: &series, symbol: "X", tf: Timeframe::M5, server_now: None,
            levels: &[], overlays: &overlays, map_levels: &map_levels, pane: Some(&pane), pane_open: true,
            layers: &Layer::DEFAULT, quote: None, can_trade: false, cursor: CursorMode::Hand, tick: 0.0, order_volume: "1",
            bracket: None, volume: None, marks: None, legend: &[], studies: &[], ribbon: None,
            shading: Some(Shading { study: "tint", classes: &classes, palette: &[Color32::GREEN] }),
        };
        let plot = Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 400.0));
        let f = Frame { plot, right: 99.0, bar_px: 10.0, lo: 90.0, hi: 110.0, ppp: 1.0 };
        let pane_rect = Rect::from_min_size(Pos2::new(0.0, 400.0), Vec2::new(1000.0, 100.0));
        let at = |bar: f64, price: f64| Pos2::new(f.x(bar), f.y(price));
        let hit = |p: Pos2| study_at(&f, &data, p, Some(pane_rect), 380.0, 400.0);
        assert_eq!(hit(at(40.0, 100.0) + Vec2::new(0.0, 4.0)), Some("line"));
        assert_eq!(hit(at(5.0, 100.0)), None, "no line where it has no values");
        assert_eq!(hit(at(60.0, 105.0)), Some("map"));
        assert_eq!(hit(at(40.0, 105.0)), None, "a level starts at its bar");
        assert_eq!(hit(at(90.0, 95.0)), Some("tint"), "the tint behind the bar, away from the lines");
        assert_eq!(hit(Pos2::new(500.0, 450.0)), Some("pane"));
    }

    #[test]
    fn right_click_on_a_line_offers_to_edit_its_indicator() {
        let series = Series::new((0..100).map(|i| Bar { time: i * 300, open: 100.0, high: 101.0, low: 99.0, close: 100.0, volume: 1.0 }).collect(), 2);
        let line = vec![100.5; 100];
        let overlays = [Overlay { study: "line", values: &line, shade: None, palette: &[Color32::WHITE], width: 2.0, dotted: false }];
        let studies = [("line", "Linha X"), ("other", "Outro")];
        let data = ChartData {
            series: &series, symbol: "X", tf: Timeframe::M5, server_now: None,
            levels: &[], overlays: &overlays, map_levels: &[], pane: None, pane_open: false,
            layers: &Layer::DEFAULT, quote: None, can_trade: false, cursor: CursorMode::Hand, tick: 0.0, order_volume: "1",
            bracket: None, volume: None, marks: None, shading: None, ribbon: None, legend: &[], studies: &studies,
        };
        let ctx = egui::Context::default();
        let mut view = ChartView { bar_px: 6.0, ..Default::default() };
        chart_frame(&ctx, &mut view, &data, vec![]);
        // where the line is drawn at bar 80
        let plot = Rect::from_min_max(Pos2::ZERO, Pos2::new(800.0 - PRICE_AXIS_W, 600.0 - TIME_AXIS_H));
        let f = Frame { plot, right: view.right, bar_px: view.bar_px, lo: view.y_lo, hi: view.y_hi, ppp: 1.0 };
        let at = Pos2::new(f.x(80.0), f.y(100.5));
        let secondary = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Default::default() };
        chart_frame(&ctx, &mut view, &data, vec![egui::Event::PointerMoved(at), secondary(true)]);
        chart_frame(&ctx, &mut view, &data, vec![secondary(false)]);
        assert_eq!(view.menu_study, Some("line"));
        let output = chart_frame(&ctx, &mut view, &data, vec![]);
        let texts: Vec<(String, Rect)> = output.shapes.iter().filter_map(|s| match &s.shape {
            Shape::Text(t) => Some((t.galley.job.text.clone(), t.visual_bounding_rect())),
            _ => None,
        }).collect();
        let edit = texts.iter().find(|(t, _)| t == "Editar Linha X…").map(|(_, r)| r.center()).expect("menu item to edit the line");
        assert!(texts.iter().any(|(t, _)| t == "Indicadores"), "every indicator is listed too");
        assert!(!texts.iter().any(|(t, _)| t.starts_with("Compra")), "no orders while trading is locked");
        chart_frame(&ctx, &mut view, &data, vec![egui::Event::PointerMoved(edit), pointer_button(edit, true)]);
        chart_frame(&ctx, &mut view, &data, vec![pointer_button(edit, false)]);
        assert_eq!(view.actions, vec![ChartAction::EditStudy("line")]);
    }
}
