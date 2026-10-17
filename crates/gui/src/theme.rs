//! The app's look: a single dark theme with one accent colour.

use eframe::egui::{
    self, Color32, CornerRadius, FontId, Frame, Margin, Stroke, Style, TextStyle, Theme, ThemePreference, Vec2,
    epaint::Shadow,
};

pub const ACCENT: Color32 = Color32::from_rgb(124, 131, 255);
const ACCENT_DIM: Color32 = Color32::from_rgb(58, 62, 140);
/// Behind the image viewer; darker than every panel so the picture reads as the focus.
pub const CANVAS: Color32 = Color32::from_rgb(10, 10, 13);
const PANEL: Color32 = Color32::from_rgb(20, 21, 26);
const RAISED: Color32 = Color32::from_rgb(27, 28, 35);
const BORDER: Color32 = Color32::from_rgb(44, 46, 56);

const TEXT: Color32 = Color32::from_rgb(222, 224, 232);
const TEXT_MUTED: Color32 = Color32::from_rgb(142, 146, 160);

/// Forces the dark theme (egui would otherwise follow the OS) and installs the app's style.
pub fn apply(ctx: &egui::Context) {
    ctx.set_theme(ThemePreference::Dark);
    ctx.style_mut_of(Theme::Dark, restyle);
}

fn restyle(style: &mut Style) {
    let v = &mut style.visuals;
    v.panel_fill = PANEL;
    v.window_fill = RAISED;
    v.window_stroke = Stroke::new(1.0, BORDER);
    v.window_corner_radius = CornerRadius::same(12);
    v.menu_corner_radius = CornerRadius::same(10);
    v.window_shadow = Shadow { offset: [0, 12], blur: 32, spread: 0, color: Color32::from_black_alpha(140) };
    v.popup_shadow = Shadow { offset: [0, 6], blur: 18, spread: 0, color: Color32::from_black_alpha(120) };
    v.extreme_bg_color = Color32::from_rgb(13, 14, 18);
    v.faint_bg_color = Color32::from_rgb(24, 25, 31);
    v.code_bg_color = RAISED;
    v.hyperlink_color = ACCENT;
    v.warn_fg_color = Color32::from_rgb(255, 184, 77);
    v.error_fg_color = Color32::from_rgb(255, 107, 120);
    v.selection.bg_fill = ACCENT_DIM;
    v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
    v.slider_trailing_fill = true;
    v.indent_has_left_vline = false;
    v.striped = false;

    let radius = CornerRadius::same(8);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = PANEL;
    w.noninteractive.weak_bg_fill = PANEL;
    w.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT_MUTED);
    w.noninteractive.corner_radius = radius;

    w.inactive.weak_bg_fill = RAISED;
    w.inactive.bg_fill = Color32::from_rgb(38, 40, 50);
    w.inactive.bg_stroke = Stroke::new(1.0, BORDER);
    w.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    w.inactive.corner_radius = radius;

    w.hovered.weak_bg_fill = Color32::from_rgb(36, 38, 48);
    w.hovered.bg_fill = Color32::from_rgb(48, 50, 62);
    w.hovered.bg_stroke = Stroke::new(1.0, ACCENT);
    w.hovered.fg_stroke = Stroke::new(1.5, Color32::WHITE);
    w.hovered.corner_radius = radius;
    w.hovered.expansion = 0.0;

    w.active.weak_bg_fill = ACCENT_DIM;
    w.active.bg_fill = ACCENT;
    w.active.bg_stroke = Stroke::new(1.0, ACCENT);
    w.active.fg_stroke = Stroke::new(2.0, Color32::WHITE);
    w.active.corner_radius = radius;
    w.active.expansion = 0.0;

    w.open.weak_bg_fill = RAISED;
    w.open.bg_fill = RAISED;
    w.open.bg_stroke = Stroke::new(1.0, ACCENT);
    w.open.fg_stroke = Stroke::new(1.0, TEXT);
    w.open.corner_radius = radius;

    let s = &mut style.spacing;
    s.item_spacing = Vec2::new(8.0, 7.0);
    s.button_padding = Vec2::new(10.0, 5.0);
    s.interact_size.y = 26.0;
    s.window_margin = Margin::same(16);
    s.menu_margin = Margin::same(8);
    s.indent = 14.0;

    style.text_styles.insert(TextStyle::Heading, FontId::proportional(17.0));
    style.text_styles.insert(TextStyle::Body, FontId::proportional(14.0));
    style.text_styles.insert(TextStyle::Button, FontId::proportional(14.0));
    style.text_styles.insert(TextStyle::Small, FontId::proportional(12.0));
}

/// Frame for the toolbar and export bar: one step lighter than the side panel, hairline-bordered.
pub fn bar_frame(style: &Style) -> Frame {
    Frame::side_top_panel(style).fill(RAISED).inner_margin(Margin::symmetric(14, 8))
}

/// Frame for the settings side panel.
pub fn side_frame(style: &Style) -> Frame {
    Frame::side_top_panel(style).fill(PANEL).inner_margin(Margin::symmetric(14, 12))
}

/// Frame for the viewer area: no padding, so the canvas runs edge to edge.
pub fn canvas_frame() -> Frame {
    Frame::NONE.fill(CANVAS)
}
