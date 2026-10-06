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
/// Height of the indicator pane under the price plot.
const PANE_H: f32 = 170.0;
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

pub struct ChartView {
    right: f64,
    bar_px: f32,
    auto_y: bool,
    y_lo: f64,
    y_hi: f64,
    drag: Option<Zone>,
    drag_dy: f32,
    follow: bool,
    known_len: usize,
    /// Time of the oldest bar seen, to keep the view still when older history is loaded in front.
    known_first: i64,
    /// The view reaches (or is within a screen of) the oldest loaded bar: more history is wanted.
    pub wants_older: bool,
    /// Trade line being dragged and where it is now.
    level_drag: Option<(Handle, f64)>,
    /// Price under the pointer when the context menu opened.
    menu_price: Option<f64>,
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
    /// New pending order from the context menu.
    Order { side: Side, kind: OrderKind, price: f64 },
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
            follow: true,
            known_len: 0,
            known_first: 0,
            wants_older: false,
            level_drag: None,
            menu_price: None,
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
    /// Drawing order of the plot's layers, back to front.
    pub layers: &'a [Layer],
    /// Bid/ask of the symbol, to tell limit from stop in the context menu (None: no menu).
    pub quote: Option<(f64, f64)>,
    /// Price step of the symbol (the order following the pointer shows the price it will get).
    pub tick: f64,
    /// Volume of the ticket, shown on the order following the pointer.
    pub order_volume: &'a str,
    /// Stop and target of the next order, drawn with the order following the pointer.
    pub bracket: Option<crate::trading::Bracket>,
}

/// Lines with a × to remove them: every trade line (a position's × closes it).
fn removable(_h: Handle) -> bool {
    true
}

/// What the price plot draws, in an order the user picks (the last one is in front).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Layer {
    /// Candles and the last price line.
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
    pub values: &'a [f64],
    pub shade: Option<&'a [u8]>,
    pub palette: &'a [Color32],
    pub width: f32,
    pub dotted: bool,
}

pub struct MapLevel {
    pub price: f64,
    /// Bar time where the line starts.
    pub from: i64,
    pub color: Color32,
    pub width: f32,
    pub dashed: bool,
    pub text: String,
}

/// One strip of the pane: a class per bar (`u8::MAX` = nothing) and the color of each class.
pub struct Strip<'a> {
    pub classes: &'a [u8],
    pub palette: &'a [Color32],
    /// Height on the pane's 0.4–7.6 scale (like an MT5 indicator subwindow).
    pub level: f32,
}

pub struct Pane<'a> {
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
        *self = Self { bar_px: self.bar_px, ..Self::default() };
    }

    pub fn ui(&mut self, ui: &mut Ui, data: &ChartData, pal: &Palette) {
        let rect = ui.available_rect_before_wrap();
        let response = ui.allocate_rect(rect, Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        let ppp = ui.ctx().pixels_per_point();
        painter.rect_filled(rect, 0.0, pal.chart_bg);

        let pane_h = if data.pane.is_some() { PANE_H.min(rect.height() * 0.4) } else { 0.0 };
        let plot = Rect::from_min_max(rect.min, Pos2::new(rect.right() - PRICE_AXIS_W, rect.bottom() - TIME_AXIS_H - pane_h));
        let pane_rect = Rect::from_min_max(Pos2::new(rect.left(), plot.bottom()), Pos2::new(plot.right(), plot.bottom() + pane_h));
        let price_axis = Rect::from_min_max(Pos2::new(plot.right(), rect.top()), Pos2::new(rect.right(), plot.bottom()));
        let time_axis = Rect::from_min_max(Pos2::new(rect.left(), rect.bottom() - TIME_AXIS_H), rect.max);

        let bars = &data.series.bars;
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
        }
        self.known_first = bars[0].time;

        // --- keep following the market when new bars arrive -------------------------------------
        if self.known_len == 0 {
            self.right = n as f64 - 1.0 + RIGHT_PAD;
        } else if n > self.known_len && self.follow {
            self.right += (n - self.known_len) as f64;
        }
        self.known_len = n;

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
        // trade lines drag only with Alt held; without it a drag always pans the chart
        let alt = ui.input(|i| i.modifiers.alt);
        let alt_grab = |p: Pos2| if alt { grab_within(p, ALT_GRAB_PX) } else { None };
        // the × of a removable line under the pointer
        let remove_at = |p: Pos2| -> Option<Handle> {
            if p.x < plot.left() + REMOVE_X || p.x > plot.left() + REMOVE_X + REMOVE_W {
                return None;
            }
            data.levels
                .iter()
                .filter_map(|l| l.handle.filter(|h| removable(*h)).map(|h| (h, (y_at(l.price) - p.y).abs())))
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
        let snap_price = |price: f64| if data.tick > 0.0 { (price / data.tick).round() * data.tick } else { price };
        let ghost = match (ghost_side, hover.filter(|p| plot.contains(*p)), data.quote) {
            (Some(side), Some(p), Some((bid, ask))) if self.level_drag.is_none() && self.drag.is_none() => {
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
                _ if remove_at(p).is_some() => CursorIcon::PointingHand,
                _ if ghost.is_some() => CursorIcon::Crosshair,
                Zone::Plot if self.drag.is_none() && alt_grab(p).is_some() => CursorIcon::ResizeVertical,
                Zone::PriceAxis => CursorIcon::ResizeVertical,
                Zone::TimeAxis => CursorIcon::ResizeHorizontal,
                Zone::Plot if self.drag.is_some() => CursorIcon::Grabbing,
                Zone::Plot => CursorIcon::Crosshair,
            });
        }

        // y range before this frame's input, needed to unlock auto-scale smoothly
        let (auto_lo, auto_hi) = self.auto_range(bars, data.series.last, plot.width());
        if self.auto_y {
            (self.y_lo, self.y_hi) = (auto_lo, auto_hi);
        }

        if response.drag_started() {
            // where the button went down: egui reports a drag only after the pointer has moved a few
            // pixels, by then off a thin line (a fast hand would pan instead of grabbing it)
            let at = ui.input(|i| i.pointer.press_origin()).or_else(|| response.interact_pointer_pos());
            match at.and_then(|p| alt_grab(p).map(|h| (h, price_at(p.y)))) {
                Some(grabbed) => self.level_drag = Some(grabbed),
                None => self.drag = at.map(zone_at),
            }
            self.drag_dy = 0.0;
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
        if response.clicked() {
            match response.interact_pointer_pos().and_then(remove_at) {
                Some(handle) => self.actions.push(ChartAction::Remove { handle }),
                None => {
                    if let Some((side, kind, price)) = ghost {
                        self.actions.push(ChartAction::Order { side, kind, price });
                    }
                }
            }
        }
        // right click: pending order at that price
        if response.secondary_clicked() {
            self.menu_price = response.interact_pointer_pos().filter(|p| plot.contains(*p)).map(|p| price_at(p.y));
        }
        if let (Some(price), Some((bid, ask))) = (self.menu_price, data.quote) {
            let digits = data.series.digits as usize;
            response.context_menu(|ui| {
                let buy = if price < ask { OrderKind::Limit } else { OrderKind::Stop };
                let sell = if price > bid { OrderKind::Limit } else { OrderKind::Stop };
                let name = |k: OrderKind| if k == OrderKind::Limit { "limite" } else { "stop" };
                for (side, kind, title) in [(Side::Buy, buy, "Compra"), (Side::Sell, sell, "Venda")] {
                    if ui.button(format!("{title} {} @ {price:.digits$}", name(kind))).clicked() {
                        self.actions.push(ChartAction::Order { side, kind, price });
                        ui.close();
                    }
                }
            });
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
        if response.hovered() || ui.ctx().memory(|m| m.focused().is_none()) {
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
        if self.auto_y {
            let (lo, hi) = self.auto_range(bars, data.series.last, plot.width());
            (self.y_lo, self.y_hi) = (lo, hi);
        }
        if !(self.y_hi - self.y_lo).is_normal() || self.y_hi <= self.y_lo {
            self.y_hi = self.y_lo + 1.0;
        }

        let f = Frame { plot, right: self.right, bar_px: self.bar_px, lo: self.y_lo, hi: self.y_hi, ppp };
        let i0 = (f.index_at(plot.left()).floor() as isize).max(0) as usize;
        let i1 = (f.right.ceil() as isize).clamp(0, n as isize - 1) as usize;
        let digits = data.series.digits;

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
        let max_vol = bars[i0..=i1].iter().map(|b| b.volume).fold(0.0, f64::max);
        let vol_h = plot.height() * 0.16;
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
                        Pos2::new(f.snap(cx - w / 2.0, false), plot.bottom() - h),
                        Pos2::new(f.snap(cx + w / 2.0, false).max(f.snap(cx - w / 2.0, false) + 1.0 / ppp), plot.bottom()),
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
        // volume is background context: always behind every layer
        plot_painter.add(Shape::mesh(vol_mesh));

        // --- layers, back to front -----------------------------------------------------------------
        let mut candles = Some(mesh);
        let last_x = f.x((n - 1) as f64);
        for layer in data.layers {
            match layer {
                Layer::Price => {
                    if let Some(mesh) = candles.take() {
                        plot_painter.add(Shape::mesh(mesh));
                    }
                    if let Some(last) = data.series.last {
                        let color = if last >= bars[n - 1].open { pal.up } else { pal.down };
                        let y = f.snap(f.y(last), true);
                        if plot.y_range().contains(y) {
                            plot_painter.extend(Shape::dashed_line(
                                &[Pos2::new(plot.left(), y), Pos2::new(plot.right(), y)],
                                Stroke::new(1.0 / ppp, color.gamma_multiply(0.8)),
                                3.0,
                                3.0,
                            ));
                        }
                    }
                }
                Layer::Indicators => {
                    for o in data.overlays {
                        draw_overlay(&plot_painter, &f, o, i0.saturating_sub(1), i1.min(o.values.len().saturating_sub(1)));
                    }
                }
                Layer::Levels => {
                    for m in data.map_levels {
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
                        let at = Pos2::new(last_x.min(plot.right() - 4.0), y - 2.0);
                        plot_painter.text(at, Align2::RIGHT_BOTTOM, &m.text, FontId::proportional(11.0), m.color);
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
            self.draw_pane(&painter.with_clip_rect(pane_rect), pane_rect, &f, pane, i0, i1, pal);
        }

        // --- last price tag on the axis (on top of the other axis tags) ----------------------------
        if let Some(last) = data.series.last {
            let b = bars[n - 1];
            let color = if last >= b.open { pal.up } else { pal.down };
            let y = f.snap(f.y(last), true);
            let countdown = data
                .server_now
                .map(|now| axis::countdown(b.time + data.tf.seconds() - now.floor() as i64))
                .filter(|_| data.tf != Timeframe::D1);
            let h = if countdown.is_some() { 34.0 } else { 19.0 };
            let top = (y - 9.5).clamp(price_axis.top(), price_axis.bottom() - h);
            let tag = Rect::from_min_size(Pos2::new(price_axis.left() + 1.0, top), Vec2::new(PRICE_AXIS_W - 2.0, h));
            painter.rect_filled(tag, CornerRadius::same(3), color);
            painter.text(
                Pos2::new(tag.left() + 7.0, top + 9.5),
                Align2::LEFT_CENTER,
                format!("{last:.prec$}", prec = digits as usize),
                FontId::proportional(11.5),
                Color32::WHITE,
            );
            if let Some(cd) = countdown {
                painter.text(
                    Pos2::new(tag.left() + 7.0, top + 25.0),
                    Align2::LEFT_CENTER,
                    cd,
                    FontId::proportional(10.5),
                    Color32::from_white_alpha(210),
                );
            }
        }

        // --- crosshair + legend -------------------------------------------------------------------
        let hovered_bar = hover.filter(|p| plot.contains(*p)).map(|p| {
            let i = f.index_at(p.x).round().clamp(0.0, (n - 1) as f64) as usize;
            (p, i)
        });
        if let Some((p, i)) = hovered_bar {
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

        if data.server_now.is_some() {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
        }
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

    /// Auto-scale range: highs/lows of the visible bars plus the last price, with padding.
    fn auto_range(&self, bars: &[crate::model::Bar], last: Option<f64>, plot_w: f32) -> (f64, f64) {
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
    }
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
