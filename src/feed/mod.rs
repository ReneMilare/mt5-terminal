//! Market data sources. The UI only sees [`Feed`]: commands go in, events come out.
//!
//! Wire protocol with the MT5 bridge EA: one JSON object per line (`\n`), tagged by `"t"`.
//! See `docs/protocol.md`; the EA is `mql5/TerminalBridge.mq5`.

pub mod bridge;
pub mod synthetic;

use crate::model::{Bar, Timeframe};
use crossbeam_channel::{Receiver, Sender};
use serde::{Deserialize, Serialize};

/// App -> source.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Command {
    /// Send the last `count` closed + forming bars of `symbol` on `tf`; with `before`, the `count`
    /// bars before that time instead (older history, loaded as the chart scrolls back).
    History {
        symbol: String,
        tf: Timeframe,
        count: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        before: Option<i64>,
    },
    /// Stream ticks for these symbols (replaces the previous list).
    Subscribe { symbols: Vec<String> },
    /// New order. `price` is ignored for market orders; `sl`/`tp` of 0 mean none.
    /// Answered by one [`Message::TradeResult`] with the same `id`.
    Order {
        id: u64,
        symbol: String,
        side: Side,
        kind: OrderKind,
        volume: f64,
        #[serde(default)]
        price: f64,
        #[serde(default)]
        sl: f64,
        #[serde(default)]
        tp: f64,
    },
    /// Close a whole position at market.
    Close { id: u64, ticket: u64 },
    /// Delete a pending order.
    Cancel { id: u64, ticket: u64 },
    /// Close every position and delete every pending order of `symbol` (one result per request sent).
    Flatten { id: u64, symbol: String },
    /// Buy/sell volume (tick rule) of the last `count` closed bars, newest first, in several
    /// [`Message::Delta`] batches computed by the EA in slices. With `row` > 0 each bar also brings
    /// its POC: the middle of the `row`-high price level with the most counted ticks.
    Delta {
        symbol: String,
        tf: Timeframe,
        count: u32,
        #[serde(default, skip_serializing_if = "is_zero")]
        row: f64,
    },
    /// Change a pending order (`price`, `sl`, `tp`) or a position's stops (`sl`, `tp`; `price` ignored).
    /// All values absolute; 0 removes a stop.
    Modify {
        id: u64,
        ticket: u64,
        #[serde(default)]
        price: f64,
        #[serde(default)]
        sl: f64,
        #[serde(default)]
        tp: f64,
    },
    /// Diagnostics: last `count` values of buffer `buffer` of the indicator whose short name starts with
    /// `indicator`, on the MT5 chart of `symbol`/`tf`. Answered by [`Message::Probe`].
    Probe { id: u64, symbol: String, tf: Timeframe, indicator: String, buffer: u32, count: u32 },
    /// Diagnostics: text and price of the chart objects whose name starts with `prefix`.
    Objects { id: u64, symbol: String, tf: Timeframe, prefix: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChartObject {
    pub name: String,
    pub text: String,
    pub price: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Buy,
    Sell,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderKind {
    Market,
    Limit,
    Stop,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Position {
    pub ticket: u64,
    pub symbol: String,
    pub side: Side,
    pub volume: f64,
    /// Open price.
    pub price: f64,
    #[serde(default)]
    pub sl: f64,
    #[serde(default)]
    pub tp: f64,
    /// Floating result in the account currency, swap included.
    #[serde(default)]
    pub profit: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PendingOrder {
    pub ticket: u64,
    pub symbol: String,
    pub side: Side,
    pub kind: OrderKind,
    pub volume: f64,
    pub price: f64,
    #[serde(default)]
    pub sl: f64,
    #[serde(default)]
    pub tp: f64,
}

/// Source -> app, as it travels on the wire.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Message {
    Hello {
        symbol: String,
        digits: u32,
        server: String,
        login: i64,
        /// "demo", "contest" or "real".
        account: String,
        /// Protocol version of the EA (absent = 1).
        #[serde(default = "first_version")]
        version: u32,
        /// Netting account: one position per symbol, opposite orders reduce or reverse it.
        #[serde(default)]
        netting: bool,
    },
    Bars {
        symbol: String,
        tf: Timeframe,
        digits: u32,
        /// Echo of the request's `before`; with it, an empty list means there is no older history.
        #[serde(default)]
        before: Option<i64>,
        /// `[time, open, high, low, close, volume]`
        bars: Vec<[f64; 6]>,
    },
    Tick {
        symbol: String,
        time_msc: i64,
        bid: f64,
        ask: f64,
        #[serde(default)]
        volume: f64,
    },
    /// Trading rules of a symbol, sent with its history.
    Symbol {
        symbol: String,
        digits: u32,
        tick_size: f64,
        vol_min: f64,
        vol_max: f64,
        vol_step: f64,
    },
    /// Sent when it changes.
    Account {
        balance: f64,
        equity: f64,
        margin_free: f64,
        currency: String,
        /// Terminal, account and EA all allow trading ("Algo Trading" on).
        trade_allowed: bool,
    },
    /// Every open position of the account (all symbols), sent when anything in it changes.
    Positions {
        positions: Vec<Position>,
    },
    /// Every pending order of the account, sent when it changes.
    Orders {
        orders: Vec<PendingOrder>,
    },
    /// Outcome of an order/close/cancel/flatten request.
    TradeResult {
        id: u64,
        ok: bool,
        #[serde(default)]
        retcode: u32,
        #[serde(default)]
        msg: String,
        #[serde(default)]
        ticket: u64,
        #[serde(default)]
        price: f64,
    },
    /// `[time, buy, sell]` per closed bar, plus `poc` when the request had a `row` (bars without
    /// ticks are left out).
    Delta {
        symbol: String,
        tf: Timeframe,
        bars: Vec<Vec<f64>>,
    },
    Probe {
        id: u64,
        indicator: String,
        buffer: u32,
        times: Vec<i64>,
        /// `null` = empty value.
        values: Vec<Option<f64>>,
    },
    Objects {
        id: u64,
        items: Vec<ChartObject>,
    },
    Error {
        msg: String,
    },
}

/// Protocol version this app speaks; older EAs lack the POC per bar (v6), the delta (v5), modify + netting flag (v4), paged history (v3),
/// the history queue and probes (v2).
pub const BRIDGE_VERSION: u32 = 6;

fn is_zero(v: &f64) -> bool {
    *v == 0.0
}

fn first_version() -> u32 {
    1
}

impl Message {
    pub fn decode_bars(raw: &[[f64; 6]]) -> Vec<Bar> {
        raw.iter()
            .map(|b| Bar { time: b[0] as i64, open: b[1], high: b[2], low: b[3], close: b[4], volume: b[5] })
            .collect()
    }
}

#[derive(Clone, Debug)]
pub enum Event {
    Connected,
    Disconnected,
    Message(Message),
}

pub struct Feed {
    pub events: Receiver<Event>,
    pub commands: Sender<Command>,
}

impl Feed {
    pub fn send(&self, cmd: Command) {
        let _ = self.commands.send(cmd);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_format() {
        let cmd = Command::History { symbol: "UsaTec".into(), tf: Timeframe::M5, count: 10, before: None };
        assert_eq!(serde_json::to_string(&cmd).unwrap(), r#"{"t":"history","symbol":"UsaTec","tf":"M5","count":10}"#);
        let cmd = Command::History { symbol: "X".into(), tf: Timeframe::M1, count: 2, before: Some(600) };
        assert_eq!(serde_json::to_string(&cmd).unwrap(), r#"{"t":"history","symbol":"X","tf":"M1","count":2,"before":600}"#);
        let m: Message =
            serde_json::from_str(r#"{"t":"tick","symbol":"UsaTec","time_msc":1,"bid":2.5,"ask":2.75}"#).unwrap();
        assert_eq!(m, Message::Tick { symbol: "UsaTec".into(), time_msc: 1, bid: 2.5, ask: 2.75, volume: 0.0 });
        let m: Message = serde_json::from_str(
            r#"{"t":"bars","symbol":"X","tf":"H1","digits":2,"bars":[[3600,1,2,0.5,1.5,10]]}"#,
        )
        .unwrap();
        let Message::Bars { bars, .. } = m else { panic!() };
        assert_eq!(Message::decode_bars(&bars)[0].low, 0.5);
    }

    #[test]
    fn trade_wire_format() {
        let cmd = Command::Order {
            id: 7,
            symbol: "UsaTec".into(),
            side: Side::Buy,
            kind: OrderKind::Limit,
            volume: 0.1,
            price: 100.5,
            sl: 0.0,
            tp: 0.0,
        };
        assert_eq!(
            serde_json::to_string(&cmd).unwrap(),
            r#"{"t":"order","id":7,"symbol":"UsaTec","side":"buy","kind":"limit","volume":0.1,"price":100.5,"sl":0.0,"tp":0.0}"#
        );
        let m: Message = serde_json::from_str(
            r#"{"t":"positions","positions":[{"ticket":12,"symbol":"X","side":"sell","volume":1,"price":2.5,"profit":-3.25}]}"#,
        )
        .unwrap();
        let Message::Positions { positions } = m else { panic!() };
        assert_eq!((positions[0].side, positions[0].sl, positions[0].profit), (Side::Sell, 0.0, -3.25));
        let m: Message = serde_json::from_str(r#"{"t":"trade_result","id":7,"ok":false,"retcode":10019,"msg":"sem margem"}"#).unwrap();
        assert!(matches!(m, Message::TradeResult { id: 7, ok: false, retcode: 10019, .. }));
    }
}
