//! Offline source: a random walk with volatility clustering, so the chart can be developed and
//! demoed without MT5. Speaks the same commands/messages as the bridge.

use super::{Command, Event, Feed, Message, OrderKind, PendingOrder, Position, Side};
use crate::model::{Bar, Series, Timeframe};
use crossbeam_channel::{RecvTimeoutError, unbounded};
use std::collections::HashMap;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const HISTORY_MINUTES: i64 = 60 * 24 * 30;
const TICK_EVERY: Duration = Duration::from_millis(100);
const SPREAD: f64 = 0.5;
const VOL_MIN: f64 = 0.01;
const VOL_STEP: f64 = 0.01;
const VOL_MAX: f64 = 100.0;
/// Account currency per point per lot.
const CONTRACT: f64 = 1.0;

struct Rng(u64);

impl Rng {
    fn next_f64(&mut self) -> f64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Standard normal (Box–Muller).
    fn gauss(&mut self) -> f64 {
        let u = self.next_f64().max(1e-12);
        let v = self.next_f64();
        (-2.0 * u.ln()).sqrt() * (std::f64::consts::TAU * v).cos()
    }
}

struct Walk {
    rng: Rng,
    price: f64,
    vol: f64,
    base_vol: f64,
    m1: Series,
}

impl Walk {
    fn new(symbol: &str, now: i64) -> Self {
        let seed = symbol.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
            (h ^ b as u64).wrapping_mul(0x100_0000_01b3)
        });
        let start = match symbol {
            "UsaTec" => 31_000.0,
            "UsaInd" => 51_000.0,
            "UsaRus" => 2_600.0,
            _ => 1_000.0,
        };
        let base_vol = start * 0.00012; // per-minute sigma (~0.5% a day before clustering)
        let mut walk = Walk { rng: Rng(seed | 1), price: start, vol: base_vol, base_vol, m1: Series::default() };
        walk.m1.digits = 2;
        let first = Timeframe::M1.bar_open(now) - HISTORY_MINUTES * 60;
        for minute in 0..HISTORY_MINUTES {
            let t = first + minute * 60;
            for k in 0..4 {
                let p = walk.step(0.5);
                walk.m1.apply_tick(Timeframe::M1, t + k * 15, p, 1.0 + walk.rng.next_f64() * 9.0);
            }
        }
        walk
    }

    /// One price move scaled by `frac` of a minute's variance.
    fn step(&mut self, frac: f64) -> f64 {
        // GARCH-like clustering, mean-reverting to the base volatility
        let shock = self.rng.gauss();
        self.vol = (0.98 * self.vol + 0.02 * self.base_vol + 0.05 * self.base_vol * shock.abs()).max(self.base_vol * 0.2);
        self.price += self.vol * frac.sqrt() * self.rng.gauss();
        (self.price * 100.0).round() / 100.0
    }

    /// (bid, ask) at the current price.
    fn quote(&self) -> (f64, f64) {
        let bid = (self.price * 100.0).round() / 100.0;
        (bid, bid + SPREAD)
    }

    /// The last `count` bars of `tf`, or the `count` before `before`.
    fn bars(&self, tf: Timeframe, count: u32, before: Option<i64>) -> Vec<[f64; 6]> {
        let mut out: Vec<Bar> = Vec::new();
        for b in &self.m1.bars {
            let open = tf.bar_open(b.time);
            match out.last_mut() {
                Some(cur) if cur.time == open => {
                    cur.high = cur.high.max(b.high);
                    cur.low = cur.low.min(b.low);
                    cur.close = b.close;
                    cur.volume += b.volume;
                }
                _ => out.push(Bar { time: open, ..*b }),
            }
        }
        let end = before.map(|t| out.partition_point(|b| b.time < t)).unwrap_or(out.len());
        let skip = end.saturating_sub(count as usize);
        out[skip..end].iter().map(|b| [b.time as f64, b.open, b.high, b.low, b.close, b.volume]).collect()
    }
}

/// Paper broker: fills against the synthetic quotes, one position per fill (hedging, like most MT5
/// CFD accounts). Pending orders, SL and TP trigger on the ticks this feed generates.
struct Paper {
    next_ticket: u64,
    balance: f64,
    day_start: i64,
    day_balance: f64,
    positions: Vec<Position>,
    orders: Vec<PendingOrder>,
}

fn result(id: u64, ok: bool, retcode: u32, msg: &str, ticket: u64, price: f64) -> Message {
    Message::TradeResult { id, ok, retcode, msg: msg.into(), ticket, price }
}

fn floating(p: &Position, bid: f64, ask: f64) -> f64 {
    match p.side {
        Side::Buy => (bid - p.price) * p.volume * CONTRACT,
        Side::Sell => (p.price - ask) * p.volume * CONTRACT,
    }
}

impl Paper {
    fn new() -> Self {
        Self { next_ticket: 1000, balance: 10_000.0, day_start: now_secs().div_euclid(86400) * 86400, day_balance: 10_000.0, positions: Vec::new(), orders: Vec::new() }
    }

    fn roll_day(&mut self, now: i64) {
        let day = now.div_euclid(86400) * 86400;
        if day != self.day_start {
            self.day_start = day;
            self.day_balance = self.balance;
        }
    }

    fn ticket(&mut self) -> u64 {
        self.next_ticket += 1;
        self.next_ticket
    }

    fn open(&mut self, symbol: &str, side: Side, volume: f64, price: f64, sl: f64, tp: f64) -> u64 {
        let ticket = self.ticket();
        self.positions.push(Position { ticket, symbol: symbol.into(), side, volume, price, sl, tp, profit: 0.0 });
        ticket
    }

    #[allow(clippy::too_many_arguments)]
    fn order(
        &mut self,
        id: u64,
        symbol: &str,
        side: Side,
        kind: OrderKind,
        volume: f64,
        price: f64,
        sl: f64,
        tp: f64,
        (bid, ask): (f64, f64),
    ) -> Message {
        let steps = volume / VOL_STEP;
        if !(VOL_MIN..=VOL_MAX).contains(&volume) || (steps - steps.round()).abs() > 1e-6 {
            return result(id, false, 10014, "volume inválido", 0, 0.0);
        }
        let valid = match (kind, side) {
            (OrderKind::Market, _) => true,
            (OrderKind::Limit, Side::Buy) | (OrderKind::Stop, Side::Sell) => {
                price > 0.0 && price < if side == Side::Buy { ask } else { bid }
            }
            (OrderKind::Limit, Side::Sell) | (OrderKind::Stop, Side::Buy) => {
                price > if side == Side::Buy { ask } else { bid }
            },
        };
        if !valid {
            return result(id, false, 10015, "preço inválido", 0, 0.0);
        }
        if kind == OrderKind::Market {
            let fill = if side == Side::Buy { ask } else { bid };
            let ticket = self.open(symbol, side, volume, fill, sl, tp);
            return result(id, true, 10009, "executada", ticket, fill);
        }
        let ticket = self.ticket();
        self.orders.push(PendingOrder { ticket, symbol: symbol.into(), side, kind, volume, price, sl, tp });
        result(id, true, 10008, "ordem colocada", ticket, price)
    }

    fn close(&mut self, id: u64, ticket: u64, quote: impl Fn(&str) -> (f64, f64)) -> Message {
        self.roll_day(now_secs());
        let Some(i) = self.positions.iter().position(|p| p.ticket == ticket) else {
            return result(id, false, 10036, "posição não encontrada", ticket, 0.0);
        };
        let p = self.positions.remove(i);
        let (bid, ask) = quote(&p.symbol);
        self.balance += floating(&p, bid, ask);
        result(id, true, 10009, "fechada", ticket, if p.side == Side::Buy { bid } else { ask })
    }

    /// Move a pending order or a position's stops. Stops must be on the losing/winning side of the
    /// reference (entry for orders, the closing quote for positions), as brokers require.
    fn modify(&mut self, id: u64, ticket: u64, price: f64, sl: f64, tp: f64, quote: impl Fn(&str) -> (f64, f64)) -> Message {
        let stops_ok = |side: Side, reference: f64| {
            let (lo, hi) = if side == Side::Buy { (sl, tp) } else { (tp, sl) };
            (lo <= 0.0 || lo < reference) && (hi <= 0.0 || hi > reference)
        };
        if let Some(p) = self.positions.iter_mut().find(|p| p.ticket == ticket) {
            let (bid, ask) = quote(&p.symbol);
            if !stops_ok(p.side, if p.side == Side::Buy { bid } else { ask }) {
                return result(id, false, 10016, "stops inválidos", ticket, 0.0);
            }
            (p.sl, p.tp) = (sl, tp);
            return result(id, true, 10009, "stops alterados", ticket, 0.0);
        }
        let Some(i) = self.orders.iter().position(|o| o.ticket == ticket) else {
            return result(id, false, 10036, "ordem não encontrada", ticket, 0.0);
        };
        let o = self.orders[i].clone();
        let (bid, ask) = quote(&o.symbol);
        let reference = if o.side == Side::Buy { ask } else { bid };
        let side_ok = match (o.kind, o.side) {
            (OrderKind::Limit, Side::Buy) | (OrderKind::Stop, Side::Sell) => price < reference,
            _ => price > reference,
        };
        if price <= 0.0 || !side_ok {
            return result(id, false, 10015, "preço inválido", ticket, 0.0);
        }
        if !stops_ok(o.side, price) {
            return result(id, false, 10016, "stops inválidos", ticket, 0.0);
        }
        let o = &mut self.orders[i];
        (o.price, o.sl, o.tp) = (price, sl, tp);
        result(id, true, 10009, "ordem alterada", ticket, price)
    }

    fn cancel(&mut self, id: u64, ticket: u64) -> Message {
        match self.orders.iter().position(|o| o.ticket == ticket) {
            Some(i) => {
                self.orders.remove(i);
                result(id, true, 10009, "ordem cancelada", ticket, 0.0)
            }
            None => result(id, false, 10036, "ordem não encontrada", ticket, 0.0),
        }
    }

    fn flatten(&mut self, id: u64, symbol: &str, quote: impl Fn(&str) -> (f64, f64)) -> Vec<Message> {
        let mut out = Vec::new();
        let tickets: Vec<u64> = self.positions.iter().filter(|p| p.symbol == symbol).map(|p| p.ticket).collect();
        for t in tickets {
            out.push(self.close(id, t, &quote));
        }
        let orders: Vec<u64> = self.orders.iter().filter(|o| o.symbol == symbol).map(|o| o.ticket).collect();
        for t in orders {
            out.push(self.cancel(id, t));
        }
        if out.is_empty() {
            out.push(result(id, true, 0, "nada a zerar", 0, 0.0));
        }
        out
    }

    /// Trigger pending orders, SL and TP of `symbol` and refresh the floating results.
    fn on_quote(&mut self, symbol: &str, bid: f64, ask: f64) {
        self.roll_day(now_secs());
        let mut filled = Vec::new();
        self.orders.retain(|o| {
            let hit = o.symbol == symbol
                && match (o.kind, o.side) {
                    (OrderKind::Limit, Side::Buy) => ask <= o.price,
                    (OrderKind::Stop, Side::Buy) => ask >= o.price,
                    (OrderKind::Limit, Side::Sell) => bid >= o.price,
                    (OrderKind::Stop, Side::Sell) => bid <= o.price,
                    (OrderKind::Market, _) => true,
                };
            if hit {
                filled.push(o.clone());
            }
            !hit
        });
        for o in filled {
            let fill = if o.side == Side::Buy { ask } else { bid };
            self.open(symbol, o.side, o.volume, fill, o.sl, o.tp);
        }
        let mut realized = 0.0;
        self.positions.retain_mut(|p| {
            if p.symbol != symbol {
                return true;
            }
            p.profit = floating(p, bid, ask);
            let exit = if p.side == Side::Buy { bid } else { ask };
            let stop = p.sl > 0.0 && if p.side == Side::Buy { exit <= p.sl } else { exit >= p.sl };
            let take = p.tp > 0.0 && if p.side == Side::Buy { exit >= p.tp } else { exit <= p.tp };
            if stop || take {
                realized += p.profit;
            }
            !(stop || take)
        });
        self.balance += realized;
    }

    fn state(&mut self) -> [Message; 4] {
        self.roll_day(now_secs());
        let floating: f64 = self.positions.iter().map(|p| p.profit).sum();
        [
            Message::Positions { positions: self.positions.clone() },
            Message::Orders { orders: self.orders.clone() },
            Message::Account {
                balance: self.balance,
                equity: self.balance + floating,
                margin_free: self.balance + floating,
                currency: "USD".into(),
                trade_allowed: true,
            },
            Message::DailyResult { day_start: self.day_start, realized: Some(self.balance - self.day_balance), floating, currency: "USD".into() },
        ]
    }
}

fn now_secs() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

pub fn spawn(wake: impl Fn() + Send + 'static) -> Feed {
    let (ev_tx, ev_rx) = unbounded();
    let (cmd_tx, cmd_rx) = unbounded::<Command>();

    thread::Builder::new()
        .name("synthetic".into())
        .spawn(move || {
            let mut walks: HashMap<String, Walk> = HashMap::new();
            let mut subscribed: Vec<String> = Vec::new();
            let mut paper = Paper::new();
            let send = |msgs: Vec<Message>| {
                let ok = msgs.into_iter().all(|m| ev_tx.send(Event::Message(m)).is_ok());
                wake();
                ok
            };
            let _ = ev_tx.send(Event::Connected);
            let hello = Message::Hello {
                symbol: "UsaTec".into(),
                digits: 2,
                server: "Sintético".into(),
                login: 0,
                account: "demo".into(),
                version: super::BRIDGE_VERSION,
                netting: false,
            };
            send(std::iter::once(hello).chain(paper.state()).collect());
            loop {
                let cmd = match cmd_rx.recv_timeout(TICK_EVERY) {
                    Ok(cmd) => cmd,
                    Err(RecvTimeoutError::Timeout) => {
                        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
                        // quotes move for every symbol the account holds, ticks go out for the subscribed
                        let mut symbols = subscribed.clone();
                        for s in paper.positions.iter().map(|p| &p.symbol).chain(paper.orders.iter().map(|o| &o.symbol)) {
                            if !symbols.contains(s) {
                                symbols.push(s.clone());
                            }
                        }
                        let mut out = Vec::new();
                        for symbol in &symbols {
                            let walk = walks.entry(symbol.clone()).or_insert_with(|| Walk::new(symbol, now_secs()));
                            let price = walk.step(TICK_EVERY.as_secs_f64() / 60.0);
                            // same volume per minute as the history (4 ticks of 1..10 a minute)
                            let volume = (1.0 + walk.rng.next_f64() * 9.0) * 4.0 * TICK_EVERY.as_secs_f64() / 60.0;
                            walk.m1.apply_tick(Timeframe::M1, now.as_secs() as i64, price, volume);
                            let (bid, ask) = walk.quote();
                            paper.on_quote(symbol, bid, ask);
                            if subscribed.contains(symbol) {
                                out.push(Message::Tick { symbol: symbol.clone(), time_msc: now.as_millis() as i64, bid, ask, volume });
                            }
                        }
                        if !paper.positions.is_empty() || !paper.orders.is_empty() {
                            out.extend(paper.state());
                        }
                        if !send(out) {
                            return;
                        }
                        continue;
                    }
                    Err(RecvTimeoutError::Disconnected) => return,
                };
                let mut quote = |symbol: &str| {
                    walks.entry(symbol.to_string()).or_insert_with(|| Walk::new(symbol, now_secs())).quote()
                };
                let out: Vec<Message> = match cmd {
                    Command::History { symbol, tf, count, before } => {
                        let walk = walks.entry(symbol.clone()).or_insert_with(|| Walk::new(&symbol, now_secs()));
                        let bars = walk.bars(tf, count, before);
                        if before.is_some() {
                            vec![Message::Bars { symbol, tf, digits: 2, before, bars }]
                        } else {
                            vec![
                            Message::Symbol {
                                symbol: symbol.clone(),
                                digits: 2,
                                tick_size: 0.01,
                                tick_value_profit: 0.01 * CONTRACT,
                                tick_value_loss: 0.01 * CONTRACT,
                                vol_min: VOL_MIN,
                                vol_max: VOL_MAX,
                                vol_step: VOL_STEP,
                            },
                            Message::Bars { symbol, tf, digits: 2, before, bars },
                        ]
                        }
                    }
                    Command::Subscribe { symbols } => {
                        subscribed = symbols;
                        continue;
                    }
                    // no tick history here: the live bars' delta comes from the ticks in the app
                    Command::Probe { .. } | Command::Objects { .. } | Command::Delta { .. } => {
                        continue;
                    }
                    Command::Order { id, symbol, side, kind, volume, price, sl, tp } => {
                        let q = quote(&symbol);
                        let r = paper.order(id, &symbol, side, kind, volume, price, sl, tp, q);
                        std::iter::once(r).chain(paper.state()).collect()
                    }
                    Command::Close { id, ticket } => {
                        let symbols: HashMap<String, (f64, f64)> =
                            paper.positions.iter().map(|p| (p.symbol.clone(), quote(&p.symbol))).collect();
                        let r = paper.close(id, ticket, |s| symbols[s]);
                        std::iter::once(r).chain(paper.state()).collect()
                    }
                    Command::Cancel { id, ticket } => std::iter::once(paper.cancel(id, ticket)).chain(paper.state()).collect(),
                    Command::Modify { id, ticket, price, sl, tp } => {
                        let symbols: HashMap<String, (f64, f64)> = paper
                            .positions
                            .iter()
                            .map(|p| p.symbol.clone())
                            .chain(paper.orders.iter().map(|o| o.symbol.clone()))
                            .map(|s| {
                                let q = quote(&s);
                                (s, q)
                            })
                            .collect();
                        let r = paper.modify(id, ticket, price, sl, tp, |s| {
                            symbols.get(s).copied().unwrap_or((0.0, 0.0))
                        });
                        std::iter::once(r).chain(paper.state()).collect()
                    }
                    Command::Flatten { id, symbol } => {
                        let q = quote(&symbol);
                        let mut r = paper.flatten(id, &symbol, |_| q);
                        r.extend(paper.state());
                        r
                    }
                };
                if !send(out) {
                    return;
                }
            }
        })
        .expect("spawn synthetic feed");

    Feed { events: ev_rx, commands: cmd_tx }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paper_daily_result_separates_closed_and_open_and_resets_at_midnight() {
        let mut p = Paper::new();
        p.order(1, "X", Side::Buy, OrderKind::Market, 1.0, 0.0, 0.0, 0.0, (100.0, 100.5));
        p.on_quote("X", 102.0, 102.5);
        assert!(matches!(p.state()[3], Message::DailyResult { realized: Some(0.0), floating: 1.5, .. }));
        p.close(2, p.positions[0].ticket, |_| (102.0, 102.5));
        assert!(matches!(p.state()[3], Message::DailyResult { realized: Some(1.5), floating: 0.0, .. }));
        p.day_start -= 86400;
        p.order(3, "X", Side::Sell, OrderKind::Market, 1.0, 0.0, 0.0, 0.0, (102.0, 102.5));
        p.on_quote("X", 99.5, 100.0);
        assert!(matches!(p.state()[3], Message::DailyResult { realized: Some(0.0), floating: 2.0, .. }));
        p.close(4, p.positions[0].ticket, |_| (99.5, 100.0));
        assert!(matches!(p.state()[3], Message::DailyResult { realized: Some(2.0), floating: 0.0, .. }));
        assert_eq!(p.balance, 10003.5);
    }

    #[test]
    fn aggregates_history() {
        let walk = Walk::new("UsaTec", 1_700_000_000);
        let m1 = walk.bars(Timeframe::M1, 10, None);
        let h1 = walk.bars(Timeframe::H1, 5, None);
        let older = walk.bars(Timeframe::H1, 3, Some(h1[0][0] as i64));
        assert_eq!(older.len(), 3);
        assert_eq!(older[2][0] + 3600.0, h1[0][0]);
        assert_eq!(m1.len(), 10);
        assert_eq!(h1.len(), 5);
        for b in h1.iter().chain(&m1) {
            assert!(b[2] >= b[1].max(b[4]) && b[3] <= b[1].min(b[4]), "OHLC consistente: {b:?}");
            assert_eq!(b[0] as i64 % 60, 0);
        }
        assert_eq!(h1[1][0] - h1[0][0], 3600.0);
    }

    #[test]
    fn paper_fills_and_triggers() {
        let mut p = Paper::new();
        let q = (100.0, 100.5);
        assert!(matches!(p.order(1, "X", Side::Buy, OrderKind::Market, 0.015, 0.0, 0.0, 0.0, q), Message::TradeResult { ok: false, .. }));
        assert!(matches!(p.order(2, "X", Side::Buy, OrderKind::Limit, 1.0, 101.0, 0.0, 0.0, q), Message::TradeResult { ok: false, .. }));
        p.order(3, "X", Side::Buy, OrderKind::Market, 2.0, 0.0, 99.0, 0.0, q);
        p.order(4, "X", Side::Sell, OrderKind::Limit, 1.0, 102.0, 0.0, 0.0, q);
        assert_eq!((p.positions[0].price, p.orders.len()), (100.5, 1));

        p.on_quote("X", 102.0, 102.5); // limit sell fills at the bid
        assert_eq!(p.orders.len(), 0);
        assert_eq!(p.positions.len(), 2);
        assert_eq!(p.positions[0].profit, 3.0); // (102 - 100.5) * 2

        p.on_quote("X", 99.0, 99.5); // stop of the buy
        assert_eq!(p.positions.len(), 1);
        assert_eq!(p.balance, 10_000.0 - 3.0);
        let closed = p.flatten(5, "X", |_| (99.0, 99.5));
        assert!(matches!(closed[0], Message::TradeResult { ok: true, price: 99.5, .. }));
        assert_eq!(p.balance, 10_000.0 - 3.0 + 2.5);
        assert!(p.positions.is_empty());
    }

    #[test]
    fn paper_modifies_orders_and_stops() {
        let mut p = Paper::new();
        let q = (100.0, 100.5);
        p.order(1, "X", Side::Buy, OrderKind::Market, 1.0, 0.0, 0.0, 0.0, q);
        p.order(2, "X", Side::Buy, OrderKind::Limit, 1.0, 98.0, 0.0, 0.0, q);
        let (pos, ord) = (p.positions[0].ticket, p.orders[0].ticket);
        let ok = |m: Message| matches!(m, Message::TradeResult { ok: true, .. });
        assert!(ok(p.modify(3, pos, 0.0, 99.0, 103.0, |_| q)));
        assert_eq!((p.positions[0].sl, p.positions[0].tp), (99.0, 103.0));
        assert!(!ok(p.modify(4, pos, 0.0, 100.5, 0.0, |_| q)), "stop above the bid of a buy");
        assert!(ok(p.modify(5, ord, 97.0, 95.0, 0.0, |_| q)));
        assert_eq!((p.orders[0].price, p.orders[0].sl), (97.0, 95.0));
        assert!(!ok(p.modify(6, ord, 101.0, 0.0, 0.0, |_| q)), "buy limit above the ask");
    }
}
