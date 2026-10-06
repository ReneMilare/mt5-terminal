//! Order ticket, positions and pending orders. Knows nothing about where orders go: it turns clicks into
//! [`Command`]s and folds the source's trade messages back into its state.

use crate::chart::{ChartAction, Handle, Level};
use crate::feed::{Command, Message, OrderKind, PendingOrder, Position, Side};
use crate::theme::Palette;
use eframe::egui::{self, Align, Color32, Layout, RichText};
use std::collections::{HashMap, VecDeque};

const LOG_LINES: usize = 6;

/// Trading rules of the chart symbol.
pub struct Spec {
    pub digits: u32,
    pub tick_size: f64,
    pub vol_min: f64,
    pub vol_max: f64,
    pub vol_step: f64,
}

pub struct Funds {
    pub balance: f64,
    pub equity: f64,
    pub margin_free: f64,
    pub currency: String,
    pub trade_allowed: bool,
}

pub struct Trading {
    pub positions: Vec<Position>,
    pub orders: Vec<PendingOrder>,
    pub funds: Option<Funds>,
    pub spec: Option<Spec>,
    /// Bid/ask of the chart symbol.
    pub quote: Option<(f64, f64)>,
    volume: f64,
    kind: OrderKind,
    /// Entry price for limit/stop orders.
    price: f64,
    /// Stop loss / take profit distance in price units from the entry; 0 = none.
    sl_dist: f64,
    tp_dist: f64,
    /// Trading on a real account is locked until armed in this session.
    armed: bool,
    /// Netting account (one position per symbol): reversing is a single order.
    pub netting: bool,
    next_id: u64,
    requests: HashMap<u64, String>,
    log: VecDeque<(bool, String)>,
}

impl Default for Trading {
    fn default() -> Self {
        Self {
            positions: Vec::new(),
            orders: Vec::new(),
            funds: None,
            spec: None,
            quote: None,
            volume: 0.0,
            kind: OrderKind::Market,
            price: 0.0,
            sl_dist: 0.0,
            tp_dist: 0.0,
            armed: false,
            next_id: 1,
            requests: HashMap::new(),
            netting: false,
            log: VecDeque::new(),
        }
    }
}

fn side_label(side: Side) -> &'static str {
    match side {
        Side::Buy => "Compra",
        Side::Sell => "Venda",
    }
}

fn kind_label(kind: OrderKind) -> &'static str {
    match kind {
        OrderKind::Market => "Mercado",
        OrderKind::Limit => "Limite",
        OrderKind::Stop => "Stop",
    }
}

/// Decimals needed to show multiples of `step`.
fn step_decimals(step: f64) -> usize {
    if step <= 0.0 {
        return 2;
    }
    (0..8).find(|&d| ((step * 10f64.powi(d)).round() - step * 10f64.powi(d)).abs() < 1e-9).unwrap_or(8) as usize
}

fn round_to(v: f64, step: f64) -> f64 {
    if step > 0.0 { (v / step).round() * step } else { v }
}

impl Trading {
    /// Forget everything account related (source switched or disconnected).
    pub fn reset(&mut self) {
        self.positions.clear();
        self.orders.clear();
        self.funds = None;
        self.spec = None;
        self.quote = None;
        self.requests.clear();
        self.armed = false;
    }

    /// New chart symbol: its rules and quote come again from the source.
    pub fn symbol_changed(&mut self) {
        self.spec = None;
        self.quote = None;
        self.price = 0.0;
    }

    /// Fold a source message in. Returns false when it isn't a trading message.
    pub fn on_message(&mut self, msg: &Message, chart_symbol: &str) -> bool {
        match msg {
            Message::Symbol { symbol, digits, tick_size, vol_min, vol_max, vol_step } if symbol == chart_symbol => {
                let spec = Spec { digits: *digits, tick_size: *tick_size, vol_min: *vol_min, vol_max: *vol_max, vol_step: *vol_step };
                self.volume = if self.volume <= 0.0 { spec.vol_min } else { round_to(self.volume, spec.vol_step).clamp(spec.vol_min, spec.vol_max) };
                self.spec = Some(spec);
            }
            Message::Symbol { .. } => {}
            Message::Account { balance, equity, margin_free, currency, trade_allowed } => {
                self.funds = Some(Funds {
                    balance: *balance,
                    equity: *equity,
                    margin_free: *margin_free,
                    currency: currency.clone(),
                    trade_allowed: *trade_allowed,
                });
            }
            Message::Positions { positions } => self.positions = positions.clone(),
            Message::Orders { orders } => self.orders = orders.clone(),
            Message::TradeResult { id, ok, retcode, msg, price, .. } => {
                let what = self.requests.get(id).cloned().unwrap_or_else(|| format!("#{id}"));
                let mut text = format!("{what}: {msg}");
                if *ok && *price > 0.0 {
                    text += &format!(" @ {}", self.fmt_price(*price));
                }
                if !ok && *retcode != 0 {
                    text += &format!(" ({retcode})");
                }
                self.log.push_front((*ok, text));
                self.log.truncate(LOG_LINES);
            }
            Message::Tick { symbol, bid, ask, .. } if symbol == chart_symbol => self.quote = Some((*bid, *ask)),
            _ => return false,
        }
        true
    }

    fn request(&mut self, what: String, make: impl FnOnce(u64) -> Command) -> Command {
        let id = self.next_id;
        self.next_id += 1;
        self.requests.insert(id, what);
        make(id)
    }

    fn order(&mut self, symbol: &str, side: Side) -> Option<Command> {
        let (kind, price) = (self.kind, self.price);
        self.order_at(symbol, side, kind, price)
    }

    /// An order with the ticket's volume and stop/target distances, entering at `price` (market: the quote).
    fn order_at(&mut self, symbol: &str, side: Side, kind: OrderKind, price: f64) -> Option<Command> {
        let spec = self.spec.as_ref()?;
        let (bid, ask) = self.quote?;
        let price = round_to(price, spec.tick_size);
        let entry = match kind {
            OrderKind::Market => if side == Side::Buy { ask } else { bid },
            _ => price,
        };
        let dir = if side == Side::Buy { 1.0 } else { -1.0 };
        let level = |dist: f64, sign: f64| if dist > 0.0 { round_to(entry + sign * dir * dist, spec.tick_size) } else { 0.0 };
        let (sl, tp) = (level(self.sl_dist, -1.0), level(self.tp_dist, 1.0));
        let volume = self.volume;
        let vd = step_decimals(spec.vol_step);
        let what = match kind {
            OrderKind::Market => format!("{} {volume:.vd$} {symbol}", side_label(side)),
            _ => format!("{} {} {volume:.vd$} {symbol} @ {}", side_label(side), kind_label(kind).to_lowercase(), self.fmt_price(price)),
        };
        let symbol = symbol.to_string();
        Some(self.request(what, |id| Command::Order { id, symbol, side, kind, volume, price, sl, tp }))
    }

    fn fmt_price(&self, v: f64) -> String {
        let d = self.spec.as_ref().map(|s| s.digits).unwrap_or(2) as usize;
        if v > 0.0 { format!("{v:.d$}") } else { "—".into() }
    }

    /// Lines for the chart: positions solid, pending orders dashed, stops and targets in their color.
    /// With `draggable`, each line carries the handle that moves it.
    pub fn levels(&self, symbol: &str, pal: &Palette, draggable: bool) -> Vec<Level> {
        let vd = self.spec.as_ref().map(|s| step_decimals(s.vol_step)).unwrap_or(2);
        let color = |side| if side == Side::Buy { pal.up } else { pal.down };
        let handle = |h: Handle| draggable.then_some(h);
        let mut out = Vec::new();
        let stops = |out: &mut Vec<Level>, ticket: u64, sl: f64, tp: f64| {
            if sl > 0.0 {
                out.push(Level { price: sl, color: pal.danger, label: format!("SL #{ticket}"), dashed: true, handle: handle(Handle::Sl(ticket)) });
            }
            if tp > 0.0 {
                out.push(Level { price: tp, color: pal.ok, label: format!("TP #{ticket}"), dashed: true, handle: handle(Handle::Tp(ticket)) });
            }
        };
        for p in self.positions.iter().filter(|p| p.symbol == symbol) {
            let s = if p.side == Side::Buy { "C" } else { "V" };
            out.push(Level {
                price: p.price,
                color: color(p.side),
                label: format!("{s} {:.vd$}   {:+.2}", p.volume, p.profit),
                dashed: false,
                handle: handle(Handle::Position(p.ticket)),
            });
            stops(&mut out, p.ticket, p.sl, p.tp);
        }
        for o in self.orders.iter().filter(|o| o.symbol == symbol) {
            let s = format!("{}{}", if o.side == Side::Buy { "C" } else { "V" }, if o.kind == OrderKind::Limit { "L" } else { "S" });
            out.push(Level {
                price: o.price,
                color: color(o.side).gamma_multiply(0.75),
                label: format!("{s} {:.vd$}", o.volume),
                dashed: true,
                handle: handle(Handle::Order(o.ticket)),
            });
            stops(&mut out, o.ticket, o.sl, o.tp);
        }
        out
    }

    /// Turn what the user did on the chart into commands (the caller checked trading is allowed).
    pub fn chart_action(&mut self, action: ChartAction, symbol: &str) -> Option<Command> {
        let tick = self.spec.as_ref()?.tick_size;
        match action {
            ChartAction::Order { side, kind, price } => self.order_at(symbol, side, kind, price),
            ChartAction::Move { handle, price } => {
                let price = round_to(price, tick);
                let (ticket, new_price, sl, tp, what) = match handle {
                    Handle::Order(t) => {
                        // the order's stop and target keep their distance to the entry
                        let o = self.orders.iter().find(|o| o.ticket == t)?;
                        let d = price - o.price;
                        let shift = |v: f64| if v > 0.0 { round_to(v + d, tick) } else { 0.0 };
                        (t, price, shift(o.sl), shift(o.tp), format!("Mover ordem #{t} para {}", self.fmt_price(price)))
                    }
                    Handle::Position(t) => {
                        // dragged out of the entry line (ProfitChart): towards the gain sets the target,
                        // towards the loss the stop. A buy gains upwards, a sell downwards.
                        let p = self.positions.iter().find(|p| p.ticket == t)?;
                        let losing = if p.side == Side::Buy { price < p.price } else { price > p.price };
                        let (sl, tp) = if losing { (price, p.tp) } else { (p.sl, price) };
                        (t, 0.0, sl, tp, format!("{} #{t} em {}", if losing { "Stop" } else { "Alvo" }, self.fmt_price(price)))
                    }
                    Handle::Sl(t) | Handle::Tp(t) => {
                        let (entry, sl, tp) = self
                            .positions
                            .iter()
                            .find(|p| p.ticket == t)
                            .map(|p| (0.0, p.sl, p.tp))
                            .or_else(|| self.orders.iter().find(|o| o.ticket == t).map(|o| (o.price, o.sl, o.tp)))?;
                        let is_sl = matches!(handle, Handle::Sl(_));
                        let (sl, tp) = if is_sl { (price, tp) } else { (sl, price) };
                        (t, entry, sl, tp, format!("{} #{t} para {}", if is_sl { "Stop" } else { "Alvo" }, self.fmt_price(price)))
                    }
                };
                Some(self.request(what, |id| Command::Modify { id, ticket, price: new_price, sl, tp }))
            }
        }
    }

    /// Net volume of `symbol` (buy positive).
    fn net(&self, symbol: &str) -> f64 {
        self.positions.iter().filter(|p| p.symbol == symbol).map(|p| if p.side == Side::Buy { p.volume } else { -p.volume }).sum()
    }

    /// Reverse the net position: one order of twice the volume on netting, flatten + opposite on hedging.
    fn reverse(&mut self, symbol: &str) -> Vec<Command> {
        let net = self.net(symbol);
        if net == 0.0 {
            return Vec::new();
        }
        let side = if net > 0.0 { Side::Sell } else { Side::Buy };
        let vd = self.spec.as_ref().map(|s| step_decimals(s.vol_step)).unwrap_or(2);
        let (s, volume) = (symbol.to_string(), if self.netting { 2.0 * net.abs() } else { net.abs() });
        let mut out = Vec::new();
        if !self.netting {
            let s = s.clone();
            out.push(self.request(format!("Inverter {symbol}: zerar"), |id| Command::Flatten { id, symbol: s }));
        }
        let what = format!("Inverter {symbol}: {} {volume:.vd$}", side_label(side));
        out.push(self.request(what, |id| Command::Order {
            id,
            symbol: s,
            side,
            kind: OrderKind::Market,
            volume,
            price: 0.0,
            sl: 0.0,
            tp: 0.0,
        }));
        out
    }

    /// Stop at the entry price for every position of `symbol` (target kept). Only positions already
    /// past their entry: otherwise the stop would sit on the wrong side of the price and the broker
    /// refuses it, so it says why here instead of sending.
    fn breakeven(&mut self, symbol: &str) -> Vec<Command> {
        let Some((bid, ask)) = self.quote else { return Vec::new() };
        let mut out = Vec::new();
        let positions: Vec<Position> = self.positions.iter().filter(|p| p.symbol == symbol).cloned().collect();
        for p in positions {
            let in_gain = if p.side == Side::Buy { bid > p.price } else { ask < p.price };
            if !in_gain {
                self.log.push_front((false, format!("BE #{}: a posição ainda não passou do preço de entrada", p.ticket)));
                self.log.truncate(LOG_LINES);
                continue;
            }
            let (ticket, entry, tp) = (p.ticket, p.price, p.tp);
            out.push(self.request(format!("BE #{ticket}"), |id| Command::Modify { id, ticket, price: 0.0, sl: entry, tp }));
        }
        out
    }

    /// Whether orders can go out now; otherwise why not.
    fn blocked(&self, connected: bool, real: bool) -> Option<&'static str> {
        if !connected {
            return Some("sem conexão");
        }
        match &self.funds {
            None => Some("aguardando a conta"),
            Some(f) if !f.trade_allowed => Some("Algo Trading desligado no MT5"),
            _ if real && !self.armed => Some("conta REAL: arme para operar"),
            _ if self.spec.is_none() || self.quote.is_none() => Some("aguardando cotação"),
            _ => None,
        }
    }

    /// The order ticket (right panel). Returns the commands to send.
    pub fn ticket_ui(&mut self, ui: &mut egui::Ui, pal: &Palette, symbol: &str, connected: bool, real: bool) -> Vec<Command> {
        let mut out = Vec::new();
        ui.add_space(10.0);

        if let Some(f) = &self.funds {
            egui::Grid::new("funds").num_columns(2).spacing([10.0, 3.0]).show(ui, |ui| {
                for (k, v) in [("Saldo", f.balance), ("Patrimônio", f.equity), ("Margem livre", f.margin_free)] {
                    ui.label(RichText::new(k).color(pal.text_dim).size(12.0));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(RichText::new(format!("{v:.2} {}", f.currency)).size(12.5));
                    });
                    ui.end_row();
                }
            });
            ui.separator();
        }

        if real {
            let text = if self.armed { "REAL armada: ordens liberadas" } else { "Armar conta REAL" };
            ui.checkbox(&mut self.armed, RichText::new(text).color(pal.danger).strong());
        }

        let spec_step = self.spec.as_ref().map(|s| (s.vol_min, s.vol_max, s.vol_step, s.tick_size, s.digits));
        let (vmin, vmax, vstep, tick, digits) = spec_step.unwrap_or((0.01, 100.0, 0.01, 0.01, 2));
        let vd = step_decimals(vstep);

        ui.label(RichText::new("Volume").color(pal.text_dim).size(12.0));
        ui.horizontal(|ui| {
            if ui.button("−").clicked() {
                self.volume -= vstep;
            }
            ui.add(egui::DragValue::new(&mut self.volume).speed(vstep).range(vmin..=vmax).fixed_decimals(vd));
            if ui.button("+").clicked() {
                self.volume += vstep;
            }
        });
        ui.horizontal(|ui| {
            for mult in [1.0, 2.0, 5.0, 10.0] {
                let v = vmin * mult;
                if ui.small_button(format!("{v:.vd$}")).clicked() {
                    self.volume = v;
                }
            }
        });
        self.volume = round_to(self.volume, vstep).clamp(vmin, vmax);

        ui.horizontal(|ui| {
            for k in [OrderKind::Market, OrderKind::Limit, OrderKind::Stop] {
                if ui.selectable_label(self.kind == k, kind_label(k)).clicked() {
                    self.kind = k;
                    if k != OrderKind::Market && self.price <= 0.0 {
                        self.price = self.quote.map(|q| q.0).unwrap_or(0.0);
                    }
                }
            }
        });
        if self.kind != OrderKind::Market {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Preço").color(pal.text_dim).size(12.0));
                ui.add(egui::DragValue::new(&mut self.price).speed(tick).range(0.0..=f64::MAX).fixed_decimals(digits as usize));
                if ui.small_button("atual").on_hover_text("Preço atual (bid)").clicked() {
                    self.price = self.quote.map(|q| q.0).unwrap_or(self.price);
                }
            });
        }
        egui::Grid::new("sltp").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
            for (name, v) in [("Stop (dist.)", &mut self.sl_dist), ("Alvo (dist.)", &mut self.tp_dist)] {
                ui.label(RichText::new(name).color(pal.text_dim).size(12.0));
                ui.add(egui::DragValue::new(v).speed(tick * 10.0).range(0.0..=f64::MAX).fixed_decimals(digits as usize))
                    .on_hover_text("Distância em preço a partir da entrada; 0 = sem");
                ui.end_row();
            }
        });
        ui.add_space(6.0);

        let blocked = self.blocked(connected, real);
        // Ctrl+Shift+B/S/Z: same locks as the buttons, ignored while typing in a field
        let key = |k| ui.input_mut(|i| i.consume_key(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, k));
        let typing = ui.ctx().egui_wants_keyboard_input();
        let (key_buy, key_sell, key_flat, key_reverse, key_be) = if typing {
            (false, false, false, false, false)
        } else {
            (key(egui::Key::B), key(egui::Key::S), key(egui::Key::Z), key(egui::Key::R), key(egui::Key::E))
        };
        let (bid, ask) = self.quote.unwrap_or((0.0, 0.0));
        let half = (ui.available_width() - ui.spacing().item_spacing.x) / 2.0;
        ui.horizontal(|ui| {
            for (side, price, color) in [(Side::Sell, bid, pal.down), (Side::Buy, ask, pal.up)] {
                let title = if side == Side::Buy { "COMPRAR" } else { "VENDER" };
                let text = RichText::new(format!("{title}\n{price:.p$}", p = digits as usize)).strong().size(14.0).color(Color32::WHITE);
                let btn = egui::Button::new(text).fill(if blocked.is_none() { color } else { pal.tag_bg }).min_size(egui::vec2(half, 52.0));
                let shortcut = if side == Side::Buy { key_buy } else { key_sell };
                let hint = if side == Side::Buy { "Ctrl+Shift+B" } else { "Ctrl+Shift+S" };
                if ui.add_enabled(blocked.is_none(), btn).on_hover_text(hint).clicked() || (shortcut && blocked.is_none()) {
                    out.extend(self.order(symbol, side));
                }
            }
        });

        let net = self.net(symbol);
        let pnl: f64 = self.positions.iter().filter(|p| p.symbol == symbol).map(|p| p.profit).sum();
        let has_any = self.positions.iter().any(|p| p.symbol == symbol) || self.orders.iter().any(|o| o.symbol == symbol);
        let zerar = egui::Button::new(RichText::new(format!("ZERAR {symbol}")).strong()).min_size(egui::vec2(ui.available_width(), 30.0));
        let zerar = ui.add_enabled(blocked.is_none() && has_any, zerar).on_hover_text("Fecha as posições e cancela as ordens do símbolo (Ctrl+Shift+Z)");
        if zerar.clicked() || (key_flat && blocked.is_none() && has_any) {
            let s = symbol.to_string();
            out.push(self.request(format!("Zerar {symbol}"), |id| Command::Flatten { id, symbol: s }));
        }
        let open = net != 0.0 && blocked.is_none();
        let half = (ui.available_width() - ui.spacing().item_spacing.x) / 2.0;
        ui.horizontal(|ui| {
            let inverter = egui::Button::new(RichText::new("INVERTER").strong()).min_size(egui::vec2(half, 26.0));
            let hint = if self.netting { "Ordem oposta com o dobro do volume, netting (Ctrl+Shift+R)" } else { "Zera e abre o lado oposto, hedge (Ctrl+Shift+R)" };
            if ui.add_enabled(open, inverter).on_hover_text(hint).clicked() || (key_reverse && open) {
                out.extend(self.reverse(symbol));
            }
            let be = egui::Button::new(RichText::new("BE").strong()).min_size(egui::vec2(half, 26.0));
            if ui.add_enabled(open, be).on_hover_text("Stop no preço de entrada das posições do símbolo (Ctrl+Shift+E)").clicked() || (key_be && open) {
                out.extend(self.breakeven(symbol));
            }
        });
        if net != 0.0 || pnl != 0.0 {
            let color = if pnl >= 0.0 { pal.up } else { pal.down };
            ui.label(RichText::new(format!("Posição {net:+.vd$}   {pnl:+.2}")).color(color).strong());
        }
        if let Some(why) = blocked {
            ui.label(RichText::new(why).color(pal.warn).size(12.0));
        }

        if !self.log.is_empty() {
            ui.separator();
            for (ok, line) in &self.log {
                ui.label(RichText::new(line).size(11.5).color(if *ok { pal.text_dim } else { pal.danger }));
            }
        }
        out
    }

    /// Positions and pending orders of every symbol, with close/cancel buttons.
    pub fn book_ui(&mut self, ui: &mut egui::Ui, pal: &Palette, can_trade: bool) -> Vec<Command> {
        let mut out = Vec::new();
        let mut close = None;
        let mut cancel = None;
        egui::ScrollArea::vertical().auto_shrink([false, true]).show(ui, |ui| {
            egui::Grid::new("book").num_columns(8).striped(true).spacing([18.0, 4.0]).show(ui, |ui| {
                for h in ["Ticket", "Símbolo", "Tipo", "Volume", "Preço", "SL", "TP", "Resultado"] {
                    ui.label(RichText::new(h).color(pal.text_dim).size(11.5));
                }
                ui.end_row();
                let fmt = |v: f64| self.fmt_price(v);
                for p in &self.positions {
                    ui.label(p.ticket.to_string());
                    ui.label(&p.symbol);
                    ui.label(RichText::new(side_label(p.side)).color(if p.side == Side::Buy { pal.up } else { pal.down }));
                    ui.label(format!("{}", p.volume));
                    ui.label(fmt(p.price));
                    ui.label(fmt(p.sl));
                    ui.label(fmt(p.tp));
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("{:+.2}", p.profit)).color(if p.profit >= 0.0 { pal.up } else { pal.down }).strong());
                        if ui.add_enabled(can_trade, egui::Button::new("Fechar").small()).clicked() {
                            close = Some((p.ticket, p.symbol.clone()));
                        }
                    });
                    ui.end_row();
                }
                for o in &self.orders {
                    ui.label(o.ticket.to_string());
                    ui.label(&o.symbol);
                    ui.label(
                        RichText::new(format!("{} {}", side_label(o.side), kind_label(o.kind).to_lowercase()))
                            .color(if o.side == Side::Buy { pal.up } else { pal.down }),
                    );
                    ui.label(format!("{}", o.volume));
                    ui.label(fmt(o.price));
                    ui.label(fmt(o.sl));
                    ui.label(fmt(o.tp));
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("pendente").color(pal.text_dim));
                        if ui.add_enabled(can_trade, egui::Button::new("Cancelar").small()).clicked() {
                            cancel = Some(o.ticket);
                        }
                    });
                    ui.end_row();
                }
            });
        });
        if let Some((ticket, symbol)) = close {
            out.push(self.request(format!("Fechar #{ticket} {symbol}"), |id| Command::Close { id, ticket }));
        }
        if let Some(ticket) = cancel {
            out.push(self.request(format!("Cancelar #{ticket}"), |id| Command::Cancel { id, ticket }));
        }
        out
    }

    pub fn can_trade(&self, connected: bool, real: bool) -> bool {
        self.blocked(connected, real).is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimals_of_steps() {
        assert_eq!(step_decimals(0.01), 2);
        assert_eq!(step_decimals(1.0), 0);
        assert_eq!(step_decimals(0.25), 2);
        assert_eq!(step_decimals(0.1), 1);
    }

    #[test]
    fn market_order_levels_from_quote() {
        let mut t = Trading::default();
        t.on_message(
            &Message::Symbol { symbol: "X".into(), digits: 2, tick_size: 0.25, vol_min: 0.1, vol_max: 10.0, vol_step: 0.1 },
            "X",
        );
        t.on_message(&Message::Tick { symbol: "X".into(), time_msc: 0, bid: 100.0, ask: 100.5, volume: 0.0 }, "X");
        (t.sl_dist, t.tp_dist) = (10.1, 20.0);
        let Some(Command::Order { side, volume, sl, tp, kind, .. }) = t.order("X", Side::Sell) else { panic!() };
        assert_eq!((side, kind, volume), (Side::Sell, OrderKind::Market, 0.1));
        // sell at the bid: stop above, target below, on the tick grid
        assert_eq!((sl, tp), (110.0, 80.0));
    }

    fn with_book(netting: bool) -> Trading {
        let mut t = Trading { netting, ..Trading::default() };
        t.on_message(&Message::Symbol { symbol: "X".into(), digits: 2, tick_size: 0.25, vol_min: 0.1, vol_max: 10.0, vol_step: 0.1 }, "X");
        t.on_message(&Message::Tick { symbol: "X".into(), time_msc: 0, bid: 100.0, ask: 100.5, volume: 0.0 }, "X");
        t.positions = vec![Position { ticket: 7, symbol: "X".into(), side: Side::Buy, volume: 0.3, price: 99.0, sl: 0.0, tp: 104.0, profit: 0.3 }];
        t.orders = vec![PendingOrder { ticket: 8, symbol: "X".into(), side: Side::Sell, kind: OrderKind::Limit, volume: 0.1, price: 103.0, sl: 105.0, tp: 101.0 }];
        t
    }

    #[test]
    fn chart_drags_become_modifies() {
        let mut t = with_book(true);
        let modify = |c: Option<Command>| match c {
            Some(Command::Modify { ticket, price, sl, tp, .. }) => (ticket, price, sl, tp),
            other => panic!("{other:?}"),
        };
        // a buy dragged below its entry gets a stop, above it a target (on the tick grid)
        assert_eq!(modify(t.chart_action(ChartAction::Move { handle: Handle::Position(7), price: 97.1 }, "X")), (7, 0.0, 97.0, 104.0));
        assert_eq!(modify(t.chart_action(ChartAction::Move { handle: Handle::Position(7), price: 102.6 }, "X")), (7, 0.0, 0.0, 102.5));
        // relative to the entry (99), not the quote: just above the entry is still the target side
        assert_eq!(modify(t.chart_action(ChartAction::Move { handle: Handle::Position(7), price: 99.5 }, "X")), (7, 0.0, 0.0, 99.5));
        // moving a pending order carries its stop and target
        assert_eq!(modify(t.chart_action(ChartAction::Move { handle: Handle::Order(8), price: 104.0 }, "X")), (8, 104.0, 106.0, 102.0));
        // a pending order's stop alone keeps entry and target
        assert_eq!(modify(t.chart_action(ChartAction::Move { handle: Handle::Sl(8), price: 106.3 }, "X")), (8, 103.0, 106.25, 101.0));
        // context menu: a pending order with the ticket's volume
        let Some(Command::Order { side, kind, price, volume, .. }) =
            t.chart_action(ChartAction::Order { side: Side::Buy, kind: OrderKind::Limit, price: 98.1 }, "X")
        else {
            panic!()
        };
        assert_eq!((side, kind, price, volume), (Side::Buy, OrderKind::Limit, 98.0, 0.1));
        assert!(t.chart_action(ChartAction::Move { handle: Handle::Order(99), price: 1.0 }, "X").is_none());
    }

    #[test]
    fn reverse_and_breakeven() {
        let mut t = with_book(true);
        let cmds = t.reverse("X");
        assert!(matches!(cmds[..], [Command::Order { side: Side::Sell, volume, kind: OrderKind::Market, .. }] if (volume - 0.6).abs() < 1e-9));
        let mut h = with_book(false);
        let cmds = h.reverse("X");
        assert!(matches!(cmds[0], Command::Flatten { .. }));
        assert!(matches!(cmds[1], Command::Order { side: Side::Sell, volume, .. } if (volume - 0.3).abs() < 1e-9));
        // bought at 99, bid 100: past the entry
        let be = t.breakeven("X");
        assert!(matches!(be[..], [Command::Modify { ticket: 7, sl, tp, .. }] if sl == 99.0 && tp == 104.0));
        // still below the entry: nothing sent, the reason in the log
        t.positions[0].price = 101.0;
        assert!(t.breakeven("X").is_empty());
        assert!(t.log.front().is_some_and(|(ok, l)| !ok && l.contains("preço de entrada")));
        let levels = t.levels("X", &Palette::default(), true);
        // position entry + its target (no stop), order entry + its stop + its target
        assert_eq!(levels.iter().filter(|l| l.handle.is_some()).count(), 5);
        assert!(t.levels("X", &Palette::default(), false).iter().all(|l| l.handle.is_none()));
    }
}
