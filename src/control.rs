//! `mt5-terminal ctl ...`: commands to the running app (like `hyprctl` for Hyprland), over a Unix
//! socket only this user can open (`$XDG_RUNTIME_DIR/mt5-terminal.sock`). One line in, one reply out.
//! It configures and inspects; it never sends orders.
//!
//! Replies never wait for the UI (a window on a hidden workspace isn't drawn, so its frame loop
//! sleeps): `state` reads a snapshot the app publishes, settings are written to config.toml (which the
//! app applies when it draws again) and symbol/timeframe changes are queued.

use crate::chart::Layer;
use crate::model::Timeframe;
use crate::settings::{self, Grid, Settings};
use crossbeam_channel::{unbounded, Receiver, Sender};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const HELP: &str = "\
Comandos (mt5-terminal ctl <comando>):
  state                    estado em JSON: símbolo, timeframe, conta, camadas, posições...
  symbol <SÍMBOLO>         troca o símbolo do gráfico
  tf <M1|M5|M15|M30|H1|H4|D1|W1>
                           (symbol e tf valem para o gráfico ativo)
  layout <1|2|2v|3|4|6>    gráficos lado a lado: 1, 2 lado a lado, 2 empilhados, 3, 2×2, 3×2
  chart <N>                ativa o gráfico N (1 = o primeiro, da esquerda para a direita)
  studies <on|off>         liga/desliga os indicadores (grava no config.toml)
  layers <a,b,c,d>         ordem de desenho, de trás para a frente: levels, indicators, trades, price
  front <camada>           põe uma camada na frente (grava no config.toml)
  reload                   relê o config.toml
  config                   caminho do config.toml
  help";

/// What only the UI thread can do.
#[derive(Debug, PartialEq)]
pub enum Action {
    Symbol(String),
    Timeframe(Timeframe),
    Layout(Grid),
    /// Activate chart N (1-based).
    Chart(usize),
    Reload,
}

/// Published by the app every few frames, read by `state`.
pub struct Shared {
    pub state: Mutex<String>,
    pub last_frame: Mutex<Instant>,
}

impl Shared {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { state: Mutex::new("{}".into()), last_frame: Mutex::new(Instant::now()) })
    }
}

pub fn socket_path() -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    dir.join("mt5-terminal.sock")
}

/// Listen for commands (call only from the single running instance: it replaces a stale socket).
pub fn serve(shared: Arc<Shared>, wake: impl Fn() + Send + Clone + 'static) -> std::io::Result<Receiver<Action>> {
    let path = socket_path();
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    let (tx, rx) = unbounded();
    std::thread::Builder::new().name("control".into()).spawn(move || {
        for stream in listener.incoming().flatten() {
            let (tx, wake, shared) = (tx.clone(), wake.clone(), shared.clone());
            let _ = std::thread::Builder::new().name("control-conn".into()).spawn(move || {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let mut line = String::new();
                let Ok(read) = stream.try_clone() else { return };
                if BufReader::new(read.take(4096)).read_line(&mut line).is_err() {
                    return;
                }
                let reply = execute(line.trim(), &shared, &tx);
                wake();
                let mut w = stream;
                let _ = w.write_all(format!("{reply}\n").as_bytes());
            });
        }
    })?;
    Ok(rx)
}

/// Run one command line; the reply text (errors start with "erro").
fn execute(line: &str, shared: &Shared, tx: &Sender<Action>) -> String {
    let mut words = line.split_whitespace();
    let (cmd, arg) = (words.next().unwrap_or("help"), words.next().unwrap_or(""));
    let idle = shared.last_frame.lock().map(|t| t.elapsed() > Duration::from_secs(1)).unwrap_or(false);
    let later = if idle { " (a janela não está sendo desenhada: aplica quando ela voltar a aparecer)" } else { "" };
    let queue = |a: Action, ok: String| match tx.send(a) {
        Ok(()) => format!("ok: {ok}{later}"),
        Err(_) => "erro: o app está fechando".into(),
    };
    let current = || Settings::load().map(|(s, _)| s).map_err(|e| format!("erro: config.toml inválido: {e}"));
    let save = |layers: &[Layer], studies: bool, ok: String| match settings::save_chart(layers, studies) {
        Ok(()) => format!("ok: {ok}{later}"),
        Err(e) => format!("erro: não gravou o config.toml: {e}"),
    };
    match cmd {
        "help" => HELP.into(),
        "state" => shared.state.lock().map(|s| s.clone()).unwrap_or_else(|_| "{}".into()),
        "config" => settings::path().map(|p| p.display().to_string()).unwrap_or_default(),
        "symbol" if !arg.is_empty() => queue(Action::Symbol(arg.to_string()), arg.to_string()),
        "tf" => match Timeframe::parse(arg) {
            Some(tf) => queue(Action::Timeframe(tf), tf.label().to_string()),
            None => "erro: timeframe? use M1, M5, M15, M30, H1, H4, D1 ou W1".into(),
        },
        "layout" => match Grid::from_key(arg) {
            Some(g) => queue(Action::Layout(g), g.label().to_string()),
            None => "erro: layout? use 1, 2, 2v, 3, 4 ou 6".into(),
        },
        "chart" => match arg.parse::<usize>() {
            Ok(n) if n >= 1 => queue(Action::Chart(n), format!("gráfico {n} ativo")),
            _ => "erro: chart <N>, a partir de 1".into(),
        },
        "reload" => queue(Action::Reload, "relendo o config.toml".into()),
        "studies" if arg == "on" || arg == "off" => match current() {
            Ok(s) => save(&s.chart.layers, arg == "on", format!("studies {arg}")),
            Err(e) => e,
        },
        "layers" => {
            let parsed: Option<Vec<Layer>> = arg.split(',').map(|k| Layer::from_key(k.trim())).collect();
            match (parsed, current()) {
                (Some(l), Ok(s)) if l.len() == Layer::ALL.len() && Layer::ALL.iter().all(|x| l.contains(x)) => {
                    save(&l, s.chart.show_studies, arg.to_string())
                }
                (_, Err(e)) => e,
                _ => "erro: as quatro camadas, uma vez cada, de trás para a frente: levels,indicators,trades,price".into(),
            }
        }
        "front" => match (Layer::from_key(arg), current()) {
            (Some(l), Ok(s)) => {
                let mut layers = s.chart.layers;
                layers.retain(|x| *x != l);
                layers.push(l);
                save(&layers, s.chart.show_studies, format!("{arg} na frente"))
            }
            (_, Err(e)) => e,
            (None, _) => "erro: camada? levels, indicators, trades ou price".into(),
        },
        _ => format!("erro: comando {line:?}\n{HELP}"),
    }
}

/// The `ctl` client: send `args` as one line, print the reply. Exit code 1 for errors.
pub fn client(args: &[String]) -> i32 {
    let line = if args.is_empty() { "help".to_string() } else { args.join(" ") };
    let mut stream = match UnixStream::connect(socket_path()) {
        Ok(s) => s,
        Err(_) => {
            eprintln!("erro: o MT5 Terminal não está aberto ({})", socket_path().display());
            return 1;
        }
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    if stream.write_all(format!("{line}\n").as_bytes()).is_err() {
        eprintln!("erro: falha ao enviar o comando");
        return 1;
    }
    let mut reply = String::new();
    let _ = stream.read_to_string(&mut reply);
    let reply = reply.trim_end();
    if reply.starts_with("erro") {
        eprintln!("{reply}");
        1
    } else {
        println!("{reply}");
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_validate_and_queue() {
        let shared = Shared::new();
        let (tx, rx) = unbounded();
        assert!(execute("tf h1", &shared, &tx).starts_with("ok: H1"));
        assert_eq!(rx.try_recv(), Ok(Action::Timeframe(Timeframe::H1)));
        assert!(execute("tf X9", &shared, &tx).starts_with("erro"));
        assert!(execute("symbol UsaInd", &shared, &tx).starts_with("ok: UsaInd"));
        assert_eq!(rx.try_recv(), Ok(Action::Symbol("UsaInd".into())));
        assert!(execute("layout 2v", &shared, &tx).starts_with("ok: 2 empilhados"));
        assert_eq!(rx.try_recv(), Ok(Action::Layout(Grid::TwoStacked)));
        assert!(execute("layout 5", &shared, &tx).starts_with("erro"));
        assert!(execute("chart 2", &shared, &tx).starts_with("ok"));
        assert_eq!(rx.try_recv(), Ok(Action::Chart(2)));
        assert!(execute("chart 0", &shared, &tx).starts_with("erro"));
        assert!(execute("layers price,levels", &shared, &tx).starts_with("erro"));
        assert!(execute("front nada", &shared, &tx).starts_with("erro"));
        assert!(execute("comprar 1", &shared, &tx).starts_with("erro"), "ctl never trades");
        *shared.state.lock().unwrap() = "{\"symbol\":\"X\"}".into();
        assert_eq!(execute("state", &shared, &tx), "{\"symbol\":\"X\"}");
    }
}
