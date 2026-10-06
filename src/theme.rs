//! Colors and egui style. One dark theme for now.

use eframe::egui::{self, Color32, CornerRadius, Stroke};

pub struct Palette {
    pub app_bg: Color32,
    pub panel_bg: Color32,
    pub chart_bg: Color32,
    pub grid: Color32,
    pub border: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub crosshair: Color32,
    pub tag_bg: Color32,
    pub accent: Color32,
    pub up: Color32,
    pub down: Color32,
    pub up_vol: Color32,
    pub down_vol: Color32,
    pub ok: Color32,
    pub warn: Color32,
    pub danger: Color32,
}

impl Default for Palette {
    fn default() -> Self {
        let rgb = Color32::from_rgb;
        Self {
            app_bg: rgb(11, 14, 19),
            panel_bg: rgb(15, 19, 26),
            chart_bg: rgb(13, 17, 23),
            grid: rgb(25, 31, 41),
            border: rgb(34, 41, 54),
            text: rgb(214, 220, 230),
            text_dim: rgb(124, 134, 150),
            crosshair: rgb(110, 120, 138),
            tag_bg: rgb(42, 50, 66),
            accent: rgb(59, 130, 246),
            up: rgb(38, 166, 154),
            down: rgb(239, 83, 80),
            up_vol: Color32::from_rgba_unmultiplied(38, 166, 154, 60),
            down_vol: Color32::from_rgba_unmultiplied(239, 83, 80, 60),
            ok: rgb(34, 197, 94),
            warn: rgb(234, 179, 8),
            danger: rgb(239, 68, 68),
        }
    }
}

pub fn apply(ctx: &egui::Context, pal: &Palette) {
    // same look whatever the system theme is
    ctx.all_styles_mut(|style| {
        let v = &mut style.visuals;
        *v = egui::Visuals::dark();
        v.panel_fill = pal.panel_bg;
        v.window_fill = pal.panel_bg;
        v.extreme_bg_color = pal.app_bg;
        v.override_text_color = Some(pal.text);
        v.selection.bg_fill = pal.accent;
        v.selection.stroke = Stroke::new(1.0, pal.text);
        let radius = CornerRadius::same(5);
        for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
            w.corner_radius = radius;
        }
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, pal.border);
        v.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
        v.widgets.inactive.bg_fill = pal.tag_bg;
        v.widgets.hovered.weak_bg_fill = pal.tag_bg;
        v.widgets.hovered.bg_stroke = Stroke::NONE;
        v.widgets.active.weak_bg_fill = pal.border;
        style.spacing.button_padding = egui::vec2(9.0, 4.0);
        style.spacing.item_spacing = egui::vec2(6.0, 6.0);
    });
}
