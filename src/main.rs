//! MT5 Terminal: our own chart front-end; MetaTrader 5 is only the bridge to the broker.
//!
//! `mt5-terminal [--synthetic] [--symbol S] [--tf TF] [--verify FILE]`
//! `mt5-terminal ctl <comando>`   commands to the running app (`ctl help`)
//! `mt5-terminal config <path|init|check|defaults>`

mod chart;
mod control;
mod feed;
mod history;
mod launcher;
mod model;
#[cfg(has_preset)]
#[path = "../preset/mod.rs"]
mod preset;
mod settings;
mod studies;
mod theme;
mod trading;

use chart::{ChartData, ChartView, Layer};
use history::{Loader, Need};
use settings::Settings;
use eframe::egui::{self, Align, Color32, Layout, RichText};
use feed::{Command, Event, Feed, Message};
use studies::Studies;
use model::{Series, Store, Timeframe};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use theme::Palette;
use trading::Trading;

/// How often the config file is checked for changes (a stat, nothing more).
const CONFIG_EVERY: Duration = Duration::from_millis(500);
/// Re-fetch the last bars of every series this often, to correct what the ticks built.
const RESYNC_EVERY: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, PartialEq)]
enum Source {
    Mt5,
    Synthetic,
}

struct Account {
    server: String,
    login: i64,
    kind: String,
}

struct App {
    pal: Palette,
    wake: egui::Context,
    bridge: Option<Feed>,
    bridge_error: Option<String>,
    synthetic: Option<Feed>,
    source: Source,
    connected: bool,
    account: Option<Account>,
    symbol: String,
    tf: Timeframe,
    store: Store,
    studies: Studies,
    settings: Settings,
    /// Modification time of config.toml when last read, and when it was last checked.
    config_mtime: Option<std::time::SystemTime>,
    last_config_check: Instant,
    /// `mt5-terminal ctl`: queued actions and the state it reports (only the primary instance listens).
    control: Option<(crossbeam_channel::Receiver<control::Action>, std::sync::Arc<control::Shared>)>,
    last_state: Instant,
    loader: Loader,
    /// Last (bid, tick time in ms) of every symbol.
    quotes: HashMap<String, (f64, i64)>,
    last_resync: Instant,
    #[cfg(has_preset)]
    verify: Option<preset::verify::Verify>,
    /// MetaTrader 5 was started by the app and hasn't connected yet.
    mt5_starting: bool,
    view: ChartView,
    /// Last tick server time (ms) and when it arrived, to extrapolate server time.
    last_tick: Option<(i64, Instant)>,
    status: Option<String>,
    trading: Trading,
}

impl App {
    fn new(cc: &eframe::CreationContext, args: Args) -> Self {
        let (settings, config_status) = match Settings::load() {
            Ok((s, warnings)) => (s, (!warnings.is_empty()).then(|| format!("config: {}", warnings.join("; ")))),
            Err(e) => (Settings::default(), Some(format!("config.toml inválido, usando o padrão: {e}"))),
        };
        let pal = settings.palette();
        theme::apply(&cc.egui_ctx, &pal);
        let symbol = args.symbol.clone().unwrap_or_else(|| settings.chart.symbol.clone());
        let tf = args.tf.unwrap_or(settings.chart.timeframe);
        let control = if args.primary {
            let (ctx, shared) = (cc.egui_ctx.clone(), control::Shared::new());
            control::serve(shared.clone(), move || ctx.request_repaint()).ok().map(|rx| (rx, shared))
        } else {
            None
        };
        let ctx = cc.egui_ctx.clone();
        let (bridge, bridge_error) = {
            let ctx = ctx.clone();
            match feed::bridge::spawn(feed::bridge::DEFAULT_ADDR, move || ctx.request_repaint()) {
                Ok(f) => (Some(f), None),
                Err(e) => (None, Some(format!("não abriu {}: {e}", feed::bridge::DEFAULT_ADDR))),
            }
        };
        let mut app = Self {
            pal,
            wake: ctx,
            bridge,
            bridge_error,
            synthetic: None,
            source: Source::Mt5,
            connected: false,
            account: None,
            studies: Studies::new(&symbol, tf),
            symbol,
            tf,
            store: Store::default(),
            settings,
            config_mtime: settings::mtime(),
            last_config_check: Instant::now(),
            control,
            last_state: Instant::now(),
            loader: Loader::default(),
            quotes: HashMap::new(),
            last_resync: Instant::now(),
            #[cfg(has_preset)]
            verify: args.verify.map(preset::verify::Verify::new),
            mt5_starting: false,
            view: ChartView::default(),
            last_tick: None,
            status: config_status,
            trading: Trading::default(),
        };
        if args.synthetic {
            app.set_source(Source::Synthetic);
        } else if app.settings.mt5.auto_start && !launcher::mt5_running() {
            let cmd = if app.settings.mt5.command.is_empty() { launcher::default_mt5_command() } else { app.settings.mt5.command.clone() };
            match launcher::start_mt5(&cmd) {
                Ok(()) => app.mt5_starting = true,
                Err(e) => app.status = Some(format!("não abriu o MetaTrader 5: {e}")),
            }
        }
        app
    }

    fn feed(&self) -> Option<&Feed> {
        match self.source {
            Source::Mt5 => self.bridge.as_ref(),
            Source::Synthetic => self.synthetic.as_ref(),
        }
    }

    fn set_source(&mut self, source: Source) {
        if source == Source::Synthetic && self.synthetic.is_none() {
            let ctx = self.wake.clone();
            self.synthetic = Some(feed::synthetic::spawn(move || ctx.request_repaint()));
        }
        self.source = source;
        self.connected = false;
        self.account = None;
        self.trading.reset();
        self.store.clear();
        self.loader.clear();
        self.quotes.clear();
        self.clear_chart();
        // a source that is already connected won't say so again
        if source == Source::Mt5 && self.bridge.is_some() {
            self.request();
        }
    }

    fn clear_chart(&mut self) {
        self.view.reset();
        self.last_tick = None;
        self.studies = Studies::new(&self.symbol, self.tf);
    }

    fn series(&self) -> Option<&Series> {
        self.store.get(&self.symbol, self.tf)
    }

    /// Ask the active source for the first chunk of what the chart and its indicators need.
    fn request(&mut self) {
        let needs = self.studies.needs();
        let mut symbols = vec![self.symbol.clone()];
        symbols.extend(needs.symbols.into_iter().filter(|s| *s != self.symbol));
        let mut cmds = vec![Command::Subscribe { symbols }];
        let (store, loader) = (&self.store, &mut self.loader);
        cmds.extend(loader.first(store, &self.symbol, self.tf, self.settings.chart.first_bars));
        for (symbol, tf, count) in needs.first {
            cmds.extend(loader.first(store, &symbol, tf, count));
        }
        self.send_all(cmds);
    }

    /// Older chunks for whatever still needs them: the view reaching the oldest bar, the indicators'
    /// warm-up and extra series. At most one request in flight per series.
    fn load_more(&mut self) {
        if !self.connected {
            return;
        }
        let (store, loader, sym, tf) = (&self.store, &mut self.loader, self.symbol.as_str(), self.tf);
        let mut cmds = Vec::new();
        if self.view.wants_older {
            cmds.extend(loader.older(store, sym, tf, Need::More));
        }
        if self.settings.chart.show_studies {
            cmds.extend(loader.older(store, sym, tf, Studies::warmup(tf)));
            for (symbol, tf, need) in self.studies.needs().older {
                cmds.extend(loader.older(store, &symbol, tf, need));
            }
        }
        self.send_all(cmds);
    }

    /// Re-fetch the last bars of every series (corrects volumes and closes built from ticks).
    fn resync(&mut self) {
        if self.last_resync.elapsed() < RESYNC_EVERY || !self.connected {
            return;
        }
        self.last_resync = Instant::now();
        if let Some(feed) = self.feed() {
            for (symbol, tf) in self.store.keys() {
                feed.send(Command::History { symbol: symbol.clone(), tf: *tf, count: 3, before: None });
            }
        }
    }

    fn select(&mut self, symbol: Option<&str>, tf: Option<Timeframe>) {
        let symbol = symbol.unwrap_or(&self.symbol).to_string();
        let tf = tf.unwrap_or(self.tf);
        if symbol == self.symbol && tf == self.tf {
            return;
        }
        if symbol != self.symbol {
            self.trading.symbol_changed();
        }
        self.symbol = symbol;
        self.tf = tf;
        self.clear_chart();
        self.request();
    }

    fn pump(&mut self) {
        // drain the inactive source too, so its queue never grows
        let inactive = match self.source {
            Source::Mt5 => self.synthetic.as_ref(),
            Source::Synthetic => self.bridge.as_ref(),
        };
        if let Some(f) = inactive {
            while f.events.try_recv().is_ok() {}
        }
        let events: Vec<Event> = match self.feed() {
            Some(f) => f.events.try_iter().collect(),
            None => return,
        };
        for ev in events {
            match ev {
                Event::Connected => {
                    self.connected = true;
                    self.mt5_starting = false;
                    self.status = None;
                    self.store.clear();
                    self.loader.clear();
                    self.clear_chart();
                    self.request();
                    #[cfg(has_preset)]
                    if let Some(v) = self.verify.as_mut() {
                        v.restart();
                    }
                }
                Event::Disconnected => {
                    self.connected = false;
                    self.account = None;
                    self.trading.reset();
                }
                Event::Message(msg) if self.trading.on_message(&msg, &self.symbol) => {
                    // a tick also feeds the chart
                    if let Message::Tick { symbol, time_msc, bid, volume, .. } = msg {
                        self.on_tick(&symbol, time_msc, bid, volume);
                    }
                }
                Event::Message(Message::Hello { server, login, account, version, netting, .. }) => {
                    self.connected = true;
                    self.trading.netting = netting;
                    if version < feed::BRIDGE_VERSION {
                        self.status = Some(format!(
                            "EA TerminalBridge desatualizado (v{version}, o app precisa da v{}): remova e anexe de novo no MT5",
                            feed::BRIDGE_VERSION
                        ));
                    }
                    self.account = Some(Account { server, login, kind: account });
                }
                Event::Message(Message::Bars { symbol, tf, digits, before, bars }) => {
                    self.loader.on_bars(&symbol, tf, before, bars.is_empty());
                    self.store.put(&symbol, tf, Message::decode_bars(&bars), digits);
                    self.studies.data_arrived();
                }
                #[cfg(has_preset)]
                Event::Message(msg @ (Message::Probe { .. } | Message::Objects { .. })) => {
                    if let Some(v) = self.verify.as_mut() {
                        let bars = self.store.bars(&self.symbol, self.tf);
                        v.on_message(&msg, bars, &self.studies);
                    }
                }
                Event::Message(Message::Tick { symbol, time_msc, bid, volume, .. }) => self.on_tick(&symbol, time_msc, bid, volume),
                Event::Message(Message::Error { msg }) => self.status = Some(msg),
                Event::Message(_) => {}
            }
        }
    }

    fn on_tick(&mut self, symbol: &str, time_msc: i64, bid: f64, volume: f64) {
        self.store.tick(symbol, time_msc.div_euclid(1000), bid, volume);
        self.quotes.insert(symbol.to_string(), (bid, time_msc));
        if symbol == self.symbol {
            self.last_tick = Some((time_msc, Instant::now()));
        }
    }

    /// Server time now, extrapolated from the last tick of the chart symbol.
    fn server_now(&self) -> Option<f64> {
        self.last_tick.map(|(ms, at)| ms as f64 / 1000.0 + at.elapsed().as_secs_f64())
    }

    fn is_real(&self) -> bool {
        self.account.as_ref().is_some_and(|a| a.kind == "real")
    }

    fn send_all(&self, cmds: Vec<Command>) {
        if let Some(feed) = self.feed() {
            for cmd in cmds {
                feed.send(cmd);
            }
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_centered(|ui| {
            dot(ui, self.pal.accent, 5.0);
            ui.label(RichText::new("MT5 Terminal").strong().color(self.pal.text).size(15.0));
            ui.separator();

            let mut symbol: Option<String> = None;
            egui::ComboBox::from_id_salt("symbol")
                .selected_text(RichText::new(&self.symbol).strong())
                .width(96.0)
                .show_ui(ui, |ui| {
                    for s in &self.settings.chart.symbols {
                        if ui.selectable_label(self.symbol == *s, s).clicked() {
                            symbol = Some(s.clone());
                        }
                    }
                });
            let mut tf = None;
            for t in Timeframe::ALL {
                if ui.selectable_label(self.tf == t, t.label()).clicked() {
                    tf = Some(t);
                }
            }
            if symbol.is_some() || tf.is_some() {
                self.select(symbol.as_deref(), tf);
            }

            ui.separator();
            let studies = ui
                .toggle_value(&mut self.settings.chart.show_studies, "Indicadores")
                .on_hover_text("Indicadores do preset (pasta preset/)");
            if studies.changed() {
                self.settings.save_ui();
            }
            ui.menu_button("Camadas", |ui| self.layers_menu(ui));

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let mut source = self.source;
                ui.selectable_value(&mut source, Source::Synthetic, "Sintético");
                ui.selectable_value(&mut source, Source::Mt5, "MT5");
                if source != self.source {
                    self.set_source(source);
                }
                ui.label(RichText::new("Fonte").color(self.pal.text_dim).size(12.0));
            });
        });
    }

    /// Drawing order of the plot, front first, with buttons to move each layer.
    fn layers_menu(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Na frente").color(self.pal.text_dim).size(11.5));
        let layers = &mut self.settings.chart.layers;
        let n = layers.len();
        let mut swap = None;
        // shown front first: the end of the back-to-front list
        for k in (0..n).rev() {
            ui.horizontal(|ui| {
                if ui.add_enabled(k + 1 < n, egui::Button::new("↑").small()).on_hover_text("Mais à frente").clicked() {
                    swap = Some((k, k + 1));
                }
                if ui.add_enabled(k > 0, egui::Button::new("↓").small()).on_hover_text("Mais atrás").clicked() {
                    swap = Some((k, k - 1));
                }
                ui.label(layers[k].label());
            });
        }
        ui.label(RichText::new("Atrás (o volume fica sempre no fundo)").color(self.pal.text_dim).size(11.5));
        ui.separator();
        let reset = ui.button("Padrão: preço na frente").clicked();
        if reset {
            *layers = Layer::DEFAULT.to_vec();
        }
        if let Some((a, b)) = swap {
            layers.swap(a, b);
        }
        if reset || swap.is_some() {
            self.settings.save_ui();
        }
    }

    /// Re-read config.toml when it changed (a stat every half second) and apply it.
    fn watch_config(&mut self) {
        if self.last_config_check.elapsed() < CONFIG_EVERY {
            return;
        }
        self.last_config_check = Instant::now();
        let m = settings::mtime();
        if m != self.config_mtime {
            self.config_mtime = m;
            self.reload_config();
        }
    }

    fn reload_config(&mut self) {
        match Settings::load() {
            Ok((s, warnings)) => {
                self.pal = s.palette();
                theme::apply(&self.wake, &self.pal);
                self.settings = s;
                self.status = (!warnings.is_empty()).then(|| format!("config: {}", warnings.join("; ")));
            }
            Err(e) => self.status = Some(format!("config.toml inválido (mantida a anterior): {e}")),
        }
    }

    /// Apply what `ctl` queued and publish the state it reports (a few times a second).
    fn serve_control(&mut self) {
        let Some((rx, shared)) = self.control.clone() else { return };
        if let Ok(mut t) = shared.last_frame.lock() {
            *t = Instant::now();
        }
        for action in rx.try_iter() {
            match action {
                control::Action::Symbol(s) => self.select(Some(&s), None),
                control::Action::Timeframe(tf) => self.select(None, Some(tf)),
                control::Action::Reload => {
                    self.config_mtime = settings::mtime();
                    self.reload_config();
                }
            }
        }
        if self.last_state.elapsed() >= Duration::from_millis(250) {
            self.last_state = Instant::now();
            if let Ok(mut st) = shared.state.lock() {
                *st = self.state_json();
            }
        }
    }

    fn state_json(&self) -> String {
        serde_json::json!({
            "symbol": self.symbol,
            "timeframe": self.tf.label(),
            "source": if self.source == Source::Mt5 { "mt5" } else { "synthetic" },
            "connected": self.connected,
            "account": self.account.as_ref().map(|a| serde_json::json!({"server": a.server, "login": a.login, "kind": a.kind})),
            "candles": self.series().map(|s| s.bars.len()).unwrap_or(0),
            "studies": self.settings.chart.show_studies,
            "layers": self.settings.chart.layers.iter().map(|l| l.key()).collect::<Vec<_>>(),
            "positions": self.trading.positions.len(),
            "orders": self.trading.orders.len(),
            "config": settings::path(),
            "status": self.status,
        })
        .to_string()
    }

    fn status_bar(&self, ui: &mut egui::Ui) {
        let pal = &self.pal;
        ui.horizontal_centered(|ui| {
            let (color, text) = match (self.source, self.connected, &self.bridge_error) {
                (Source::Mt5, _, Some(err)) => (pal.danger, err.clone()),
                (_, true, _) => (
                    pal.ok,
                    self.account.as_ref().map(|a| format!("{} · {}", a.server, a.login)).unwrap_or_else(|| "conectado".into()),
                ),
                (Source::Mt5, false, _) if self.mt5_starting => (pal.warn, "abrindo o MetaTrader 5… o EA conecta sozinho".into()),
                (Source::Mt5, false, _) => (pal.warn, format!("aguardando o EA TerminalBridge em {}", feed::bridge::DEFAULT_ADDR)),
                (Source::Synthetic, false, _) => (pal.warn, "iniciando…".into()),
            };
            dot(ui, color, 4.0);
            ui.label(RichText::new(text).color(pal.text_dim).size(12.0));
            if let Some(acc) = &self.account {
                let (bg, label) = match acc.kind.as_str() {
                    "real" => (pal.danger, "REAL"),
                    "contest" => (pal.warn, "CONCURSO"),
                    _ => (pal.tag_bg, "DEMO"),
                };
                egui::Frame::new().fill(bg).corner_radius(4).inner_margin(egui::Margin::symmetric(6, 0)).show(ui, |ui| {
                    ui.label(RichText::new(label).strong().size(10.5).color(Color32::WHITE));
                });
            }
            if let Some(s) = &self.status {
                ui.separator();
                ui.label(RichText::new(s).color(pal.warn).size(12.0));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let n = self.series().map(|s| s.bars.len()).unwrap_or(0);
                ui.label(RichText::new(format!("{n} candles")).color(pal.text_dim).size(12.0));
                if self.loader.loading() {
                    ui.label(RichText::new("carregando histórico…").color(pal.warn).size(12.0));
                }
            });
        });
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.watch_config();
        self.serve_control();
        self.pump();
        egui::Panel::top("top")
            .exact_size(40.0)
            .frame(egui::Frame::new().fill(self.pal.panel_bg).inner_margin(egui::Margin::symmetric(10, 0)))
            .show(ui, |ui| self.top_bar(ui));
        egui::Panel::bottom("status")
            .exact_size(24.0)
            .frame(egui::Frame::new().fill(self.pal.panel_bg).inner_margin(egui::Margin::symmetric(10, 0)))
            .show(ui, |ui| self.status_bar(ui));
        let (connected, real) = (self.connected, self.is_real());
        let cmds = egui::Panel::right("ticket")
            .exact_size(250.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(self.pal.panel_bg).inner_margin(egui::Margin::symmetric(12, 0)))
            .show(ui, |ui| self.trading.ticket_ui(ui, &self.pal, &self.symbol, connected, real))
            .inner;
        self.send_all(cmds);
        if !self.trading.positions.is_empty() || !self.trading.orders.is_empty() {
            let can_trade = self.trading.can_trade(connected, real);
            let cmds = egui::Panel::bottom("book")
                .resizable(true)
                .default_size(120.0)
                .frame(egui::Frame::new().fill(self.pal.panel_bg).inner_margin(egui::Margin::symmetric(10, 6)))
                .show(ui, |ui| self.trading.book_ui(ui, &self.pal, can_trade))
                .inner;
            self.send_all(cmds);
        }
        self.resync();
        let server_now = self.server_now();
        if !self.studies.is_for(&self.symbol, self.tf) {
            self.studies = Studies::new(&self.symbol, self.tf);
        }
        self.load_more();
        let warm = self.loader.ready(&self.store, &self.symbol, self.tf, Studies::warmup(self.tf));
        if self.settings.chart.show_studies {
            let digits = self.series().map(|s| s.digits).unwrap_or(2);
            let quotes = &self.quotes;
            self.studies.update(&self.store, digits, warm, server_now.map(|t| t as i64), |s| {
                quotes.get(s).map(|&(bid, ms)| (bid, ms.div_euclid(1000)))
            });
        }
        #[cfg(has_preset)]
        if let Some(v) = self.verify.as_mut() {
            let cmds = v.poll(warm, &self.symbol, self.tf);
            let done = v.done.then(|| v.path.clone());
            self.send_all(cmds);
            if let Some(path) = done {
                eprintln!("verify: relatório em {path}");
                self.verify = None;
            }
        }

        // trade lines drag and the context menu opens only when orders may go out (same locks as the ticket)
        let can_trade = self.trading.can_trade(self.connected, self.is_real());
        let positions = self.trading.levels(&self.symbol, &self.pal, can_trade);
        let show = self.settings.chart.show_studies;
        let (overlays, map_levels, pane) = if show {
            (self.studies.overlays(), self.studies.map_levels(), self.studies.pane())
        } else {
            (Vec::new(), Vec::new(), None)
        };
        let empty = Series::default();
        egui::CentralPanel::no_frame().show(ui, |ui| {
            let data = ChartData {
                series: self.store.get(&self.symbol, self.tf).unwrap_or(&empty),
                symbol: &self.symbol,
                tf: self.tf,
                server_now,
                levels: &positions,
                overlays: &overlays,
                map_levels: &map_levels,
                pane: pane.as_ref(),
                layers: &self.settings.chart.layers,
                quote: self.trading.quote.filter(|_| can_trade),
            };
            self.view.ui(ui, &data, &self.pal);
        });
        if can_trade {
            let actions = std::mem::take(&mut self.view.actions);
            let cmds: Vec<Command> = actions.into_iter().filter_map(|a| self.trading.chart_action(a, &self.symbol)).collect();
            self.send_all(cmds);
        }
    }
}

/// A small filled circle laid out like a widget (the default fonts lack bullet glyphs).
fn dot(ui: &mut egui::Ui, color: Color32, radius: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(radius * 2.0 + 2.0, radius * 2.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), radius, color);
}

struct Args {
    synthetic: bool,
    /// Over the config file's `chart.symbol` / `chart.timeframe`.
    symbol: Option<String>,
    tf: Option<Timeframe>,
    #[cfg_attr(not(has_preset), allow(dead_code))]
    verify: Option<String>,
    /// No other instance is running: this one serves `ctl`.
    primary: bool,
}

fn parse_args(raw: &[String]) -> Args {
    let mut args = Args { synthetic: false, symbol: None, tf: None, verify: None, primary: true };
    let mut it = raw.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--synthetic" => args.synthetic = true,
            "--verify" => args.verify = it.next().cloned(),
            "--symbol" => args.symbol = it.next().cloned(),
            "--tf" => args.tf = it.next().and_then(|v| Timeframe::parse(v)),
            _ => eprintln!("argumento ignorado: {a}"),
        }
    }
    args
}

/// `mt5-terminal config <path|init|check|defaults>`, for people and agents editing the file.
fn config_command(args: &[String]) -> i32 {
    let path = settings::path().map(|p| p.display().to_string()).unwrap_or_default();
    match args.first().map(String::as_str) {
        Some("path") | None => println!("{path}"),
        Some("defaults") => print!("{}", settings::DEFAULT_TOML),
        Some("init") => match Settings::init() {
            Ok(p) => println!("{}", p.display()),
            Err(e) => {
                eprintln!("erro: {e}");
                return 1;
            }
        },
        Some("check") => match Settings::load() {
            Ok((_, warnings)) if warnings.is_empty() => println!("ok: {path}"),
            Ok((_, warnings)) => {
                for w in warnings {
                    println!("aviso: {w}");
                }
            }
            Err(e) => {
                eprintln!("erro: {path}: {e}");
                return 1;
            }
        },
        Some(other) => {
            eprintln!("erro: config {other}? use path, init, check ou defaults");
            return 1;
        }
    }
    0
}

fn main() -> eframe::Result {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    match raw.first().map(String::as_str) {
        Some("ctl") => std::process::exit(control::client(&raw[1..])),
        Some("config") => std::process::exit(config_command(&raw[1..])),
        _ => {}
    }
    let mut args = parse_args(&raw);
    // one instance owns the EA's port: a second launch brings it forward instead (the synthetic
    // source needs no port, so it may run beside it, without taking over `ctl`)
    if launcher::other_instance() {
        if !args.synthetic {
            launcher::focus_other();
            return Ok(());
        }
        args.primary = false;
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("MT5 Terminal")
            .with_app_id("mt5-terminal")
            .with_inner_size([1440.0, 880.0])
            .with_min_inner_size([640.0, 400.0]),
        ..Default::default()
    };
    eframe::run_native("MT5 Terminal", options, Box::new(|cc| Ok(Box::new(App::new(cc, args)))))
}
