//! The configuration file, `$XDG_CONFIG_HOME/mt5-terminal/config.toml` (`~/.config/...` by default).
//! Plain commented TOML so a person or an agent can change anything; the running app applies it as
//! soon as it is saved. Changes made in the UI are written back in place, comments kept.
//! A missing file means the defaults (`DEFAULT_TOML`, also what `mt5-terminal config init` writes).

use crate::chart::{CursorMode, Layer};
use crate::model::Timeframe;
use crate::theme::Palette;
use eframe::egui::Color32;
use serde::Deserialize;
use std::path::PathBuf;
use std::time::SystemTime;

/// The defaults, documented. Every key the app reads is here.
pub const DEFAULT_TOML: &str = r##"# MT5 Terminal — configuração.
# Salvou, aplicou: o app em execução relê este arquivo na hora (não precisa reiniciar).
# Valide depois de editar: mt5-terminal config check

[chart]
# Símbolos do seletor (nomes exatos do MT5).
symbols = ["UsaTec", "UsaInd", "UsaRus"]
# Símbolo e timeframe do primeiro gráfico (M1, M5, M15, M30, H1, H4, D1, W1). O app grava a escolha.
symbol = "UsaTec"
timeframe = "M5"
# Gráficos lado a lado (também pelo menu "Gráficos"): "1" (um só), "2" (dois lado a lado),
# "2v" (dois, um sobre o outro), "3" (três lado a lado), "4" (2×2) ou "6" (3×2).
# Cada gráfico tem símbolo, timeframe e indicadores próprios. O ativo (borda em destaque) é o que está
# sob o mouse parado, ou o clicado por último: segue a barra superior, a data, a boleta e os atalhos.
layout = "1"
# Os outros gráficos, em ordem, como "SÍMBOLO TIMEFRAME" (ex.: ["UsaInd M5", "UsaRus M15"]); o app
# grava a escolha de cada um. Faltando, usa o próximo símbolo de `symbols` no timeframe do primeiro.
charts = []
# Ordem de desenho, de trás para a frente: o último fica na frente.
# Camadas: "levels" (níveis), "indicators" (linhas), "trades" (posições e ordens), "price" (candles).
layers = ["levels", "indicators", "trades", "price"]
# Indicadores do preset (pasta preset/), se houver.
show_studies = true
# Candles do primeiro bloco do gráfico (100 a 20000). Mais antigos vêm conforme você volta no tempo.
first_bars = 1000
# Cursor: "arrow" (seta), "hand" (mão para arrastar), "cross" (cruz para medir % e barras).
cursor = "hand"
# Ajustar o eixo de preços aos candles visíveis. Arrastar o gráfico mantém a escala automática.
# Ajustar o eixo de preços manualmente desliga; "Reenquadrar" liga de novo.
auto_scale = true

[fibonacci]
# Níveis no mapa de convergências, no M5, M15, H1 e D1.
enabled = true
# Último movimento entre pivôs confirmados, procurando nestes candles fechados de cada timeframe.
lookback = 300
# Candles fechados de cada lado que confirmam um topo/fundo. O candle em formação não participa.
pivot_bars = 2
# Retração a partir do fim do movimento: 0 = fim, 1 = início; 0.5 = 50%.
levels = [0.236, 0.382, 0.5, 0.618, 0.786]

# Opções de cada indicador do preset (também pelo gráfico: botão direito sobre o indicador → Editar).
# Uma tabela por indicador, só com o que difere do padrão; apagar a tabela volta ao padrão.
# `mt5-terminal ctl state` lista os indicadores ("indicators") e as opções de cada um.
# [studies.<indicador>]
# <opção> = <valor>

[mt5]
# Abrir o MetaTrader 5 ao iniciar, se ele não estiver aberto.
auto_start = true
# Comando que abre o MT5. Vazio = como o atalho do MT5 (Wine do sistema, prefixo ~/.mt5).
# Exemplo: ["env", "WINEPREFIX=/home/voce/.mt5", "wine", "C:\\Program Files\\MetaTrader 5\\terminal64.exe"]
command = []

[ui]
# Boleta aberta à direita (false = recolhida numa faixa; « abre de novo).
ticket_open = true
# Painel do preset sob o gráfico (false = minimizado numa faixa).
pane_open = true

[ticket]
# Operação predefinida ativa na boleta (nome de uma das [[presets]] abaixo); vazio = Manual.
preset = ""

# Operações predefinidas (também pela boleta: "Operação" → Editar). Com Shift/Ctrl segurado no gráfico,
# a ordem que acompanha o ponteiro já leva o volume, o stop e o alvo da operação ativa.
# unit: "percent" (% do preço de entrada) ou "points" (distância em preço). 0 = sem stop/alvo.
# [[presets]]
# name = "0.2 · 0,20% / 0,40%"
# volume = 0.2
# stop = 0.20
# target = 0.40
# unit = "percent"

# Contas para trocar pela barra de status. O app anota sozinho cada conta em que o MT5 conectar;
# a senha nunca fica aqui: o MT5 usa a que você salvou nele. kind: "demo", "real" ou "contest".
# [[accounts]]
# name = "Demo"
# login = 1234567
# server = "Corretora-Demo"
# kind = "demo"

[colors]
# Cores em "#rrggbb" (também pela janela Cores do app, com predefinições).
background = "#0d1117"   # fundo do gráfico
panel = "#0f131a"        # barras e boleta
grid = "#191f29"
text = "#d6dce6"
accent = "#3b82f6"       # destaques e seleção
up = "#26a69a"           # alta: botões, preço, volume
down = "#ef5350"         # baixa
# Candles como no MT5: corpo ("Bull/Bear candle") e contorno/pavio ("Bar up/down").
# Corpo diferente do contorno desenha o candle com borda (corpo da cor do fundo = candle vazado).
candle_up = "#26a69a"
candle_down = "#ef5350"
wick_up = "#26a69a"
wick_down = "#ef5350"
"##;

/// Every color the user picks: key in `[colors]` and its name in the Colors window.
pub const COLOR_KEYS: [(&str, &str); 11] = [
    ("background", "Fundo do gráfico"),
    ("panel", "Painéis (barras e boleta)"),
    ("grid", "Grade"),
    ("text", "Texto"),
    ("accent", "Destaque"),
    ("up", "Alta (botões, preço, volume)"),
    ("down", "Baixa"),
    ("candle_up", "Corpo do candle de alta"),
    ("candle_down", "Corpo do candle de baixa"),
    ("wick_up", "Contorno e pavio de alta"),
    ("wick_down", "Contorno e pavio de baixa"),
];

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Chart {
    pub symbols: Vec<String>,
    pub symbol: String,
    pub timeframe: Timeframe,
    /// Plot layers, back to front.
    pub layers: Vec<Layer>,
    pub show_studies: bool,
    pub first_bars: u32,
    pub cursor: CursorMode,
    pub auto_scale: bool,
    pub layout: Grid,
    /// Charts after the first, as "SYMBOL TIMEFRAME" (`chart_entry`).
    pub charts: Vec<String>,
}

/// How many charts the window shows, and how they are arranged.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize)]
pub enum Grid {
    #[default]
    #[serde(rename = "1")]
    One,
    #[serde(rename = "2")]
    Two,
    #[serde(rename = "2v")]
    TwoStacked,
    #[serde(rename = "3")]
    Three,
    #[serde(rename = "4")]
    Four,
    #[serde(rename = "6")]
    Six,
}

impl Grid {
    pub const ALL: [Grid; 6] = [Grid::One, Grid::Two, Grid::TwoStacked, Grid::Three, Grid::Four, Grid::Six];

    /// Name in the config file and in `ctl`.
    pub fn key(self) -> &'static str {
        match self {
            Grid::One => "1",
            Grid::Two => "2",
            Grid::TwoStacked => "2v",
            Grid::Three => "3",
            Grid::Four => "4",
            Grid::Six => "6",
        }
    }

    pub fn from_key(key: &str) -> Option<Grid> {
        Grid::ALL.into_iter().find(|g| g.key() == key)
    }

    pub fn label(self) -> &'static str {
        match self {
            Grid::One => "1 gráfico",
            Grid::Two => "2 lado a lado",
            Grid::TwoStacked => "2 empilhados",
            Grid::Three => "3 lado a lado",
            Grid::Four => "4 (2×2)",
            Grid::Six => "6 (3×2)",
        }
    }

    /// Columns and rows.
    pub fn shape(self) -> (usize, usize) {
        match self {
            Grid::One => (1, 1),
            Grid::Two => (2, 1),
            Grid::TwoStacked => (1, 2),
            Grid::Three => (3, 1),
            Grid::Four => (2, 2),
            Grid::Six => (3, 2),
        }
    }

    pub fn count(self) -> usize {
        let (c, r) = self.shape();
        c * r
    }
}

/// "SYMBOL TIMEFRAME" of `chart.charts` (the symbol may not contain spaces in MT5 either way).
pub fn chart_entry(text: &str) -> Option<(String, Timeframe)> {
    let (symbol, tf) = text.trim().rsplit_once(char::is_whitespace)?;
    let (symbol, tf) = (symbol.trim(), Timeframe::parse(tf)?);
    (!symbol.is_empty()).then(|| (symbol.to_string(), tf))
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Fibonacci {
    pub enabled: bool,
    pub lookback: usize,
    pub pivot_bars: usize,
    pub levels: Vec<f64>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Mt5 {
    pub auto_start: bool,
    pub command: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Colors {
    pub background: String,
    pub panel: String,
    pub grid: String,
    pub text: String,
    pub accent: String,
    pub up: String,
    pub down: String,
    pub candle_up: String,
    pub candle_down: String,
    pub wick_up: String,
    pub wick_down: String,
}

impl Colors {
    pub fn get(&self, key: &str) -> &str {
        match key {
            "background" => &self.background,
            "panel" => &self.panel,
            "grid" => &self.grid,
            "text" => &self.text,
            "accent" => &self.accent,
            "up" => &self.up,
            "down" => &self.down,
            "candle_up" => &self.candle_up,
            "candle_down" => &self.candle_down,
            "wick_up" => &self.wick_up,
            "wick_down" => &self.wick_down,
            _ => "",
        }
    }

    pub fn set(&mut self, key: &str, value: String) {
        let slot = match key {
            "background" => &mut self.background,
            "panel" => &mut self.panel,
            "grid" => &mut self.grid,
            "text" => &mut self.text,
            "accent" => &mut self.accent,
            "up" => &mut self.up,
            "down" => &mut self.down,
            "candle_up" => &mut self.candle_up,
            "candle_down" => &mut self.candle_down,
            "wick_up" => &mut self.wick_up,
            "wick_down" => &mut self.wick_down,
            _ => return,
        };
        *slot = value;
    }

    pub fn color(&self, key: &str) -> Color32 {
        parse_hex(self.get(key)).unwrap_or(Color32::MAGENTA)
    }
}

/// Ready-made schemes for the Colors window: (name, colors).
pub fn presets() -> Vec<(&'static str, Colors)> {
    let c = |v: [&str; 11]| Colors {
        background: v[0].into(),
        panel: v[1].into(),
        grid: v[2].into(),
        text: v[3].into(),
        accent: v[4].into(),
        up: v[5].into(),
        down: v[6].into(),
        candle_up: v[7].into(),
        candle_down: v[8].into(),
        wick_up: v[9].into(),
        wick_down: v[10].into(),
    };
    vec![
        ("Padrão escuro", Settings::default().colors),
        // MT5's default "Green on Black": hollow bull candles, white bear bodies, lime bars
        ("MetaTrader clássico", c(["#000000", "#101010", "#2f3a45", "#ffffff", "#1e90ff", "#00ff00", "#ff3030", "#000000", "#ffffff", "#00ff00", "#00ff00"])),
        ("Claro", c(["#ffffff", "#f3f4f6", "#e5e7eb", "#111827", "#2563eb", "#089981", "#f23645", "#089981", "#f23645", "#089981", "#f23645"])),
        ("TradingView", c(["#131722", "#1e222d", "#232837", "#d1d4dc", "#2962ff", "#089981", "#f23645", "#089981", "#f23645", "#089981", "#f23645"])),
    ]
}

pub fn color_hex(c: Color32) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b())
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Account {
    pub name: String,
    pub login: i64,
    pub server: String,
    pub kind: String,
}

impl Account {
    pub fn is_real(&self) -> bool {
        self.kind == "real"
    }
}

/// A ready-made order: volume and stop/target distances from the entry.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Preset {
    pub name: String,
    pub volume: f64,
    #[serde(default)]
    pub stop: f64,
    #[serde(default)]
    pub target: f64,
    /// "percent" of the entry price, or "points" (price distance).
    #[serde(default = "percent")]
    pub unit: String,
}

fn percent() -> String {
    "percent".into()
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Ui {
    pub ticket_open: bool,
    pub pane_open: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct Ticket {
    /// Active preset by name; empty = manual.
    #[serde(default)]
    pub preset: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Settings {
    pub chart: Chart,
    pub fibonacci: Fibonacci,
    pub mt5: Mt5,
    pub colors: Colors,
    pub ui: Ui,
    #[serde(default)]
    pub accounts: Vec<Account>,
    #[serde(default)]
    pub ticket: Ticket,
    #[serde(default)]
    pub presets: Vec<Preset>,
    /// `[studies.<indicator>]`: the options each indicator changed (checked by the preset).
    #[serde(default)]
    pub studies: toml::Table,
}

impl Default for Settings {
    fn default() -> Self {
        toml::from_str(DEFAULT_TOML).expect("DEFAULT_TOML is valid")
    }
}

pub fn path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("mt5-terminal/config.toml"))
}

pub fn mtime() -> Option<SystemTime> {
    path().and_then(|p| std::fs::metadata(p).ok()).and_then(|m| m.modified().ok())
}

pub fn parse_hex(s: &str) -> Option<Color32> {
    let h = s.strip_prefix('#')?;
    if h.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    Some(Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

/// Every layer exactly once: the given order first, then any missing one behind them.
fn normalize(saved: &[Layer]) -> Vec<Layer> {
    let mut out: Vec<Layer> = Vec::new();
    for l in saved {
        if !out.contains(l) {
            out.push(*l);
        }
    }
    for l in Layer::DEFAULT.iter().rev() {
        if !out.contains(l) {
            out.insert(0, *l);
        }
    }
    out
}

/// Keys present in `user` but not in `defaults`, as dotted paths (typos an agent should hear about).
fn unknown_keys(user: &toml::Table, defaults: &toml::Table, prefix: &str, out: &mut Vec<String>) {
    for (k, v) in user {
        let name = format!("{prefix}{k}");
        match (v, defaults.get(k)) {
            (_, None) => out.push(name),
            (toml::Value::Table(u), Some(toml::Value::Table(d))) => {
                unknown_keys(u, d, &format!("{name}."), out)
            }
            _ => {}
        }
    }
}

/// Parse and check a config text over the defaults. Ok: the settings and warnings (unknown keys,
/// values fixed up); Err: why the file can't be used.
pub fn parse(text: &str) -> Result<(Settings, Vec<String>), String> {
    let user: toml::Table = text.parse().map_err(|e: toml::de::Error| e.to_string())?;
    let mut merged: toml::Table = DEFAULT_TOML.parse().expect("DEFAULT_TOML is valid");
    let mut warnings = Vec::new();
    let mut known = merged.clone();
    known.insert("accounts".into(), toml::Value::Array(Vec::new()));
    known.insert("presets".into(), toml::Value::Array(Vec::new()));
    // checked by the indicators themselves (they know their options)
    known.insert("studies".into(), toml::Value::Array(Vec::new()));
    unknown_keys(&user, &known, "", &mut warnings);
    let warnings_unknown: Vec<String> = warnings.drain(..).map(|k| format!("chave desconhecida: {k}")).collect();
    for (section, value) in user {
        match (merged.get_mut(&section), value) {
            (Some(toml::Value::Table(d)), toml::Value::Table(u)) => d.extend(u),
            (_, v) => {
                merged.insert(section, v);
            }
        }
    }
    let mut s: Settings = toml::Value::Table(merged).try_into().map_err(|e: toml::de::Error| e.to_string())?;
    warnings.extend(warnings_unknown);
    let fixed = normalize(&s.chart.layers);
    if fixed != s.chart.layers {
        warnings.push("chart.layers: cada camada uma vez; as que faltavam foram para trás".into());
        s.chart.layers = fixed;
    }
    if !(100..=20_000).contains(&s.chart.first_bars) {
        warnings.push(format!("chart.first_bars = {} fora de 100..20000; usando 1000", s.chart.first_bars));
        s.chart.first_bars = 1000;
    }
    check_fibonacci(&s.fibonacci)?;
    if s.chart.symbols.is_empty() {
        warnings.push("chart.symbols vazio; usando o padrão".into());
        s.chart.symbols = Settings::default().chart.symbols;
    }
    if !s.chart.symbols.contains(&s.chart.symbol) {
        s.chart.symbols.insert(0, s.chart.symbol.clone());
    }
    let mut charts = Vec::new();
    for text in std::mem::take(&mut s.chart.charts) {
        match chart_entry(&text) {
            Some((symbol, tf)) => {
                if !s.chart.symbols.contains(&symbol) {
                    s.chart.symbols.push(symbol.clone());
                }
                charts.push(format!("{symbol} {}", tf.label()));
            }
            None => warnings.push(format!("chart.charts: {text:?} ignorado; use \"SÍMBOLO TIMEFRAME\", ex.: \"UsaInd M5\"")),
        }
    }
    s.chart.charts = charts;
    for p in &s.presets {
        if p.unit != "percent" && p.unit != "points" {
            return Err(format!("presets \"{}\": unit = {:?}: use \"percent\" ou \"points\"", p.name, p.unit));
        }
        if p.volume.is_nan() || p.volume <= 0.0 || p.stop < 0.0 || p.target < 0.0 {
            return Err(format!("presets \"{}\": volume > 0 e stop/target >= 0", p.name));
        }
    }
    if !s.ticket.preset.is_empty() && !s.presets.iter().any(|p| p.name == s.ticket.preset) {
        warnings.push(format!("ticket.preset = {:?} não existe em [[presets]]; usando Manual", s.ticket.preset));
        s.ticket.preset.clear();
    }
    for (key, _) in COLOR_KEYS {
        let v = s.colors.get(key);
        if parse_hex(v).is_none() {
            return Err(format!("colors.{key} = {v:?}: use \"#rrggbb\""));
        }
    }
    Ok((s, warnings))
}

/// The `[fibonacci]` rules (also checked by the indicator editor before saving).
pub fn check_fibonacci(f: &Fibonacci) -> Result<(), String> {
    if !(20..=2000).contains(&f.lookback) {
        return Err("fibonacci.lookback: use 20..2000 candles fechados".into());
    }
    if !(1..=10).contains(&f.pivot_bars)
        || 2 * f.pivot_bars + 3 > f.lookback
    {
        return Err("fibonacci.pivot_bars: use 1..10 e lookback >= 2 * pivot_bars + 3".into());
    }
    if f.levels.is_empty()
        || f.levels.len() > 12
        || f
            .levels
            .iter()
            .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
    {
        return Err("fibonacci.levels: de 1 a 12 proporções finitas entre 0 e 1".into());
    }
    for (i, v) in f.levels.iter().enumerate() {
        if f.levels[..i]
            .iter()
            .any(|other| (other - v).abs() < 1e-9)
        {
            return Err("fibonacci.levels: cada proporção deve aparecer uma vez".into());
        }
    }
    Ok(())
}

impl Settings {
    /// The file (or the defaults when there is none). Err only for a file that exists and is invalid.
    pub fn load() -> Result<(Settings, Vec<String>), String> {
        match path().map(std::fs::read_to_string) {
            Some(Ok(text)) => parse(&text),
            _ => Ok((Settings::default(), Vec::new())),
        }
    }

    /// Write the documented defaults if there is no file yet. Returns the path.
    pub fn init() -> std::io::Result<PathBuf> {
        let p = path().ok_or_else(|| std::io::Error::other("sem HOME"))?;
        if !p.exists() {
            if let Some(dir) = p.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(&p, DEFAULT_TOML)?;
        }
        Ok(p)
    }

    /// Save what the UI changes (layers, studies) into the file, keeping its comments and the rest.
    pub fn save_ui(&self) {
        let _ = save_chart(&self.chart.layers, self.chart.show_studies);
    }

    pub fn palette(&self) -> Palette {
        let c = &self.colors;
        let mut p = Palette::default();
        p.chart_bg = c.color("background");
        p.panel_bg = c.color("panel");
        p.app_bg = c.color("panel");
        p.grid = c.color("grid");
        p.text = c.color("text");
        // secondary text and tags follow the text and panel colors, so light themes stay readable
        p.text_dim = p.text.lerp_to_gamma(p.panel_bg, 0.4);
        p.tag_bg = p.panel_bg.lerp_to_gamma(p.text, 0.18);
        p.border = p.panel_bg.lerp_to_gamma(p.text, 0.12);
        p.crosshair = p.text.lerp_to_gamma(p.chart_bg, 0.4);
        p.accent = c.color("accent");
        p.up = c.color("up");
        p.down = c.color("down");
        p.candle_up = c.color("candle_up");
        p.candle_down = c.color("candle_down");
        p.wick_up = c.color("wick_up");
        p.wick_down = c.color("wick_down");
        p.up_vol = Color32::from_rgba_unmultiplied(p.up.r(), p.up.g(), p.up.b(), 60);
        p.down_vol = Color32::from_rgba_unmultiplied(p.down.r(), p.down.g(), p.down.b(), 60);
        p
    }
}

/// Edit `[chart]` in the file in place (comments kept); creates it from the defaults if missing.
fn edit_chart(f: impl FnOnce(&mut toml_edit::Item)) -> std::io::Result<()> {
    let p = Settings::init()?;
    let text = std::fs::read_to_string(&p)?;
    let mut doc = text.parse::<toml_edit::DocumentMut>().map_err(std::io::Error::other)?;
    if !doc.contains_table("chart") {
        doc["chart"] = toml_edit::table();
    }
    f(&mut doc["chart"]);
    std::fs::write(p, doc.to_string())
}

/// Note an account the MT5 connected to, once (by login and server). Keeps the file's comments.
pub fn remember_account(login: i64, server: &str, kind: &str) -> std::io::Result<bool> {
    let p = Settings::init()?;
    let text = std::fs::read_to_string(&p)?;
    let mut doc = text.parse::<toml_edit::DocumentMut>().map_err(std::io::Error::other)?;
    let known = doc
        .get("accounts")
        .and_then(|a| a.as_array_of_tables())
        .is_some_and(|t| {
            t.iter().any(|a| {
                a.get("login").and_then(|v| v.as_integer()) == Some(login) && a.get("server").and_then(|v| v.as_str()) == Some(server)
            })
        });
    if known {
        return Ok(false);
    }
    let name = match kind {
        "real" => "Real",
        "contest" => "Concurso",
        _ => "Demo",
    };
    let mut t = toml_edit::Table::new();
    t["name"] = toml_edit::value(format!("{name} {login}"));
    t["login"] = toml_edit::value(login);
    t["server"] = toml_edit::value(server);
    t["kind"] = toml_edit::value(kind);
    match doc.get_mut("accounts").and_then(|a| a.as_array_of_tables_mut()) {
        Some(a) => a.push(t),
        None => {
            let mut a = toml_edit::ArrayOfTables::new();
            a.push(t);
            doc.insert("accounts", toml_edit::Item::ArrayOfTables(a));
        }
    }
    std::fs::write(p, doc.to_string())?;
    Ok(true)
}

/// Write the presets and the active one (the `[[presets]]` tables are rewritten; other comments kept).
pub fn save_presets(presets: &[Preset], active: &str) -> std::io::Result<()> {
    let p = Settings::init()?;
    let text = std::fs::read_to_string(&p)?;
    let mut doc = text.parse::<toml_edit::DocumentMut>().map_err(std::io::Error::other)?;
    if !doc.contains_table("ticket") {
        doc["ticket"] = toml_edit::table();
    }
    doc["ticket"]["preset"] = toml_edit::value(active);
    let mut tables = toml_edit::ArrayOfTables::new();
    for pr in presets {
        let mut t = toml_edit::Table::new();
        t["name"] = toml_edit::value(pr.name.as_str());
        t["volume"] = toml_edit::value(pr.volume);
        t["stop"] = toml_edit::value(pr.stop);
        t["target"] = toml_edit::value(pr.target);
        t["unit"] = toml_edit::value(pr.unit.as_str());
        tables.push(t);
    }
    if presets.is_empty() {
        doc.remove("presets");
    } else {
        doc.insert("presets", toml_edit::Item::ArrayOfTables(tables));
    }
    std::fs::write(p, doc.to_string())
}

/// Write `[ui]` in place (comments kept).
pub fn save_ui_state(ui: &Ui) -> std::io::Result<()> {
    let p = Settings::init()?;
    let text = std::fs::read_to_string(&p)?;
    let mut doc = text.parse::<toml_edit::DocumentMut>().map_err(std::io::Error::other)?;
    if !doc.contains_table("ui") {
        doc["ui"] = toml_edit::table();
    }
    doc["ui"]["ticket_open"] = toml_edit::value(ui.ticket_open);
    doc["ui"]["pane_open"] = toml_edit::value(ui.pane_open);
    std::fs::write(p, doc.to_string())
}

/// Write `[colors]` in place (comments kept).
pub fn save_colors(colors: &Colors) -> std::io::Result<()> {
    let p = Settings::init()?;
    let text = std::fs::read_to_string(&p)?;
    let mut doc = text.parse::<toml_edit::DocumentMut>().map_err(std::io::Error::other)?;
    if !doc.contains_table("colors") {
        doc["colors"] = toml_edit::table();
    }
    for (key, _) in COLOR_KEYS {
        // keep the comment that follows a value on the same line
        match doc["colors"].get_mut(key).and_then(|i| i.as_value_mut()) {
            Some(v) => {
                let decor = v.decor().clone();
                *v = toml_edit::Value::from(colors.get(key));
                *v.decor_mut() = decor;
            }
            None => doc["colors"][key] = toml_edit::value(colors.get(key)),
        }
    }
    std::fs::write(p, doc.to_string())
}

/// Replace `[studies.<key>]` with `values` (none: the table goes, back to the defaults).
pub fn save_study(key: &str, values: &toml::Table) -> std::io::Result<()> {
    let p = Settings::init()?;
    let text = std::fs::read_to_string(&p)?;
    let mut doc = text.parse::<toml_edit::DocumentMut>().map_err(std::io::Error::other)?;
    if !doc.contains_table("studies") {
        let mut t = toml_edit::Table::new();
        t.set_implicit(true);
        doc.insert("studies", toml_edit::Item::Table(t));
    }
    let studies = doc["studies"].as_table_mut().ok_or_else(|| std::io::Error::other("studies não é uma tabela"))?;
    if values.is_empty() {
        studies.remove(key);
    } else {
        let mut t = toml_edit::Table::new();
        for (k, v) in values {
            t[k.as_str()] = toml_edit::value(edit_value(v));
        }
        studies.insert(key, toml_edit::Item::Table(t));
    }
    std::fs::write(&p, doc.to_string())
}

fn edit_value(v: &toml::Value) -> toml_edit::Value {
    match v {
        toml::Value::Boolean(b) => (*b).into(),
        toml::Value::Integer(i) => (*i).into(),
        toml::Value::Float(x) => (*x).into(),
        toml::Value::Array(a) => a.iter().map(edit_value).collect::<toml_edit::Array>().into(),
        other => other.as_str().map(String::from).unwrap_or_else(|| other.to_string()).into(),
    }
}

/// Write `[fibonacci]` in place (comments kept).
pub fn save_fibonacci(f: &Fibonacci) -> std::io::Result<()> {
    let p = Settings::init()?;
    let text = std::fs::read_to_string(&p)?;
    let mut doc = text.parse::<toml_edit::DocumentMut>().map_err(std::io::Error::other)?;
    if !doc.contains_table("fibonacci") {
        doc["fibonacci"] = toml_edit::table();
    }
    let fib = &mut doc["fibonacci"];
    fib["enabled"] = toml_edit::value(f.enabled);
    fib["lookback"] = toml_edit::value(f.lookback as i64);
    fib["pivot_bars"] = toml_edit::value(f.pivot_bars as i64);
    fib["levels"] = toml_edit::value(f.levels.iter().copied().collect::<toml_edit::Array>());
    std::fs::write(&p, doc.to_string())
}

pub fn save_chart(layers: &[Layer], show_studies: bool) -> std::io::Result<()> {
    let array: toml_edit::Array = layers.iter().map(|l| l.key()).collect();
    edit_chart(|chart| {
        chart["layers"] = toml_edit::value(array);
        chart["show_studies"] = toml_edit::value(show_studies);
    })
}

pub fn save_cursor(cursor: CursorMode) -> std::io::Result<()> {
    edit_chart(|chart| chart["cursor"] = toml_edit::value(cursor.key()))
}

pub fn save_auto_scale(enabled: bool) -> std::io::Result<()> {
    edit_chart(|chart| chart["auto_scale"] = toml_edit::value(enabled))
}

/// The grid and every chart's symbol and timeframe: the first in `symbol`/`timeframe`, the others in
/// `charts` (entries past the open charts are kept, for when the grid grows again).
pub fn save_charts(layout: Grid, symbol: &str, tf: Timeframe, charts: &[String]) -> std::io::Result<()> {
    let array: toml_edit::Array = charts.iter().map(String::as_str).collect();
    edit_chart(|chart| {
        chart["symbol"] = toml_edit::value(symbol);
        chart["timeframe"] = toml_edit::value(tf.label());
        chart["layout"] = toml_edit::value(layout.key());
        chart["charts"] = toml_edit::value(array);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fibonacci_defaults_work_with_old_configs_and_validate_inputs() {
        let (s, warnings) = parse("[chart]\ncursor = 'arrow'").unwrap();
        assert!(warnings.is_empty());
        assert!(s.fibonacci.enabled);
        assert_eq!(s.fibonacci.lookback, 300);
        assert_eq!(s.fibonacci.pivot_bars, 2);
        assert_eq!(s.fibonacci.levels, [0.236, 0.382, 0.5, 0.618, 0.786]);
        for value in [
            "lookback = 2",
            "pivot_bars = 0",
            "pivot_bars = 11",
            "levels = []",
            "levels = [0.5, 0.5]",
            "levels = [1.1]",
            "levels = [nan]",
        ] {
            assert!(parse(&format!("[fibonacci]\n{value}")).is_err(), "{value}");
        }
        assert!(
            !parse("[fibonacci]\nenabled = false")
                .unwrap()
                .0
                .fibonacci
                .enabled
        );
    }

    #[test]
    fn defaults_parse_and_match() {
        let (s, w) = parse(DEFAULT_TOML).unwrap();
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(s, Settings::default());
        assert_eq!(*s.chart.layers.last().unwrap(), Layer::Price);
        assert_eq!(s.palette().up, Color32::from_rgb(0x26, 0xa6, 0x9a));
        assert_eq!(s.chart.cursor, CursorMode::Hand);
        assert!(s.chart.auto_scale);
    }

    #[test]
    fn charts_side_by_side_parse_and_skip_bad_entries() {
        let (s, warnings) = parse("").unwrap();
        assert_eq!((s.chart.layout, s.chart.charts.len()), (Grid::One, 0));
        let (s, warnings_bad) = parse("[chart]\nlayout = '4'\ncharts = ['UsaInd m15', 'Novo H1', 'semtf', 'X Q9']").unwrap();
        assert!(warnings.is_empty());
        assert_eq!(s.chart.layout, Grid::Four);
        assert_eq!(s.chart.layout.count(), 4);
        assert_eq!(s.chart.charts, ["UsaInd M15", "Novo H1"]);
        assert!(s.chart.symbols.contains(&"Novo".to_string()));
        assert_eq!(warnings_bad.len(), 2);
        assert!(parse("[chart]\nlayout = '5'").is_err());
        assert_eq!(chart_entry(" WIN$N  M5 "), Some(("WIN$N".into(), Timeframe::M5)));
        for g in Grid::ALL {
            assert_eq!(Grid::from_key(g.key()), Some(g));
        }
    }

    #[test]
    fn auto_scale_defaults_for_old_configs_and_accepts_manual_mode() {
        assert!(parse("[chart]\nsymbol = 'X'").unwrap().0.chart.auto_scale);
        let (s, warnings) = parse("[chart]\nauto_scale = false").unwrap();
        assert!(!s.chart.auto_scale);
        assert!(warnings.is_empty());
        assert!(parse("[chart]\nauto_scale = 'false'").is_err());
    }

    #[test]
    fn cursor_config_accepts_modes_and_old_files_keep_default() {
        for cursor in CursorMode::ALL {
            let (s, w) = parse(&format!("[chart]\ncursor = \"{}\"\n", cursor.key())).unwrap();
            assert_eq!(s.chart.cursor, cursor);
            assert!(w.is_empty());
        }
        assert_eq!(parse("[chart]\nsymbol = \"X\"\n").unwrap().0.chart.cursor, CursorMode::Hand);
        assert!(parse("[chart]\ncursor = \"bad\"\n").is_err());
    }

    #[test]
    fn partial_file_over_defaults_with_warnings() {
        let (s, w) = parse("[chart]\nsymbol = \"UsaRus\"\nlayers = [\"price\", \"indicators\"]\ncolour = 1\n[mt5]\nauto_start = false\n").unwrap();
        assert_eq!(s.chart.symbol, "UsaRus");
        assert_eq!(s.chart.timeframe, Timeframe::M5);
        assert!(!s.mt5.auto_start);
        assert_eq!(s.chart.layers, [Layer::Levels, Layer::Trades, Layer::Price, Layer::Indicators]);
        assert!(w.iter().any(|x| x.contains("chart.colour")), "{w:?}");
        assert!(parse("[colors]\nup = \"green\"\n").unwrap_err().contains("colors.up"));
        // an old file with only the first colors gets the candle defaults
        let (s, _) = parse("[colors]\nup = \"#00ff00\"\n").unwrap();
        assert_eq!((s.colors.up.as_str(), s.colors.candle_up.as_str()), ("#00ff00", "#26a69a"));
        for (name, c) in presets() {
            for (key, _) in COLOR_KEYS {
                assert!(parse_hex(c.get(key)).is_some(), "{name}: {key}");
            }
        }
        assert_eq!(color_hex(Color32::from_rgb(1, 2, 255)), "#0102ff");
        let (s, w) = parse("[ticket]\npreset = \"P\"\n[[presets]]\nname = \"P\"\nvolume = 0.2\nstop = 0.2\ntarget = 0.4\n").unwrap();
        assert!(w.is_empty(), "{w:?}");
        assert_eq!((s.presets[0].unit.as_str(), s.ticket.preset.as_str()), ("percent", "P"));
        assert!(parse("[[presets]]\nname = \"X\"\nvolume = 1\nunit = \"pips\"\n").is_err());
        let (s, w) = parse("[ticket]\npreset = \"nada\"\n").unwrap();
        assert!(s.ticket.preset.is_empty() && !w.is_empty());
        assert!(parse("[chart\n").is_err());
        let (s, w) = parse("[[accounts]]\nname = \"Real\"\nlogin = 1\nserver = \"S\"\nkind = \"real\"\n").unwrap();
        assert!(w.is_empty(), "{w:?}");
        assert!(s.accounts[0].is_real());
    }
}
