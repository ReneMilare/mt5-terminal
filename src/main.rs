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

use chart::{ChartData, ChartView, CursorMode, Layer};
use eframe::egui::{self, Align, Color32, Layout, RichText};
use feed::{Command, Event, Feed, Message};
use history::{Loader, Need};
use model::{Series, Store, Timeframe};
use settings::Settings;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use studies::Studies;
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
    /// `MT5_TERMINAL_PERF=1`: frame times on stderr every 5 s.
    perf: Option<Perf>,
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
    /// Newest chart bar seen, to finalize the delta of the bar that closed.
    delta_last_bar: i64,
    /// Closed bars of delta the studies want, and whether they were asked for (needs the POC row).
    delta_want: u32,
    delta_requested: bool,
    #[cfg(has_preset)]
    verify: Option<preset::verify::Verify>,
    /// MetaTrader 5 was started by the app and hasn't connected yet.
    mt5_starting: bool,
    /// The Colors window is open; colors changed and not yet written to config.toml.
    colors_open: bool,
    colors_dirty: bool,
    /// The presets editor is open; presets changed and not yet written.
    presets_open: bool,
    presets_dirty: bool,
    /// Account switch waiting for confirmation (switching to a real account asks first).
    confirm_switch: Option<settings::Account>,
    /// Account switch in progress: its reports, and the login it goes to.
    switching: Option<(crossbeam_channel::Receiver<String>, i64)>,
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
            perf: std::env::var_os("MT5_TERMINAL_PERF").map(|_| Perf::default()),
            pal,
            wake: ctx,
            bridge,
            bridge_error,
            synthetic: None,
            source: Source::Mt5,
            connected: false,
            account: None,
            studies: Studies::new(&symbol, tf, &settings.fibonacci),
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
            delta_last_bar: 0,
            delta_want: 0,
            delta_requested: false,
            #[cfg(has_preset)]
            verify: args.verify.map(preset::verify::Verify::new),
            mt5_starting: false,
            colors_open: false,
            colors_dirty: false,
            presets_open: false,
            presets_dirty: false,
            confirm_switch: None,
            switching: None,
            view: ChartView::default(),
            last_tick: None,
            status: config_status,
            trading: Trading::default(),
        };
        app.trading.set_presets(&app.settings.presets, &app.settings.ticket.preset);
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
        self.delta_last_bar = 0;
        self.last_tick = None;
        self.studies = Studies::new(&self.symbol, self.tf, &self.settings.fibonacci);
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
        // the delta goes out once the studies know the POC level height (it needs the bars)
        self.delta_want = needs.delta_bars;
        self.delta_requested = false;
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

    /// A chart bar closed: ask the EA for its exact delta (the live one is built from the streamed
    /// ticks, which may skip ticks of the same millisecond).
    fn finalize_delta(&mut self) {
        let Some(last) = self.series().and_then(|s| s.bars.last()).map(|b| b.time) else { return;
        };
        if last == self.delta_last_bar {
            return;
        }
        let first = self.delta_last_bar == 0;
        self.delta_last_bar = last;
        if !first && self.connected && self.store.deltas(&self.symbol, self.tf).is_some() {
            let row = self.store.deltas(&self.symbol, self.tf).map(|d| d.row).unwrap_or(0.0);
            let cmd = Command::Delta { symbol: self.symbol.clone(), tf: self.tf, count: 2, row };
            self.send_all(vec![cmd]);
        }
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
                    if let Message::Tick { symbol, time_msc, bid, ask, volume } = msg {
                        self.on_tick(&symbol, time_msc, bid, ask, volume);
                    }
                }
                Event::Message(Message::Hello { server, login, account, version, netting, .. }) => {
                    self.connected = true;
                    // the account switcher learns every account the MT5 connects to (never a password)
                    if self.source == Source::Mt5 && login != 0 {
                        let _ = settings::remember_account(login, &server, &account);
                    }
                    if let Some((_, target)) = &self.switching
                        && *target == login
                    {
                        self.switching = None;
                        self.status = Some(format!("conta trocada: {server} · {login}"));
                    }
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
                Event::Message(Message::Tick { symbol, time_msc, bid, ask, volume }) => {
                    if let Some(p) = &mut self.perf {
                        p.ticks += 1;
                    }
                    self.on_tick(&symbol, time_msc, bid, ask, volume)
                }
                Event::Message(Message::Delta { symbol, tf, bars }) => {
                    self.store.put_delta(&symbol, tf, &bars)
                }
                Event::Message(Message::Error { msg }) => self.status = Some(msg),
                Event::Message(_) => {}
            }
        }
    }

    fn on_tick(&mut self, symbol: &str, time_msc: i64, bid: f64, ask: f64, volume: f64) {
        self.store.tick(symbol, time_msc.div_euclid(1000), bid, volume);
        self.store.tick_quote(symbol, time_msc, bid, ask);
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
            let mut cursor = self.settings.chart.cursor;
            for mode in CursorMode::ALL {
                if mode.button(ui, cursor == mode).clicked() {
                    cursor = mode;
                }
            }
            if cursor != self.settings.chart.cursor {
                self.settings.chart.cursor = cursor;
                if let Err(e) = settings::save_cursor(cursor) {
                    self.status = Some(format!("não gravou o config.toml: {e}"));
                }
                self.config_mtime = settings::mtime();
            }
            ui.separator();
            let studies = ui
                .toggle_value(&mut self.settings.chart.show_studies, "Indicadores")
                .on_hover_text("Indicadores do preset (pasta preset/)");
            if studies.changed() {
                self.settings.save_ui();
            }
            ui.menu_button("Camadas", |ui| self.layers_menu(ui));
            if ui.selectable_label(self.colors_open, "Cores").on_hover_text("Cores do app e dos candles").clicked() {
                self.colors_open = !self.colors_open;
            }

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
                if ui.add_enabled(k + 1 < n, egui::Button::new("frente").small()).on_hover_text("Uma camada mais à frente").clicked() {
                    swap = Some((k, k + 1));
                }
                if ui.add_enabled(k > 0, egui::Button::new("trás").small()).on_hover_text("Uma camada mais atrás").clicked() {
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
                self.trading.set_presets(&s.presets, &s.ticket.preset);
                let fibonacci_changed = self.studies.configure_fibonacci(&s.fibonacci);
                self.settings = s;
                if fibonacci_changed && self.connected {
                    self.request();
                }
                self.status = (!warnings.is_empty()).then(|| format!("config: {}", warnings.join("; ")));
            }
            Err(e) => self.status = Some(format!("config.toml inválido (mantida a anterior): {e}")),
        }
    }

    /// Apply what `ctl` queued and publish the state it reports (a few times a second).
    fn serve_control(&mut self) {
        let Some((rx, shared)) = self.control.clone() else { return;
        };
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
            "cursor": self.settings.chart.cursor.key(),
            "fibonacci": self.studies.fibonacci_state(),
            "positions": self.trading.positions.len(),
            "orders": self.trading.orders.len(),
            "config": settings::path(),
            "status": self.status,
        })
        .to_string()
    }

    /// Restart MT5 into `account` (confirmed already when it is real).
    fn switch_account(&mut self, account: settings::Account) {
        if self.switching.is_some() || self.source != Source::Mt5 {
            return;
        }
        let cmd = if self.settings.mt5.command.is_empty() { launcher::default_mt5_command() } else { self.settings.mt5.command.clone() };
        let (tx, rx) = crossbeam_channel::unbounded();
        launcher::switch_account(cmd, account.login, account.server.clone(), tx);
        self.trading.reset();
        self.status = Some(format!("trocando para {}…", account.name));
        self.switching = Some((rx, account.login));
    }

    /// Like MT5's chart properties, Colors tab: every color, ready-made schemes, applied live and
    /// written to config.toml once the mouse is released (not on every step of a drag in the picker).
    fn colors_window(&mut self, ctx: &egui::Context) {
        if !self.colors_open {
            return;
        }
        let mut open = true;
        let mut changed = false;
        egui::Window::new("Cores").open(&mut open).resizable(false).collapsible(false).show(ctx, |ui| {
            ui.label(RichText::new("Predefinições").color(self.pal.text_dim).size(12.0));
            ui.horizontal_wrapped(|ui| {
                for (name, colors) in settings::presets() {
                    if ui.button(name).clicked() {
                        self.settings.colors = colors;
                        changed = true;
                    }
                }
            });
            ui.separator();
            egui::Grid::new("colors").num_columns(2).spacing([16.0, 6.0]).show(ui, |ui| {
                for (key, label) in settings::COLOR_KEYS {
                    if key == "candle_up" {
                        ui.label(RichText::new("Candles").strong());
                        ui.end_row();
                    }
                    ui.label(label);
                    let c = self.settings.colors.color(key);
                    let mut rgb = [c.r(), c.g(), c.b()];
                    if egui::color_picker::color_edit_button_srgb(ui, &mut rgb).changed() {
                        self.settings.colors.set(key, settings::color_hex(Color32::from_rgb(rgb[0], rgb[1], rgb[2])));
                        changed = true;
                    }
                    ui.end_row();
                }
            });
            ui.add_space(4.0);
            ui.label(RichText::new("Corpo igual ao fundo e contorno colorido = candle vazado, como no MT5.").color(self.pal.text_dim).size(11.5));
            ui.label(RichText::new("Gravadas no config.toml, seção [colors].").color(self.pal.text_dim).size(11.5));
        });
        if changed {
            self.pal = self.settings.palette();
            theme::apply(ctx, &self.pal);
            self.colors_dirty = true;
        }
        if self.colors_dirty && !ctx.input(|i| i.pointer.any_down()) {
            if let Err(e) = settings::save_colors(&self.settings.colors) {
                self.status = Some(format!("não gravou as cores: {e}"));
            }
            self.config_mtime = settings::mtime();
            self.colors_dirty = false;
        }
        if !open {
            self.colors_open = false;
        }
    }

    fn save_ui_state(&mut self) {
        if let Err(e) = settings::save_ui_state(&self.settings.ui) {
            self.status = Some(format!("não gravou o config.toml: {e}"));
        }
        self.config_mtime = settings::mtime();
    }

    /// Remember the preset chosen in the ticket, and run the presets editor.
    fn presets_ui(&mut self, ctx: &egui::Context) {
        if self.trading.active_name() != self.settings.ticket.preset {
            self.settings.ticket.preset = self.trading.active_name().to_string();
            self.presets_dirty = true;
        }
        if std::mem::take(&mut self.trading.edit_presets) {
            self.presets_open = true;
        }
        if self.presets_open {
            let mut open = true;
            let mut changed = false;
            let mut remove = None;
            let mut use_it = None;
            egui::Window::new("Operações predefinidas").open(&mut open).resizable(false).collapsible(false).show(ctx, |ui| {
                ui.label(
                    RichText::new("Volume, stop e alvo prontos. Com Shift (compra) ou Ctrl (venda) no gráfico, a operação ativa acompanha o ponteiro.")
                        .color(self.pal.text_dim)
                        .size(11.5),
                );
                ui.add_space(4.0);
                egui::Grid::new("presets").num_columns(7).spacing([10.0, 6.0]).show(ui, |ui| {
                    for h in ["Nome", "Volume", "Stop", "Alvo", "Unidade", "", ""] {
                        ui.label(RichText::new(h).color(self.pal.text_dim).size(11.5));
                    }
                    ui.end_row();
                    let active = self.trading.preset;
                    for (i, p) in self.settings.presets.iter_mut().enumerate() {
                        changed |= ui.add_sized([170.0, 20.0], egui::TextEdit::singleline(&mut p.name)).changed();
                        changed |= ui.add(egui::DragValue::new(&mut p.volume).speed(0.01).range(0.01..=1000.0).max_decimals(2)).changed();
                        let pct = p.unit == "percent";
                        let speed = if pct { 0.01 } else { 1.0 };
                        changed |= ui.add(egui::DragValue::new(&mut p.stop).speed(speed).range(0.0..=f64::MAX).max_decimals(2)).changed();
                        changed |= ui.add(egui::DragValue::new(&mut p.target).speed(speed).range(0.0..=f64::MAX).max_decimals(2)).changed();
                        egui::ComboBox::from_id_salt(("unit", i)).selected_text(if pct { "%" } else { "pontos" }).width(70.0).show_ui(ui, |ui| {
                            for (u, label) in [("percent", "%"), ("points", "pontos")] {
                                if ui.selectable_label(p.unit == u, label).clicked() && p.unit != u {
                                    p.unit = u.into();
                                    changed = true;
                                }
                            }
                        });
                        if ui.add_enabled(active != Some(i), egui::Button::new(if active == Some(i) { "ativa" } else { "usar" }).small()).clicked() {
                            use_it = Some(i);
                        }
                        if ui.small_button("remover").clicked() {
                            remove = Some(i);
                        }
                        ui.end_row();
                    }
                });
                ui.add_space(4.0);
                if ui.button("+ Nova operação").clicked() {
                    let n = self.settings.presets.len() + 1;
                    let volume = self.trading.order_volume();
                    self.settings.presets.push(settings::Preset {
                        name: format!("Operação {n}"),
                        volume,
                        stop: 0.2,
                        target: 0.4,
                        unit: "percent".into(),
                    });
                    changed = true;
                }
                ui.label(RichText::new("Gravadas no config.toml ([[presets]]).").color(self.pal.text_dim).size(11.5));
            });
            let active = self.trading.active_name().to_string();
            if let Some(i) = remove {
                self.settings.presets.remove(i);
                changed = true;
            }
            if changed {
                // names may have changed: keep the active one by position when its name was edited
                let keep = self.trading.preset.filter(|&i| remove != Some(i)).map(|i| {
                    if remove.is_some_and(|r| r < i) { i - 1 } else { i
                    }
                });
                let name = keep.and_then(|i| self.settings.presets.get(i)).map(|p| p.name.clone()).unwrap_or(active);
                self.trading.set_presets(&self.settings.presets, &name);
                self.presets_dirty = true;
            }
            if let Some(i) = use_it {
                self.trading.preset = Some(i);
            }
            if !open {
                self.presets_open = false;
            }
        }
        if self.presets_dirty && !ctx.input(|i| i.pointer.any_down()) && !ctx.egui_wants_keyboard_input() {
            self.settings.ticket.preset = self.trading.active_name().to_string();
            if let Err(e) = settings::save_presets(&self.settings.presets, &self.settings.ticket.preset) {
                self.status = Some(format!("não gravou as operações: {e}"));
            }
            self.config_mtime = settings::mtime();
            self.presets_dirty = false;
        }
    }

    /// Progress of an account switch, and the confirmation before a real account.
    fn account_switch_ui(&mut self, ctx: &egui::Context) {
        if let Some((rx, _)) = &self.switching {
            let msgs: Vec<String> = rx.try_iter().collect();
            if let Some(last) = msgs.last() {
                self.status = Some(last.clone());
                if last.starts_with("erro") {
                    self.switching = None;
                }
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
        let Some(target) = self.confirm_switch.clone() else { return;
        };
        let mut decision = None;
        let modal = egui::Modal::new(egui::Id::new("confirm-real")).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.label(RichText::new("Trocar para a conta REAL?").strong().size(16.0).color(self.pal.danger));
            ui.add_space(6.0);
            ui.label(format!("{} · {} · {}", target.name, target.server, target.login));
            ui.label("O MetaTrader 5 fecha e abre de novo nessa conta. As ordens continuam travadas até você armar a conta REAL na boleta.");
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button(RichText::new("Trocar para REAL").strong().color(self.pal.danger)).clicked() {
                    decision = Some(true);
                }
                if ui.button("Cancelar").clicked() {
                    decision = Some(false);
                }
            });
        });
        if modal.should_close() && decision.is_none() {
            decision = Some(false);
        }
        if let Some(go) = decision {
            self.confirm_switch = None;
            if go {
                self.switch_account(target);
            }
        }
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let pal = &self.pal;
        let mut chosen: Option<settings::Account> = None;
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
            // the account label opens the switcher (accounts the MT5 has connected to)
            let current = self.account.as_ref().map(|a| a.login);
            if self.source == Source::Mt5 && !self.settings.accounts.is_empty() && self.switching.is_none() {
                ui.menu_button(RichText::new(format!("{text}  · trocar conta")).color(pal.text_dim).size(12.0), |ui| {
                    ui.label(RichText::new("Trocar de conta (reinicia o MetaTrader 5)").color(pal.text_dim).size(11.5));
                    for a in &self.settings.accounts {
                        let label = format!("{} · {}{}", a.name, a.server, if a.is_real() { "  · REAL" } else { "" });
                        let here = current == Some(a.login);
                        if ui.add_enabled(!here, egui::Button::selectable(here, label)).clicked() {
                            chosen = Some(a.clone());
                            ui.close();
                        }
                    }
                });
            } else {
                ui.label(RichText::new(text).color(pal.text_dim).size(12.0));
            }
            if let Some(acc) = &self.account {
                let (bg, label) = match acc.kind.as_str() {
                    "real" => (pal.danger, "REAL"),
                    "contest" => (pal.warn, "CONCURSO"),
                    _ => (pal.tag_bg, "DEMO"),
                };
                // white on the red/yellow badges, the text color on the neutral one (light themes)
                let fg = if acc.kind == "real" || acc.kind == "contest" { Color32::WHITE } else { pal.text };
                egui::Frame::new().fill(bg).corner_radius(4).inner_margin(egui::Margin::symmetric(6, 0)).show(ui, |ui| {
                    ui.label(RichText::new(label).strong().size(10.5).color(fg));
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
        match chosen {
            Some(a) if a.is_real() => self.confirm_switch = Some(a),
            Some(a) => self.switch_account(a),
            None => {}
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let started = self.perf.is_some().then(std::time::Instant::now);
        self.frame(ui);
        if let (Some(p), Some(t)) = (&mut self.perf, started) {
            p.frame(t.elapsed(), frame.info().cpu_usage);
        }
    }
}

/// Frame-time diagnostics (only with `MT5_TERMINAL_PERF=1`).
#[derive(Default)]
struct Perf {
    since: Option<std::time::Instant>,
    /// `ui()` time of each frame in the window, µs.
    ui_us: Vec<u32>,
    /// Whole CPU time of the previous frames (ui + tessellation + paint submit), µs.
    cpu_us: Vec<u32>,
    ticks: u32,
}

impl Perf {
    fn frame(&mut self, ui: std::time::Duration, cpu: Option<f32>) {
        let since = *self.since.get_or_insert_with(std::time::Instant::now);
        self.ui_us.push(ui.as_micros() as u32);
        if let Some(c) = cpu {
            self.cpu_us.push((c * 1e6) as u32);
        }
        let span = since.elapsed().as_secs_f64();
        if span < 5.0 {
            return;
        }
        let stats = |v: &mut Vec<u32>| {
            v.sort_unstable();
            let n = v.len().max(1);
            let avg = v.iter().map(|&x| x as u64).sum::<u64>() / n as u64;
            let pct = |q: f64| {
                v.get(((n as f64 * q) as usize).min(n - 1)).copied().unwrap_or(0)
            };
            format!("méd {avg} µs · p99 {} µs · máx {} µs", pct(0.99), v.last().copied().unwrap_or(0))
        };
        eprintln!(
            "perf: {:.1} quadros/s · {:.1} ticks/s · ui {} · cpu {}",
            self.ui_us.len() as f64 / span,
            self.ticks as f64 / span,
            stats(&mut self.ui_us),
            stats(&mut self.cpu_us)
        );
        *self = Perf::default();
    }
}

impl App {
    fn frame(&mut self, ui: &mut egui::Ui) {
        self.watch_config();
        self.serve_control();
        let ctx = ui.ctx().clone();
        self.account_switch_ui(&ctx);
        self.colors_window(&ctx);
        self.presets_ui(&ctx);
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
        let keys = self.trading.shortcuts(ui.ctx(), &self.symbol, connected, real);
        self.send_all(keys);
        // the ticket, or a thin strip when collapsed
        let open = self.settings.ui.ticket_open;
        let mut toggle = false;
        let cmds = egui::Panel::right("ticket")
            .exact_size(if open { 250.0 } else { 26.0 })
            .resizable(false)
            .frame(egui::Frame::new().fill(self.pal.panel_bg).inner_margin(egui::Margin::symmetric(if open { 12 } else { 2 }, 0)))
            .show(ui, |ui| {
                if open {
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Boleta").color(self.pal.text_dim).size(12.0));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            toggle = ui.small_button("»").on_hover_text("Recolher a boleta").clicked();
                        });
                    });
                    self.trading.ticket_ui(ui, &self.pal, &self.symbol, connected, real)
                } else {
                    ui.add_space(6.0);
                    toggle = ui.add_sized([22.0, 40.0], egui::Button::new("«")).on_hover_text("Abrir a boleta").clicked();
                    Vec::new()
                }
            })
            .inner;
        if toggle {
            self.settings.ui.ticket_open = !open;
            self.save_ui_state();
        }
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
            self.studies = Studies::new(&self.symbol, self.tf, &self.settings.fibonacci);
        }
        self.load_more();
        self.finalize_delta();
        let warm = self.loader.ready(&self.store, &self.symbol, self.tf, Studies::warmup(self.tf));
        if self.settings.chart.show_studies {
            let digits = self.series().map(|s| s.digits).unwrap_or(2);
            let quotes = &self.quotes;
            let tick = self.trading.tick_size();
            self.studies.update(&self.store, digits, tick, warm, server_now.map(|t| t as i64), |s| quotes.get(s).map(|&(bid, ms)| (bid, ms.div_euclid(1000))),
            );
        }
        if self.delta_want > 0 && !self.delta_requested && self.connected
            && let Some(row) = self.studies.delta_row()
        {
            self.delta_requested = true;
            self.store.track_delta(&self.symbol, self.tf, row);
            let cmd = Command::Delta { symbol: self.symbol.clone(), tf: self.tf, count: self.delta_want, row };
            self.send_all(vec![cmd]);
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
        let (overlays, map_levels, pane, volume, marks) = if show {
            (self.studies.overlays(), self.studies.map_levels(), self.studies.pane(), self.studies.volume(), self.studies.marks())
        } else {
            (Vec::new(), Vec::new(), None, None, None)
        };
        let volume_text = self.trading.volume_text();
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
                pane_open: self.settings.ui.pane_open,
                layers: &self.settings.chart.layers,
                quote: self.trading.quote,
                can_trade,
                cursor: self.settings.chart.cursor,
                tick: self.trading.tick_size(),
                order_volume: &volume_text,
                bracket: Some(self.trading.bracket()),
                volume,
                marks,
            };
            self.view.ui(ui, &data, &self.pal);
        });
        let actions = std::mem::take(&mut self.view.actions);
        let mut cmds: Vec<Command> = Vec::new();
        for a in actions {
            match a {
                chart::ChartAction::TogglePane => {
                    self.settings.ui.pane_open = !self.settings.ui.pane_open;
                    self.save_ui_state();
                }
                a if can_trade => cmds.extend(self.trading.chart_action(a, &self.symbol)),
                _ => {}
            }
        }
        self.send_all(cmds);
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
