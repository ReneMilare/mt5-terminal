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
mod navigation;
mod panes;
#[cfg(has_preset)]
#[path = "../preset/mod.rs"]
mod preset;
mod settings;
mod studies;
mod theme;
mod trading;

use chart::{ChartData, CursorMode, Layer};
use eframe::egui::{self, Align, Color32, Layout, RichText};
use feed::{Command, Event, Feed, Message};
use history::{Loader, Need};
use model::{Series, Store, Timeframe};
use panes::{DeltaAsked, Pane};
use settings::{Grid, Settings};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use studies::Studies;
use theme::Palette;
use trading::Trading;

/// How often the config file is checked for changes (a stat, nothing more).
const CONFIG_EVERY: Duration = Duration::from_millis(500);
/// Re-fetch the last bars of every series this often, to correct what the ticks built.
const RESYNC_EVERY: Duration = Duration::from_secs(60);
/// The pointer resting this long over a chart makes it the active one (just passing over it, on the
/// way to the ticket, doesn't switch the ticket's symbol).
const HOVER_FOCUS: Duration = Duration::from_millis(150);

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
    /// Charts side by side (`chart.layout`), and the one the top bar, the ticket and the shortcuts use.
    panes: Vec<Pane>,
    active: usize,
    /// Chart under the resting pointer and since when (it becomes active after `HOVER_FOCUS`).
    hover_since: Option<(usize, Instant)>,
    store: Store,
    settings: Settings,
    /// Modification time of config.toml when last read, and when it was last checked.
    config_mtime: Option<std::time::SystemTime>,
    last_config_check: Instant,
    /// `mt5-terminal ctl`: queued actions and the state it reports (only the primary instance listens).
    control: Option<(crossbeam_channel::Receiver<control::Action>, std::sync::Arc<control::Shared>)>,
    last_state: Instant,
    loader: Loader,
    /// Last (bid, ask, tick time in ms) of every symbol.
    quotes: HashMap<String, (f64, f64, i64)>,
    last_resync: Instant,
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
    /// Indicator whose options are open (right click on it), and the text of its Fibonacci levels.
    study_open: Option<&'static str>,
    study_dirty: bool,
    fib_levels: String,
    /// Account switch waiting for confirmation (switching to a real account asks first).
    confirm_switch: Option<settings::Account>,
    /// Account switch in progress: its reports, and the login it goes to.
    switching: Option<(crossbeam_channel::Receiver<String>, i64)>,
    /// Newest tick server time (ms, any symbol) and when it arrived, to extrapolate server time.
    last_tick: Option<(i64, Instant)>,
    status: Option<String>,
    trading: Trading,
}

impl App {
    fn new(cc: &eframe::CreationContext, mut args: Args) -> Self {
        let (settings, config_status) = match Settings::load() {
            Ok((s, warnings)) => (s, (!warnings.is_empty()).then(|| format!("config: {}", warnings.join("; ")))),
            Err(e) => (Settings::default(), Some(format!("config.toml inválido, usando o padrão: {e}"))),
        };
        let pal = settings.palette();
        theme::apply(&cc.egui_ctx, &pal);
        let mut panes: Vec<Pane> = Vec::new();
        for i in 0..settings.chart.layout.count() {
            let open: Vec<&str> = panes.iter().map(|p| p.symbol.as_str()).collect();
            let (symbol, tf) = panes::configured(&settings, i, &open);
            let (symbol, tf) = if i == 0 { (args.symbol.clone().unwrap_or(symbol), args.tf.unwrap_or(tf)) } else { (symbol, tf) };
            panes.push(Pane::new(symbol, tf, &settings));
        }
        let control = if args.primary {
            let (ctx, shared) = (cc.egui_ctx.clone(), control::Shared::new());
            control::serve(shared.clone(), move || ctx.request_repaint()).ok().map(|rx| (rx, shared))
        } else {
            None
        };
        let ctx = cc.egui_ctx.clone();
        // the port opened before the window (`main`): from now on its events wake the frames
        let _ = args.wake.set(ctx.clone());
        let (bridge, bridge_error) = match args.bridge.take() {
            Some(Ok(f)) => (Some(f), None),
            Some(Err(e)) => (None, Some(e)),
            None => (None, None),
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
            panes,
            active: 0,
            hover_since: None,
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
            colors_open: false,
            colors_dirty: false,
            presets_open: false,
            presets_dirty: false,
            study_open: None,
            study_dirty: false,
            fib_levels: String::new(),
            confirm_switch: None,
            switching: None,
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
        self.clear_charts();
        // a source that is already connected won't say so again
        if source == Source::Mt5 && self.bridge.is_some() {
            self.request(0..self.panes.len());
        }
    }

    fn clear_charts(&mut self) {
        for p in &mut self.panes {
            p.clear(&self.settings);
        }
        self.last_tick = None;
    }

    fn pane(&self) -> &Pane {
        &self.panes[self.active]
    }

    /// Symbol of the active chart (the ticket's).
    fn symbol(&self) -> &str {
        &self.pane().symbol
    }

    fn series(&self) -> Option<&Series> {
        self.store.get(&self.pane().symbol, self.pane().tf)
    }

    fn quote(&self, symbol: &str) -> Option<(f64, f64)> {
        self.quotes.get(symbol).map(|&(bid, ask, _)| (bid, ask))
    }

    /// Ask the active source for the first chunk of what every chart and its indicators need (the
    /// loader skips what is loaded or loading). `renew`: charts whose delta is asked for again.
    fn request(&mut self, renew: std::ops::Range<usize>) {
        let mut symbols: Vec<String> = Vec::new();
        let mut add = |s: &str| {
            if !symbols.iter().any(|x| x == s) {
                symbols.push(s.to_string());
            }
        };
        for p in &self.panes {
            add(&p.symbol);
        }
        let mut first = Vec::new();
        for (i, p) in self.panes.iter_mut().enumerate() {
            let needs = p.studies.needs();
            for s in &needs.symbols {
                add(s);
            }
            // the delta goes out once the studies know the POC level height (it needs the bars)
            if renew.contains(&i) {
                p.delta_want = needs.delta_bars;
                p.delta = DeltaAsked::Nothing;
            }
            first.extend(needs.first);
        }
        let mut cmds = vec![Command::Subscribe { symbols }];
        let (store, loader) = (&self.store, &mut self.loader);
        for p in &self.panes {
            cmds.extend(loader.first(store, &p.symbol, p.tf, self.settings.chart.first_bars));
        }
        for (symbol, tf, count) in first {
            cmds.extend(loader.first(store, &symbol, tf, count));
        }
        self.send_all(cmds);
    }

    /// Older chunks for whatever still needs them: a view reaching the oldest bar, the indicators'
    /// warm-up and extra series. At most one request in flight per series.
    fn load_more(&mut self) {
        if !self.connected {
            return;
        }
        let (store, loader) = (&self.store, &mut self.loader);
        let mut cmds = Vec::new();
        for p in &self.panes {
            let (sym, tf) = (p.symbol.as_str(), p.tf);
            if let Some(time) = p.navigation.target {
                cmds.extend(loader.older(store, sym, tf, Need::At { time }));
            }
            if p.view.wants_older {
                cmds.extend(loader.older(store, sym, tf, Need::More));
            }
            if self.settings.chart.show_studies {
                cmds.extend(loader.older(store, sym, tf, Studies::warmup(tf)));
                for (symbol, tf, need) in p.studies.needs().older {
                    cmds.extend(loader.older(store, &symbol, tf, need));
                }
            }
        }
        self.send_all(cmds);
    }

    /// A chart bar closed: ask the EA for its exact delta (the live one is built from the streamed
    /// ticks, which may skip ticks of the same millisecond).
    fn finalize_delta(&mut self) {
        let mut cmds = Vec::new();
        for p in &mut self.panes {
            let Some(last) = self.store.bars(&p.symbol, p.tf).last().map(|b| b.time) else { continue };
            if last == p.delta_last_bar {
                continue;
            }
            let first = p.delta_last_bar == 0;
            p.delta_last_bar = last;
            if !first && self.connected && let Some(d) = self.store.deltas(&p.symbol, p.tf) {
                cmds.push(Command::Delta { symbol: p.symbol.clone(), tf: p.tf, count: 2, row: d.row, skip: 0 });
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

    /// New symbol and/or timeframe for the active chart.
    fn select(&mut self, symbol: Option<&str>, tf: Option<Timeframe>) {
        self.select_in(self.active, symbol, tf, true);
    }

    /// New symbol and/or timeframe for chart `i`; `save`: write it to config.toml (a choice in the app).
    fn select_in(&mut self, i: usize, symbol: Option<&str>, tf: Option<Timeframe>, save: bool) {
        let p = &self.panes[i];
        let symbol = symbol.unwrap_or(&p.symbol).to_string();
        let tf = tf.unwrap_or(p.tf);
        if symbol == p.symbol && tf == p.tf {
            return;
        }
        let symbol_changed = symbol != p.symbol;
        let p = &mut self.panes[i];
        p.symbol = symbol;
        p.tf = tf;
        p.clear(&self.settings);
        if symbol_changed && i == self.active {
            let symbol = self.symbol().to_string();
            self.trading.symbol_changed(&symbol, self.quote(&symbol));
        }
        self.request(i..i + 1);
        if save {
            self.save_charts();
        }
    }

    /// Make chart `i` the one the top bar, the date bar, the ticket and the shortcuts use.
    fn activate(&mut self, i: usize) {
        if i == self.active || i >= self.panes.len() {
            return;
        }
        let before = self.symbol().to_string();
        self.active = i;
        if self.symbol() != before {
            let symbol = self.symbol().to_string();
            self.trading.symbol_changed(&symbol, self.quote(&symbol));
        }
        // the scale checkbox shows this chart's mode; not a choice to write down
        self.settings.chart.auto_scale = self.pane().view.auto_scale();
    }

    /// Show `grid`: charts beyond it close (their symbols stay in `chart.charts` for later), new ones
    /// open with what the config says.
    fn set_layout(&mut self, grid: Grid, save: bool) {
        let n = grid.count();
        let before = self.panes.len();
        if n < before {
            // remember the closing charts' symbols before they go
            self.settings.chart.charts = self.charts_entries();
            if self.active >= n {
                self.activate(0);
            }
            self.panes.truncate(n);
        }
        for i in before..n {
            let open: Vec<&str> = self.panes.iter().map(|p| p.symbol.as_str()).collect();
            let (symbol, tf) = panes::configured(&self.settings, i, &open);
            self.panes.push(Pane::new(symbol, tf, &self.settings));
        }
        self.settings.chart.layout = grid;
        if n > before && self.connected {
            self.request(before..n);
        }
        if save {
            self.save_charts();
        }
    }

    /// `chart.charts` as the open charts set it, keeping entries of charts not open now.
    fn charts_entries(&self) -> Vec<String> {
        let mut out: Vec<String> = self.panes.iter().skip(1).map(Pane::entry).collect();
        out.extend(self.settings.chart.charts.iter().skip(out.len()).cloned());
        out
    }

    fn save_charts(&mut self) {
        let charts = self.charts_entries();
        let first = &self.panes[0];
        self.settings.chart.symbol = first.symbol.clone();
        self.settings.chart.timeframe = first.tf;
        if !self.settings.chart.symbols.contains(&first.symbol) {
            self.settings.chart.symbols.insert(0, first.symbol.clone());
        }
        self.settings.chart.charts = charts;
        let c = &self.settings.chart;
        if let Err(e) = settings::save_charts(c.layout, &c.symbol, c.timeframe, &c.charts) {
            self.status = Some(format!("não gravou os gráficos no config.toml: {e}"));
        }
        self.config_mtime = settings::mtime();
    }

    /// The charts the config asks for, where they differ from what is open (config edited outside).
    fn apply_charts(&mut self) {
        let grid = self.settings.chart.layout;
        if grid.count() != self.panes.len() {
            self.set_layout(grid, false);
        }
        for i in 0..self.panes.len() {
            // a chart without an entry keeps what it shows
            if i > 0 && self.settings.chart.charts.get(i - 1).is_none() {
                continue;
            }
            let (symbol, tf) = panes::configured(&self.settings, i, &[]);
            self.select_in(i, Some(&symbol), Some(tf), false);
        }
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
                    self.trading.reset();
                    self.mt5_starting = false;
                    self.status = None;
                    self.store.clear();
                    self.loader.clear();
                    let dates: Vec<Option<i64>> = self.panes.iter().map(|p| p.navigation.target).collect();
                    self.clear_charts();
                    for (p, date) in self.panes.iter_mut().zip(dates) {
                        p.navigation.target = date;
                    }
                    self.request(0..self.panes.len());
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
                Event::Message(msg) if self.trading.on_message(&msg, &self.panes[self.active].symbol) => {
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
                    for p in &mut self.panes {
                        p.studies.data_arrived();
                    }
                }
                #[cfg(has_preset)]
                Event::Message(msg @ (Message::Probe { .. } | Message::Objects { .. })) => {
                    if let Some(v) = self.verify.as_mut() {
                        let p = &self.panes[self.active];
                        v.on_message(&msg, self.store.bars(&p.symbol, p.tf), &p.studies);
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
        self.quotes.insert(symbol.to_string(), (bid, ask, time_msc));
        // server time is the same for every symbol: keep the newest
        if self.last_tick.is_none_or(|(last, _)| time_msc >= last) {
            self.last_tick = Some((time_msc, Instant::now()));
        }
    }

    /// Server time now, extrapolated from the newest tick.
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

            if self.panes.len() > 1 {
                ui.label(RichText::new(format!("Gráfico {}", self.active + 1)).color(self.pal.accent).size(12.0))
                    .on_hover_text("Símbolo e timeframe abaixo valem para o gráfico ativo (borda em destaque). Pare o mouse sobre outro gráfico (ou clique nele) para ativá-lo.");
            }
            let (current, current_tf) = (self.pane().symbol.clone(), self.pane().tf);
            let mut symbol: Option<String> = None;
            egui::ComboBox::from_id_salt("symbol")
                .selected_text(RichText::new(&current).strong())
                .width(96.0)
                .show_ui(ui, |ui| {
                    for s in &self.settings.chart.symbols {
                        if ui.selectable_label(current == *s, s).clicked() {
                            symbol = Some(s.clone());
                        }
                    }
                });
            let mut tf = None;
            for t in Timeframe::ALL {
                if ui.selectable_label(current_tf == t, t.label()).clicked() {
                    tf = Some(t);
                }
            }
            if symbol.is_some() || tf.is_some() {
                self.select(symbol.as_deref(), tf);
            }
            ui.separator();
            ui.menu_button("Gráficos", |ui| self.layout_menu(ui)).response.on_hover_text("Ver vários ativos ao mesmo tempo, lado a lado");

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

    fn navigation_bar(&mut self, ui: &mut egui::Ui) {
        let today = self.server_now()
            .and_then(|t| chrono::DateTime::from_timestamp(t as i64, 0))
            .map(|t| t.date_naive())
            .unwrap_or_else(|| chrono::Local::now().date_naive());
        let (pal, connected) = (&self.pal, self.connected);
        let pane = &mut self.panes[self.active];
        ui.horizontal_centered(|ui| {
            ui.label("Ir para data:");
            let input = ui.add(egui::TextEdit::singleline(&mut pane.navigation.input)
                .id_salt("chart-date").hint_text("DD/MM/AAAA").desired_width(100.0).char_limit(10))
                .on_hover_text("Data do gráfico (horário do servidor). Aceita DD/MM/AAAA ou AAAA-MM-DD.");
            let enter = input.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if ui.button("Ir").clicked() || enter {
                pane.navigation.request(today);
            }
            if ui.button("Hoje").on_hover_text("Voltar aos candles mais recentes e acompanhar o mercado").clicked() {
                pane.navigation.clear();
                pane.navigation.input = today.format("%d/%m/%Y").to_string();
                pane.view.reset();
                pane.view.set_auto_scale(true);
            }
            ui.separator();
            let mut auto_scale = pane.view.auto_scale();
            if ui.checkbox(&mut auto_scale, "Escala automática")
                .on_hover_text("Ajusta os preços aos candles visíveis, inclusive ao mover o gráfico. Ajustar o eixo de preços manualmente desliga.")
                .changed()
            {
                pane.view.set_auto_scale(auto_scale);
            }
            if ui.button("Reenquadrar").on_hover_text("Recuperar os candles e ligar a escala automática, mantendo a data e o zoom horizontal").clicked() {
                pane.view.set_auto_scale(true);
            }
            if let Some(time) = pane.navigation.target {
                let date = chrono::DateTime::from_timestamp(time, 0).unwrap();
                let text = if connected {
                    format!("Buscando {}…", date.format("%d/%m/%Y"))
                } else {
                    "Aguardando conexão para buscar a data…".into()
                };
                ui.label(RichText::new(text).color(pal.warn).size(12.0));
            } else if let Some(message) = &pane.navigation.message {
                ui.label(RichText::new(message).color(pal.text_dim).size(12.0));
            }
        });
        if let Some(time) = pane.navigation.resolve(&self.store, &self.loader, &pane.symbol, pane.tf) {
            pane.view.go_to(time);
            ui.ctx().request_repaint();
        }
        self.sync_auto_scale();
    }

    fn sync_auto_scale(&mut self) {
        let enabled = self.pane().view.auto_scale();
        if enabled != self.settings.chart.auto_scale {
            self.settings.chart.auto_scale = enabled;
            if let Err(e) = settings::save_auto_scale(enabled) {
                self.status = Some(format!("não gravou a escala automática no config.toml: {e}"));
            }
            self.config_mtime = settings::mtime();
        }
    }

    /// How many charts, side by side.
    fn layout_menu(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Gráficos lado a lado").color(self.pal.text_dim).size(11.5));
        let current = self.settings.chart.layout;
        let mut chosen = None;
        for grid in Grid::ALL {
            ui.horizontal(|ui| {
                grid_icon(ui, grid, if grid == current { self.pal.accent } else { self.pal.text_dim });
                if ui.selectable_label(grid == current, grid.label()).clicked() {
                    chosen = Some(grid);
                    ui.close();
                }
            });
        }
        ui.separator();
        ui.label(RichText::new("Cada gráfico tem símbolo, timeframe e indicadores próprios.\nPare o mouse sobre um gráfico (ou clique) para ativá-lo:\na barra de cima e a boleta passam a valer para ele.").color(self.pal.text_dim).size(11.5));
        if let Some(grid) = chosen.filter(|g| *g != current) {
            self.set_layout(grid, true);
        }
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
                let (mut changed, mut invalid) = (false, Vec::new());
                for p in &mut self.panes {
                    changed |= p.studies.configure_fibonacci(&s.fibonacci);
                    let (studies_changed, bad) = p.studies.configure(&s.studies);
                    changed |= studies_changed;
                    invalid = bad;
                }
                // the file changed the scale mode: every chart follows
                if s.chart.auto_scale != self.settings.chart.auto_scale {
                    for p in &mut self.panes {
                        p.view.set_auto_scale(s.chart.auto_scale);
                    }
                }
                let c = (&s.chart, &self.settings.chart);
                let charts_changed = c.0.layout != c.1.layout || c.0.charts != c.1.charts || c.0.symbol != c.1.symbol || c.0.timeframe != c.1.timeframe;
                self.settings = s;
                if charts_changed {
                    self.apply_charts();
                }
                if changed && self.connected {
                    self.request(0..self.panes.len());
                }
                let warnings: Vec<String> = warnings.into_iter().chain(invalid).collect();
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
                control::Action::Layout(grid) => self.set_layout(grid, true),
                control::Action::Chart(n) => self.activate(n.saturating_sub(1)),
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
        let pane = self.pane();
        serde_json::json!({
            "symbol": pane.symbol,
            "timeframe": pane.tf.label(),
            "layout": self.settings.chart.layout.key(),
            "active_chart": self.active + 1,
            "charts": self.panes.iter().map(|p| serde_json::json!({
                "symbol": p.symbol, "timeframe": p.tf.label(), "candles": self.store.bars(&p.symbol, p.tf).len(),
                // volume delta: wanted by the indicators, asked of the EA, closed bars received
                "delta": {
                    "want": p.delta_want, "asked": p.delta.key(), "row": p.studies.delta_row(),
                    "bars": self.store.deltas(&p.symbol, p.tf).map(|d| d.bars.len()),
                    // newest closed bar with an exact delta (behind the chart: MT5 lacks the ticks)
                    "last": self.store.deltas(&p.symbol, p.tf).and_then(|d| d.bars.keys().next_back().copied()),
                },
            })).collect::<Vec<_>>(),
            "source": if self.source == Source::Mt5 { "mt5" } else { "synthetic" },
            "connected": self.connected,
            "account": self.account.as_ref().map(|a| serde_json::json!({"server": a.server, "login": a.login, "kind": a.kind})),
            "candles": self.series().map(|s| s.bars.len()).unwrap_or(0),
            "studies": self.settings.chart.show_studies,
            "layers": self.settings.chart.layers.iter().map(|l| l.key()).collect::<Vec<_>>(),
            "cursor": self.settings.chart.cursor.key(),
            "auto_scale": pane.view.auto_scale(),
            "fibonacci": pane.studies.fibonacci_state(),
            "indicators": pane.studies.params().iter().map(|p| p.state()).collect::<Vec<_>>(),
            "positions": self.trading.positions.len(),
            "orders": self.trading.orders.len(),
            "daily_result": self.trading.day_result.as_ref().map(|day| serde_json::json!({
                "day_start": day.day_start, "realized": day.realized, "floating": day.floating,
                "total": day.total(), "currency": day.currency,
            })),
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

    /// The options of one indicator (right click on it in the chart): applied live, written to
    /// `[studies.<key>]` once the mouse is released (not on every step of a drag).
    fn study_window(&mut self, ctx: &egui::Context) {
        let Some(key) = self.study_open else { return };
        let Some(mut params) = self.pane().studies.params().iter().find(|p| p.key == key).cloned() else {
            self.study_open = None;
            return;
        };
        if self.fib_levels.is_empty() {
            self.fib_levels = fib_text(&self.settings.fibonacci.levels);
        }
        let mut open = true;
        let (mut changed, mut reset) = (false, false);
        let mut fib = self.settings.fibonacci.clone();
        let mut fib_changed = false;
        let mut fib_error = None;
        egui::Window::new(params.name).id(egui::Id::new(("study", key))).open(&mut open).resizable(false).collapsible(false).show(ctx, |ui| {
            changed = params.edit_ui(ui);
            if params.fibonacci {
                ui.add_space(6.0);
                ui.label(RichText::new("Fibonacci (M5, M15, H1, D1)").strong());
                egui::Grid::new("fibonacci").num_columns(2).min_col_width(170.0).spacing([16.0, 6.0]).show(ui, |ui| {
                    ui.label("Ligado");
                    fib_changed |= ui.checkbox(&mut fib.enabled, "").changed();
                    ui.end_row();
                    ui.label("Busca (candles fechados)");
                    fib_changed |= ui.add(egui::DragValue::new(&mut fib.lookback).range(20..=2000)).changed();
                    ui.end_row();
                    ui.label("Candles que confirmam o pivô");
                    fib_changed |= ui.add(egui::DragValue::new(&mut fib.pivot_bars).range(1..=10)).changed();
                    ui.end_row();
                    ui.label("Retrações");
                    let edit = ui.add(egui::TextEdit::singleline(&mut self.fib_levels).desired_width(180.0));
                    if edit.lost_focus() {
                        let parsed: Result<Vec<f64>, _> =
                            self.fib_levels.split([',', ';', ' ']).filter(|t| !t.is_empty()).map(|t| t.parse::<f64>()).collect();
                        match parsed {
                            Ok(levels) if levels != fib.levels => {
                                fib.levels = levels;
                                fib_changed = true;
                            }
                            Ok(_) => {}
                            Err(_) => fib_error = Some("retrações: números separados por vírgula, ex.: 0.382, 0.5, 0.618".to_string()),
                        }
                    }
                    ui.end_row();
                });
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                reset = ui.button("Padrão").on_hover_text("Volta todas as opções deste indicador ao padrão").clicked();
                let fib = if params.fibonacci { " e [fibonacci]" } else { "" };
                ui.label(RichText::new(format!("Gravado no config.toml, [studies.{key}]{fib}.")).color(self.pal.text_dim).size(11.5));
            });
        });
        if reset {
            for p in &mut params.params {
                p.value = p.default.clone();
            }
            changed = true;
        }
        if changed {
            let table = params.to_table();
            if table.is_empty() {
                self.settings.studies.remove(key);
            } else {
                self.settings.studies.insert(key.to_string(), toml::Value::Table(table));
            }
            // the options are the same in every chart
            let mut needs_changed = false;
            for p in &mut self.panes {
                needs_changed |= p.studies.configure(&self.settings.studies).0;
            }
            if needs_changed && self.connected {
                self.request(0..self.panes.len());
            }
            self.study_dirty = true;
        }
        if fib_changed {
            match settings::check_fibonacci(&fib) {
                Ok(()) => {
                    self.fib_levels = fib_text(&fib.levels);
                    let mut needs_changed = false;
                    for p in &mut self.panes {
                        needs_changed |= p.studies.configure_fibonacci(&fib);
                    }
                    if needs_changed && self.connected {
                        self.request(0..self.panes.len());
                    }
                    self.settings.fibonacci = fib;
                    if let Err(e) = settings::save_fibonacci(&self.settings.fibonacci) {
                        self.status = Some(format!("não gravou o config.toml: {e}"));
                    }
                    self.config_mtime = settings::mtime();
                }
                Err(e) => fib_error = Some(e),
            }
        }
        if let Some(e) = fib_error {
            self.fib_levels = fib_text(&self.settings.fibonacci.levels);
            self.status = Some(e);
        }
        if self.study_dirty && !ctx.input(|i| i.pointer.any_down()) {
            let table = self.settings.studies.get(key).and_then(|v| v.as_table()).cloned().unwrap_or_default();
            if let Err(e) = settings::save_study(key, &table) {
                self.status = Some(format!("não gravou o config.toml: {e}"));
            }
            self.config_mtime = settings::mtime();
            self.study_dirty = false;
        }
        if !open {
            self.study_open = None;
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
            if self.connected {
                ui.separator();
                let total = self.trading.day_result.as_ref().and_then(trading::DayResult::total);
                ui.label(RichText::new(self.trading.day_total_text()).color(trading::pnl_color(total, pal)).strong().size(12.0))
                    .on_hover_text("Resultado da conta inteira: realizado hoje com custos + posições abertas. Detalhes na boleta.");
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
        self.study_window(&ctx);
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
        let symbol = self.symbol().to_string();
        let keys = self.trading.shortcuts(ui.ctx(), &symbol, connected, real);
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
                    self.trading.ticket_ui(ui, &self.pal, &symbol, connected, real)
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
        egui::Panel::top("chart-navigation")
            .exact_size(32.0)
            .frame(egui::Frame::new().fill(self.pal.panel_bg).inner_margin(egui::Margin::symmetric(10, 0)))
            .show(ui, |ui| self.navigation_bar(ui));
        self.resync();
        let server_now = self.server_now();
        for p in &mut self.panes {
            if !p.studies.is_for(&p.symbol, p.tf) {
                p.studies = Studies::new(&p.symbol, p.tf, &self.settings.fibonacci, &self.settings.studies);
            }
        }
        self.load_more();
        self.finalize_delta();
        let mut warm = vec![false; self.panes.len()];
        let mut cmds = Vec::new();
        for (i, p) in self.panes.iter_mut().enumerate() {
            warm[i] = self.loader.ready(&self.store, &p.symbol, p.tf, Studies::warmup(p.tf));
            if self.settings.chart.show_studies {
                let digits = self.store.get(&p.symbol, p.tf).map(|s| s.digits).unwrap_or(2);
                let quotes = &self.quotes;
                let tick = self.trading.tick_size_of(&p.symbol);
                p.studies.update(&self.store, digits, tick, warm[i], server_now.map(|t| t as i64), |s| quotes.get(s).map(|&(bid, _, ms)| (bid, ms.div_euclid(1000))),
                );
            }
            // the delta once the studies know the POC level height (it needs the bars): the bars on
            // screen first
            if p.delta_want > 0 && p.delta == DeltaAsked::Nothing && self.connected
                && let Some(row) = p.studies.delta_row()
            {
                self.store.track_delta(&p.symbol, p.tf, row);
                let count = p.delta_want.min(panes::DELTA_FIRST);
                cmds.push(Command::Delta { symbol: p.symbol.clone(), tf: p.tf, count, row, skip: 0 });
                p.delta = if count < p.delta_want { DeltaAsked::Screen(Instant::now()) } else { DeltaAsked::All };
            }
        }
        // then the rest of each, behind every chart's first part (or after a while, so a chart still
        // without data doesn't hold the others)
        let first_parts_out = self.panes.iter().all(|p| p.delta_want == 0 || p.delta != DeltaAsked::Nothing);
        for p in &mut self.panes {
            if let DeltaAsked::Screen(at) = p.delta
                && (first_parts_out || at.elapsed() >= panes::DELTA_REST_AFTER)
                && let Some(d) = self.store.deltas(&p.symbol, p.tf)
            {
                let skip = panes::DELTA_FIRST;
                cmds.push(Command::Delta { symbol: p.symbol.clone(), tf: p.tf, count: p.delta_want - skip, row: d.row, skip });
                p.delta = DeltaAsked::All;
            }
        }
        if self.panes.iter().any(|p| matches!(p.delta, DeltaAsked::Screen(_))) {
            ui.ctx().request_repaint_after(panes::DELTA_REST_AFTER);
        }
        self.send_all(cmds);
        #[cfg(has_preset)]
        if let Some(v) = self.verify.as_mut() {
            let p = &self.panes[self.active];
            let cmds = v.poll(warm[self.active], &p.symbol, p.tf);
            let done = v.done.then(|| v.path.clone());
            self.send_all(cmds);
            if let Some(path) = done {
                eprintln!("verify: relatório em {path}");
                self.verify = None;
            }
        }

        // trade lines drag and the context menu opens only when orders may go out (same locks as the
        // ticket), and only in the active chart: the ticket trades its symbol
        let can_trade = self.trading.can_trade(self.connected, self.is_real());
        let show = self.settings.chart.show_studies;
        let volume_text = self.trading.volume_text();
        let empty = Series::default();
        let many = self.panes.len() > 1;
        let mut clicked = None;
        let mut hovered = None;
        let mut actions = Vec::new();
        egui::CentralPanel::no_frame().show(ui, |ui| {
            let area = ui.available_rect_before_wrap();
            if many {
                ui.painter().rect_filled(area, 0.0, self.pal.panel_bg);
            }
            let cells = panes::cells(self.settings.chart.layout, area, if many { 2.0 } else { 0.0 });
            for (i, (p, cell)) in self.panes.iter_mut().zip(cells).enumerate() {
                let active = i == self.active;
                let trade = can_trade && active;
                let positions = self.trading.levels(&p.symbol, &self.pal, trade);
                let (overlays, map_levels, pane, volume, marks, shading, ribbon, legend) = if show {
                    let s = &p.studies;
                    (s.overlays(), s.map_levels(), s.pane(), s.volume(), s.marks(), s.shading(), s.ribbon(), s.legend())
                } else {
                    (Vec::new(), Vec::new(), None, None, None, None, None, Vec::new())
                };
                let study_names: Vec<(&'static str, &'static str)> =
                    if show { p.studies.params().iter().map(|p| (p.key, p.name)).collect() } else { Vec::new() };
                let quote = if active { self.trading.quote } else { self.quotes.get(&p.symbol).map(|&(bid, ask, _)| (bid, ask)) };
                let data = ChartData {
                    series: self.store.get(&p.symbol, p.tf).unwrap_or(&empty),
                    symbol: &p.symbol,
                    tf: p.tf,
                    server_now,
                    levels: &positions,
                    overlays: &overlays,
                    map_levels: &map_levels,
                    pane: pane.as_ref(),
                    pane_open: self.settings.ui.pane_open,
                    layers: &self.settings.chart.layers,
                    quote,
                    can_trade: trade,
                    cursor: self.settings.chart.cursor,
                    tick: if active { self.trading.tick_size() } else { self.trading.tick_size_of(&p.symbol) },
                    order_volume: &volume_text,
                    bracket: active.then(|| self.trading.bracket()),
                    volume,
                    marks,
                    shading,
                    ribbon,
                    legend: &legend,
                    studies: &study_names,
                    active,
                };
                ui.scope_builder(egui::UiBuilder::new().max_rect(cell).id_salt(("chart", i)), |ui| p.view.ui(ui, &data, &self.pal));
                // a press in a chart makes it the active one at once, the pointer resting over it after
                // `HOVER_FOCUS` (both after this frame: that press never trades). Not while a button is held
                // (dragging), a menu is open or the pointer is over a window.
                if many && ui.input(|inp| inp.pointer.any_pressed() && inp.pointer.interact_pos().is_some_and(|pos| cell.contains(pos))) {
                    clicked = Some(i);
                }
                // read the pointer first: the layer lookup locks the context, which `input` holds (deadlock)
                let ctx = ui.ctx();
                let resting = ui.input(|inp| inp.pointer.hover_pos().filter(|pos| !inp.pointer.any_down() && cell.contains(*pos)));
                if many && let Some(pos) = resting
                    && !egui::Popup::is_any_open(ctx)
                    && ctx.layer_id_at(pos).is_none_or(|l| l.order == egui::Order::Background)
                {
                    hovered = Some(i);
                }
                if many && active {
                    ui.painter().rect_stroke(cell, 0.0, egui::Stroke::new(1.5, self.pal.accent), egui::StrokeKind::Inside);
                }
                actions.extend(std::mem::take(&mut p.view.actions).into_iter().map(|a| (i, trade, a)));
            }
        });
        match (clicked, hovered) {
            (Some(i), _) => {
                self.activate(i);
                self.hover_since = None;
            }
            (None, Some(i)) if i != self.active => {
                let since = match self.hover_since {
                    Some((j, t)) if j == i => t,
                    _ => self.hover_since.insert((i, Instant::now())).1,
                };
                match HOVER_FOCUS.checked_sub(since.elapsed()) {
                    Some(left) if !left.is_zero() => ui.ctx().request_repaint_after(left),
                    _ => {
                        self.activate(i);
                        self.hover_since = None;
                    }
                }
            }
            _ => self.hover_since = None,
        }
        self.sync_auto_scale();
        let mut cmds: Vec<Command> = Vec::new();
        for (i, trade, a) in actions {
            match a {
                chart::ChartAction::TogglePane => {
                    self.settings.ui.pane_open = !self.settings.ui.pane_open;
                    self.save_ui_state();
                }
                chart::ChartAction::EditStudy(key) => {
                    self.activate(i);
                    self.study_open = Some(key);
                    self.fib_levels = fib_text(&self.settings.fibonacci.levels);
                }
                a if trade => cmds.extend(self.trading.chart_action(a, &symbol)),
                _ => {}
            }
        }
        self.send_all(cmds);
    }
}

/// The shape of a grid, drawn small (menu of charts side by side).
fn grid_icon(ui: &mut egui::Ui, grid: Grid, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(22.0, 14.0), egui::Sense::hover());
    for cell in panes::cells(grid, rect, 2.0) {
        ui.painter().rect_stroke(cell, 1.0, egui::Stroke::new(1.0, color), egui::StrokeKind::Inside);
    }
}

fn fib_text(levels: &[f64]) -> String {
    levels.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(", ")
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
    /// The EA's port, opened before the window so the EA connects while the GPU starts.
    bridge: Option<Result<Feed, String>>,
    /// The egui context, once the window exists (the bridge's wake-up waits for it).
    wake: std::sync::Arc<std::sync::OnceLock<egui::Context>>,
}

fn parse_args(raw: &[String]) -> Args {
    let mut args = Args { synthetic: false, symbol: None, tf: None, verify: None, primary: true, bridge: None, wake: Default::default() };
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
    // open the EA's port first: it connects (and the first history comes) while the window and the
    // GPU start; events wait in the channel until the first frame
    let wake = args.wake.clone();
    args.bridge = Some(
        feed::bridge::spawn(feed::bridge::DEFAULT_ADDR, move || {
            if let Some(ctx) = wake.get() {
                ctx.request_repaint();
            }
        })
        .map_err(|e| format!("não abriu {}: {e}", feed::bridge::DEFAULT_ADDR)),
    );
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
