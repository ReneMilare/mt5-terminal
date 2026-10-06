//! TCP server the MT5 `TerminalBridge` EA connects to (MQL5 sockets are client-only).
//!
//! One EA connection at a time; a new connection replaces the old one. Lines that fail to parse are
//! reported and skipped, never fatal.

use super::{Command, Event, Feed, Message};
use crossbeam_channel::{unbounded, Sender};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;

pub const DEFAULT_ADDR: &str = "127.0.0.1:47011";

/// Active EA connection and its generation, so a replaced connection never reports a disconnect.
type Shared = Arc<Mutex<(u64, Option<TcpStream>)>>;

pub fn spawn(addr: &str, wake: impl Fn() + Send + Sync + Clone + 'static) -> std::io::Result<Feed> {
    let listener = TcpListener::bind(addr)?;
    let (ev_tx, ev_rx) = unbounded();
    let (cmd_tx, cmd_rx) = unbounded::<Command>();
    let current: Shared = Arc::new(Mutex::new((0, None)));

    {
        let current = current.clone();
        let ev_tx = ev_tx.clone();
        let wake = wake.clone();
        thread::Builder::new().name("bridge-accept".into()).spawn(move || {
            for stream in listener.incoming().flatten() {
                let _ = stream.set_nodelay(true);
                let Ok(writer) = stream.try_clone() else { continue };
                let generation = {
                    let mut cur = current.lock().unwrap();
                    if let Some(old) = cur.1.replace(writer) {
                        let _ = old.shutdown(std::net::Shutdown::Both);
                    }
                    cur.0 += 1;
                    cur.0
                };
                let _ = ev_tx.send(Event::Connected);
                wake();
                let (current, ev_tx, wake) = (current.clone(), ev_tx.clone(), wake.clone());
                let _ = thread::Builder::new().name("bridge-read".into()).spawn(move || {
                    read_loop(stream, &ev_tx, &wake);
                    let mut cur = current.lock().unwrap();
                    if cur.0 == generation {
                        cur.1 = None;
                        drop(cur);
                        let _ = ev_tx.send(Event::Disconnected);
                        wake();
                    }
                });
            }
        })?;
    }

    thread::Builder::new().name("bridge-write".into()).spawn(move || {
        for cmd in cmd_rx {
            let mut line = serde_json::to_string(&cmd).expect("command serializes");
            line.push('\n');
            let mut cur = current.lock().unwrap();
            if let Some(stream) = cur.1.as_mut() {
                // a failed write is detected by the reader, which reports the disconnect
                let _ = stream.write_all(line.as_bytes());
            }
        }
    })?;

    Ok(Feed { events: ev_rx, commands: cmd_tx })
}

fn read_loop(stream: TcpStream, events: &Sender<Event>, wake: &impl Fn()) {
    let reader = BufReader::with_capacity(1 << 20, stream);
    for line in reader.split(b'\n') {
        let Ok(line) = line else { break };
        if line.is_empty() {
            continue;
        }
        let event = match serde_json::from_slice::<Message>(&line) {
            Ok(msg) => Event::Message(msg),
            Err(e) => Event::Message(Message::Error { msg: format!("linha inválida do EA: {e}") }),
        };
        if events.send(event).is_err() {
            break;
        }
        wake();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Timeframe;
    use std::time::Duration;

    #[test]
    fn roundtrip_with_fake_ea() {
        // find a free port, then let the bridge bind it
        let probe = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = probe.local_addr().unwrap().to_string();
        drop(probe);
        let feed = spawn(&addr, || {}).unwrap();

        let mut ea = TcpStream::connect(&addr).unwrap();
        let ev = feed.events.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(matches!(ev, Event::Connected));

        ea.write_all(b"{\"t\":\"tick\",\"symbol\":\"X\",\"time_msc\":5,\"bid\":1.0,\"ask\":1.5}\nnot json\n").unwrap();
        let ev = feed.events.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(matches!(ev, Event::Message(Message::Tick { time_msc: 5, .. })));
        let ev = feed.events.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(matches!(ev, Event::Message(Message::Error { .. })));

        feed.send(Command::History { symbol: "X".into(), tf: Timeframe::M1, count: 3, before: None });
        let mut line = String::new();
        BufReader::new(ea.try_clone().unwrap()).read_line(&mut line).unwrap();
        assert_eq!(line, "{\"t\":\"history\",\"symbol\":\"X\",\"tf\":\"M1\",\"count\":3}\n");

        drop(ea);
        let ev = feed.events.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(matches!(ev, Event::Disconnected));
    }
}
