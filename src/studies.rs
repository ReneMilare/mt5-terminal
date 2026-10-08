//! Indicators drawn over the chart. The app itself ships none: a preset in `preset/` (a separate,
//! private repository, ignored by this one) is compiled in when the directory exists — `build.rs`
//! sets `cfg(has_preset)`. Any preset implements the API of `none::Studies` below.
//!
//! Rules for a preset: incremental (a tick without a new bar is O(1)), nothing heavy on the UI thread,
//! and it asks for history through [`Needs`] instead of loading everything (see `history`).

use crate::history::Need;
use crate::model::Timeframe;
use eframe::egui::{self, Color32, RichText};

/// What the studies read besides the chart's own series.
#[derive(Default)]
pub struct Needs {
    /// Symbols whose ticks they read (the chart symbol is always subscribed).
    pub symbols: Vec<String>,
    /// First chunk of each extra series: (symbol, timeframe, bars).
    pub first: Vec<(String, Timeframe, u32)>,
    /// How far back extra series must reach: (symbol, timeframe, need).
    pub older: Vec<(String, Timeframe, Need)>,
    /// Closed bars of the chart whose volume delta (and POC, see `delta_row`) they read (0 = none).
    pub delta_bars: u32,
}

/// What kind of value an indicator option holds, and its range.
#[cfg_attr(not(has_preset), allow(dead_code))] // declared by the preset
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Bool,
    Number { min: f64, max: f64, step: f64, decimals: usize },
    Color,
    /// One of (key in config.toml, label).
    Choice(&'static [(&'static str, &'static str)]),
    /// Any set of timeframes, kept in the order of `Timeframe::ALL`.
    Timeframes,
}

#[cfg_attr(not(has_preset), allow(dead_code))] // declared by the preset
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Bool(bool),
    Number(f64),
    Color(Color32),
    /// Index into the options of `Kind::Choice`.
    Choice(usize),
    Timeframes(Vec<Timeframe>),
}

/// One option of an indicator, edited in its window and stored in `[studies.<indicator>]`.
#[derive(Clone, Debug)]
pub struct Param {
    pub key: &'static str,
    pub label: &'static str,
    /// Heading the option is listed under in the editor ("" = none).
    pub group: &'static str,
    pub hint: &'static str,
    pub kind: Kind,
    pub default: Value,
    pub value: Value,
}

#[cfg_attr(not(has_preset), allow(dead_code))] // declared by the preset
impl Param {
    pub fn new(key: &'static str, label: &'static str, kind: Kind, default: Value) -> Self {
        Self { key, label, group: "", hint: "", kind, value: default.clone(), default }
    }

    pub fn group(mut self, group: &'static str) -> Self {
        self.group = group;
        self
    }

    pub fn hint(mut self, hint: &'static str) -> Self {
        self.hint = hint;
        self
    }

    /// A value from config.toml, checked against the kind (numbers are clamped to the range).
    fn read(&self, v: &toml::Value) -> Option<Value> {
        match (self.kind, v) {
            (Kind::Bool, toml::Value::Boolean(b)) => Some(Value::Bool(*b)),
            (Kind::Number { min, max, .. }, toml::Value::Integer(i)) => Some(Value::Number((*i as f64).clamp(min, max))),
            (Kind::Number { min, max, .. }, toml::Value::Float(x)) if x.is_finite() => Some(Value::Number(x.clamp(min, max))),
            (Kind::Color, toml::Value::String(s)) => crate::settings::parse_hex(s).map(Value::Color),
            (Kind::Choice(options), toml::Value::String(s)) => options.iter().position(|(k, _)| k == s).map(Value::Choice),
            (Kind::Timeframes, toml::Value::Array(a)) => {
                let picked: Option<Vec<Timeframe>> = a.iter().map(|v| v.as_str().and_then(Timeframe::parse)).collect();
                picked.map(|p| Value::Timeframes(Timeframe::ALL.into_iter().filter(|t| p.contains(t)).collect()))
            }
            _ => None,
        }
    }

    fn write(&self) -> toml::Value {
        match (&self.value, self.kind) {
            (Value::Bool(b), _) => toml::Value::Boolean(*b),
            (Value::Number(x), Kind::Number { decimals: 0, .. }) => toml::Value::Integer(x.round() as i64),
            (Value::Number(x), _) => toml::Value::Float(*x),
            (Value::Color(c), _) => toml::Value::String(crate::settings::color_hex(*c)),
            (Value::Choice(i), Kind::Choice(options)) => toml::Value::String(options.get(*i).map(|o| o.0).unwrap_or_default().into()),
            (Value::Choice(_), _) => toml::Value::String(String::new()),
            (Value::Timeframes(t), _) => toml::Value::Array(t.iter().map(|t| toml::Value::String(t.label().into())).collect()),
        }
    }
}

/// The options of one indicator: what the right-click editor shows and `[studies.<key>]` stores.
#[derive(Clone, Debug)]
pub struct StudyParams {
    /// Name in config.toml and in the chart's hit tests.
    pub key: &'static str,
    pub name: &'static str,
    /// The editor also shows the `[fibonacci]` options (they keep their own section).
    pub fibonacci: bool,
    pub params: Vec<Param>,
}

#[cfg_attr(not(has_preset), allow(dead_code))] // declared by the preset
impl StudyParams {
    fn find(&self, key: &str) -> Option<&Value> {
        self.params.iter().find(|p| p.key == key).map(|p| &p.value)
    }

    pub fn number(&self, key: &str) -> f64 {
        match self.find(key) {
            Some(Value::Number(x)) => *x,
            _ => f64::NAN,
        }
    }

    pub fn flag(&self, key: &str) -> bool {
        matches!(self.find(key), Some(Value::Bool(true)))
    }

    pub fn color(&self, key: &str) -> Color32 {
        match self.find(key) {
            Some(Value::Color(c)) => *c,
            _ => Color32::WHITE,
        }
    }

    /// Key of the chosen option.
    pub fn choice(&self, key: &str) -> &'static str {
        match self.params.iter().find(|p| p.key == key) {
            Some(Param { kind: Kind::Choice(options), value: Value::Choice(i), .. }) => options.get(*i).map(|o| o.0).unwrap_or(""),
            _ => "",
        }
    }

    pub fn timeframes(&self, key: &str) -> &[Timeframe] {
        match self.find(key) {
            Some(Value::Timeframes(t)) => t,
            _ => &[],
        }
    }

    /// Take the values in `studies` (the `[studies]` table of config.toml); missing or invalid ones go
    /// back to the default. Returns whether anything changed, and what was invalid.
    pub fn load(&mut self, studies: &toml::Table) -> (bool, Vec<String>) {
        let saved = studies.get(self.key).and_then(|v| v.as_table());
        let mut changed = false;
        let mut invalid = Vec::new();
        for p in &mut self.params {
            let value = match saved.and_then(|t| t.get(p.key)) {
                Some(v) => p.read(v).unwrap_or_else(|| {
                    invalid.push(format!("studies.{}.{} = {v}", self.key, p.key));
                    p.default.clone()
                }),
                None => p.default.clone(),
            };
            if value != p.value {
                p.value = value;
                changed = true;
            }
        }
        if let Some(t) = saved {
            for k in t.keys().filter(|k| !self.params.iter().any(|p| p.key == k.as_str())) {
                invalid.push(format!("chave desconhecida: studies.{}.{k}", self.key));
            }
        }
        (changed, invalid)
    }

    /// The options that differ from the default, as stored in `[studies.<key>]`.
    pub fn to_table(&self) -> toml::Table {
        self.params.iter().filter(|p| p.value != p.default).map(|p| (p.key.to_string(), p.write())).collect()
    }

    /// Every option and its value, for `ctl state`.
    pub fn state(&self) -> serde_json::Value {
        let values: serde_json::Map<String, serde_json::Value> = self
            .params
            .iter()
            .map(|p| (p.key.to_string(), serde_json::to_value(p.write()).unwrap_or_default()))
            .collect();
        serde_json::json!({"key": self.key, "name": self.name, "options": values})
    }

    /// The editor: one row per option, grouped. Returns whether something changed.
    pub fn edit_ui(&mut self, ui: &mut egui::Ui) -> bool {
        let mut changed = false;
        let mut grid = 0;
        let params = &mut self.params;
        let mut i = 0;
        while i < params.len() {
            let group = params[i].group;
            if !group.is_empty() {
                ui.add_space(6.0);
                ui.label(RichText::new(group).strong());
            }
            let end = i + params[i..].iter().take_while(|p| p.group == group).count();
            egui::Grid::new((self.key, grid)).num_columns(2).min_col_width(170.0).spacing([16.0, 6.0]).show(ui, |ui| {
                for p in &mut params[i..end] {
                    let label = ui.label(p.label);
                    if !p.hint.is_empty() {
                        label.on_hover_text(p.hint);
                    }
                    changed |= value_ui(ui, p);
                    ui.end_row();
                }
            });
            grid += 1;
            i = end;
        }
        changed
    }
}

fn value_ui(ui: &mut egui::Ui, p: &mut Param) -> bool {
    match (&mut p.value, p.kind) {
        (Value::Bool(b), _) => ui.checkbox(b, "").changed(),
        (Value::Number(x), Kind::Number { min, max, step, decimals }) => ui
            .add(egui::DragValue::new(x).range(min..=max).speed(step).max_decimals(decimals).min_decimals(decimals.min(2)))
            .changed(),
        (Value::Color(c), _) => {
            let mut rgb = [c.r(), c.g(), c.b()];
            let changed = egui::color_picker::color_edit_button_srgb(ui, &mut rgb).changed();
            if changed {
                *c = Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
            }
            changed
        }
        (Value::Choice(i), Kind::Choice(options)) => {
            let mut changed = false;
            egui::ComboBox::from_id_salt(p.key).selected_text(options.get(*i).map(|o| o.1).unwrap_or("")).show_ui(ui, |ui| {
                for (k, (_, label)) in options.iter().enumerate() {
                    changed |= ui.selectable_value(i, k, *label).changed();
                }
            });
            changed
        }
        (Value::Timeframes(t), _) => {
            let mut changed = false;
            ui.horizontal(|ui| {
                for tf in Timeframe::ALL {
                    let mut on = t.contains(&tf);
                    if ui.toggle_value(&mut on, tf.label()).changed() {
                        changed = true;
                        if on {
                            t.push(tf);
                        } else {
                            t.retain(|x| *x != tf);
                        }
                        t.sort_by_key(|x| Timeframe::ALL.iter().position(|a| a == x));
                    }
                }
            });
            changed
        }
        _ => false,
    }
}

/// A piece of the indicators' line in the chart legend: plain colored text, or a filled tag.
#[derive(Clone, Debug, PartialEq)]
pub struct LegendItem {
    pub text: String,
    pub color: Color32,
    pub tag: bool,
}

#[cfg(has_preset)]
pub use crate::preset::Studies;
#[cfg(not(has_preset))]
pub use none::Studies;

/// No preset: nothing to compute, nothing to draw.
#[cfg(not(has_preset))]
mod none {
    use super::{LegendItem, Needs, StudyParams};
    use crate::chart::{MapLevel, Marks, Overlay, Pane, Ribbon, Shading, VolumeBand};
    use crate::history::Need;
    use crate::model::{Store, Timeframe};

    pub struct Studies;

    impl Studies {
        pub fn new(_symbol: &str, _tf: Timeframe, _fibonacci: &crate::settings::Fibonacci, _studies: &toml::Table) -> Self {
            Studies
        }

        /// The editable indicators and their options.
        pub fn params(&self) -> &[StudyParams] {
            &[]
        }

        /// Apply the `[studies]` table. Returns (what they read changed — ask again, invalid values).
        pub fn configure(&mut self, _studies: &toml::Table) -> (bool, Vec<String>) {
            (false, Vec::new())
        }

        pub fn configure_fibonacci(&mut self, _settings: &crate::settings::Fibonacci) -> bool {
            false
        }

        pub fn fibonacci_state(&self) -> serde_json::Value {
            serde_json::json!({"enabled": false, "timeframes": []})
        }

        pub fn is_for(&self, _symbol: &str, _tf: Timeframe) -> bool {
            true
        }

        /// History behind the newest chart bar needed before the first full pass.
        pub fn warmup(_tf: Timeframe) -> Need {
            Need::Since { span: 0, cap: 0 }
        }

        pub fn needs(&self) -> Needs {
            Needs::default()
        }

        /// New history arrived (any series).
        pub fn data_arrived(&mut self) {}

        /// Bring everything up to date. `tick`: price step of the chart symbol. `warm`: the chart covers
        /// `warmup`. `now`: server time. `quote(symbol)`: last (bid, tick time in server seconds).
        #[allow(clippy::too_many_arguments)]
        pub fn update(&mut self, _store: &Store, _digits: u32, _tick: f64, _warm: bool, _now: Option<i64>, _quote: impl Fn(&str) -> Option<(f64, i64)>) {}

        /// Price level height for the per-bar POC once known (the delta is requested with it).
        pub fn delta_row(&self) -> Option<f64> {
            None
        }

        /// One mark per bar over the candles (e.g. POC).
        pub fn marks(&self) -> Option<Marks<'_>> {
            None
        }

        pub fn overlays(&self) -> Vec<Overlay<'_>> {
            Vec::new()
        }

        pub fn map_levels(&self) -> Vec<MapLevel> {
            Vec::new()
        }

        pub fn pane(&self) -> Option<Pane<'_>> {
            None
        }

        /// Replaces the volume band (None: the plain volume).
        pub fn volume(&self) -> Option<VolumeBand<'_>> {
            None
        }

        /// Tint behind the candles, one class per bar.
        pub fn shading(&self) -> Option<Shading<'_>> {
            None
        }

        /// Thin rows at the bottom of the plot.
        pub fn ribbon(&self) -> Option<Ribbon<'_>> {
            None
        }

        /// Line under the chart legend.
        pub fn legend(&self) -> Vec<LegendItem> {
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn study() -> StudyParams {
        StudyParams {
            key: "line",
            name: "Linha",
            fibonacci: false,
            params: vec![
                Param::new("visible", "Mostrar", Kind::Bool, Value::Bool(true)),
                Param::new("width", "Espessura", Kind::Number { min: 1.0, max: 6.0, step: 0.1, decimals: 1 }, Value::Number(2.0)),
                Param::new("window", "Janela", Kind::Number { min: 10.0, max: 500.0, step: 1.0, decimals: 0 }, Value::Number(50.0)),
                Param::new("color", "Cor", Kind::Color, Value::Color(Color32::from_rgb(1, 2, 3))),
                Param::new("mode", "Modo", Kind::Choice(&[("a", "A"), ("b", "B")]), Value::Choice(0)),
                Param::new("tfs", "Timeframes", Kind::Timeframes, Value::Timeframes(vec![Timeframe::M5])),
            ],
        }
    }

    #[test]
    fn options_load_clamp_and_store_only_changes() {
        let mut s = study();
        let table: toml::Table = r##"
            [line]
            visible = false
            width = 99
            window = 80
            color = "#ff0000"
            mode = "b"
            tfs = ["H1", "m5", "M15"]
        "##
        .parse()
        .unwrap();
        let (changed, invalid) = s.load(&table);
        assert!(changed && invalid.is_empty(), "{invalid:?}");
        assert!(!s.flag("visible"));
        assert_eq!(s.number("width"), 6.0, "clamped to the range");
        assert_eq!(s.color("color"), Color32::from_rgb(255, 0, 0));
        assert_eq!(s.choice("mode"), "b");
        assert_eq!(s.timeframes("tfs"), [Timeframe::M5, Timeframe::M15, Timeframe::H1], "ordered like Timeframe::ALL");
        let saved = s.to_table();
        assert_eq!(saved.get("window"), Some(&toml::Value::Integer(80)), "whole numbers stay integers");
        assert_eq!(saved.get("mode"), Some(&toml::Value::String("b".into())));
        // what it writes reads back the same
        let mut again = study();
        again.load(&toml::Table::from_iter([("line".to_string(), toml::Value::Table(saved))]));
        assert_eq!(again.to_table(), s.to_table());
        // bad values and unknown keys are reported and fall back to the default
        let bad: toml::Table = "[line]\nmode = \"z\"\ncolor = 3\nwat = 1".parse().unwrap();
        let (_, invalid) = s.load(&bad);
        assert_eq!(invalid.len(), 3, "{invalid:?}");
        assert_eq!(s.choice("mode"), "a");
        assert!(s.to_table().is_empty(), "back to the defaults");
    }
}
