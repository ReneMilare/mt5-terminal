//! The configuration file, `$XDG_CONFIG_HOME/mt5-terminal/config.toml` (`~/.config/...` by default).
//! Plain commented TOML so a person or an agent can change anything; the running app applies it as
//! soon as it is saved. Changes made in the UI are written back in place, comments kept.
//! A missing file means the defaults (`DEFAULT_TOML`, also what `mt5-terminal config init` writes).

use crate::chart::Layer;
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
# Símbolo e timeframe ao abrir (M1, M5, M15, M30, H1, H4, D1, W1).
symbol = "UsaTec"
timeframe = "M5"
# Ordem de desenho, de trás para a frente: o último fica na frente.
# Camadas: "levels" (níveis), "indicators" (linhas), "trades" (posições e ordens), "price" (candles).
layers = ["levels", "indicators", "trades", "price"]
# Indicadores do preset (pasta preset/), se houver.
show_studies = true
# Candles do primeiro bloco do gráfico (100 a 20000). Mais antigos vêm conforme você volta no tempo.
first_bars = 1000

[mt5]
# Abrir o MetaTrader 5 ao iniciar, se ele não estiver aberto.
auto_start = true
# Comando que abre o MT5. Vazio = como o atalho do MT5 (Wine do sistema, prefixo ~/.mt5).
# Exemplo: ["env", "WINEPREFIX=/home/voce/.mt5", "wine", "C:\\Program Files\\MetaTrader 5\\terminal64.exe"]
command = []

[colors]
# Cores em "#rrggbb".
up = "#26a69a"
down = "#ef5350"
background = "#0d1117"
grid = "#191f29"
text = "#d6dce6"
accent = "#3b82f6"
"##;

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Chart {
    pub symbols: Vec<String>,
    pub symbol: String,
    pub timeframe: Timeframe,
    /// Plot layers, back to front.
    pub layers: Vec<Layer>,
    pub show_studies: bool,
    pub first_bars: u32,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Mt5 {
    pub auto_start: bool,
    pub command: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Colors {
    pub up: String,
    pub down: String,
    pub background: String,
    pub grid: String,
    pub text: String,
    pub accent: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Settings {
    pub chart: Chart,
    pub mt5: Mt5,
    pub colors: Colors,
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

fn hex(s: &str) -> Option<Color32> {
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
            (toml::Value::Table(u), Some(toml::Value::Table(d))) => unknown_keys(u, d, &format!("{name}."), out),
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
    unknown_keys(&user, &merged, "", &mut warnings);
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
    if s.chart.symbols.is_empty() {
        warnings.push("chart.symbols vazio; usando o padrão".into());
        s.chart.symbols = Settings::default().chart.symbols;
    }
    if !s.chart.symbols.contains(&s.chart.symbol) {
        s.chart.symbols.insert(0, s.chart.symbol.clone());
    }
    let c = &s.colors;
    for (name, v) in [("up", &c.up), ("down", &c.down), ("background", &c.background), ("grid", &c.grid), ("text", &c.text), ("accent", &c.accent)] {
        if hex(v).is_none() {
            return Err(format!("colors.{name} = {v:?}: use \"#rrggbb\""));
        }
    }
    Ok((s, warnings))
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
        let set = |dst: &mut Color32, v: &str| {
            if let Some(color) = hex(v) {
                *dst = color;
            }
        };
        set(&mut p.up, &c.up);
        set(&mut p.down, &c.down);
        set(&mut p.chart_bg, &c.background);
        set(&mut p.grid, &c.grid);
        set(&mut p.text, &c.text);
        set(&mut p.accent, &c.accent);
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

pub fn save_chart(layers: &[Layer], show_studies: bool) -> std::io::Result<()> {
    let array: toml_edit::Array = layers.iter().map(|l| l.key()).collect();
    edit_chart(|chart| {
        chart["layers"] = toml_edit::value(array);
        chart["show_studies"] = toml_edit::value(show_studies);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_parse_and_match() {
        let (s, w) = parse(DEFAULT_TOML).unwrap();
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(s, Settings::default());
        assert_eq!(*s.chart.layers.last().unwrap(), Layer::Price);
        assert_eq!(s.palette().up, Color32::from_rgb(0x26, 0xa6, 0x9a));
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
        assert!(parse("[chart\n").is_err());
    }
}
