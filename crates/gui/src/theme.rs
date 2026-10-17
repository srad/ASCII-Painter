//! The app's look: a single dark theme with one accent colour.

use eframe::egui::{
    self, Button, Color32, CornerRadius, FontId, Frame, Margin, Response, RichText, Sense, Shape, Slider, Stroke,
    StrokeKind, Style, TextStyle, Theme, ThemePreference, Ui, Vec2, epaint::Shadow, style::HandleShape, vec2,
};

/// Vivid violet. Text on it must be `ON_ACCENT`.
pub const ACCENT: Color32 = Color32::from_rgb(124, 77, 255);
const ACCENT_HOVER: Color32 = Color32::from_rgb(146, 104, 255);
const ACCENT_PRESSED: Color32 = Color32::from_rgb(100, 56, 228);
const ACCENT_DEEP: Color32 = Color32::from_rgb(62, 40, 130);
/// The accent as a text colour on the dark surfaces; `ACCENT` itself is too dim to read at small sizes.
const ACCENT_TEXT: Color32 = Color32::from_rgb(176, 148, 255);
const ON_ACCENT: Color32 = Color32::from_rgb(250, 247, 255);
/// Behind the image viewer; darker than every panel so the picture reads as the focus.
pub const CANVAS: Color32 = Color32::from_rgb(8, 8, 13);
const PANEL: Color32 = Color32::from_rgb(15, 15, 24);
const RAISED: Color32 = Color32::from_rgb(21, 21, 33);
const HOVER_SURFACE: Color32 = Color32::from_rgb(30, 30, 46);
const BORDER: Color32 = Color32::from_rgb(38, 38, 58);

const TEXT: Color32 = Color32::from_rgb(232, 232, 240);
const TEXT_MUTED: Color32 = Color32::from_rgb(148, 148, 168);

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
    v.window_corner_radius = CornerRadius::same(14);
    v.menu_corner_radius = CornerRadius::same(12);
    v.window_shadow = Shadow { offset: [0, 12], blur: 32, spread: 0, color: Color32::from_black_alpha(140) };
    v.popup_shadow = Shadow { offset: [0, 6], blur: 18, spread: 0, color: Color32::from_black_alpha(120) };
    v.extreme_bg_color = Color32::from_rgb(11, 11, 18);
    v.faint_bg_color = Color32::from_rgb(18, 18, 28);
    v.code_bg_color = RAISED;
    v.hyperlink_color = ACCENT_TEXT;
    v.warn_fg_color = Color32::from_rgb(255, 184, 77);
    v.error_fg_color = Color32::from_rgb(255, 107, 120);
    // Selected tabs, slider trails, progress bars and text selection all fill with this and draw
    // their text in `selection.stroke`, so the pair has to be accent + dark.
    v.selection.bg_fill = ACCENT;
    v.selection.stroke = Stroke::new(1.0, ON_ACCENT);
    // Hinting snaps glyph outlines to whole pixels, which reads as jagged on thin light-on-dark text.
    v.text_options.font_hinting = false;
    v.slider_trailing_fill = true;
    v.handle_shape = HandleShape::Circle;
    v.indent_has_left_vline = false;
    v.striped = false;

    let radius = CornerRadius::same(10);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = PANEL;
    w.noninteractive.weak_bg_fill = PANEL;
    w.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT_MUTED);
    w.noninteractive.corner_radius = radius;

    w.inactive.weak_bg_fill = RAISED;
    w.inactive.bg_fill = Color32::from_rgb(34, 34, 52);
    w.inactive.bg_stroke = Stroke::new(1.0, BORDER);
    w.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    w.inactive.corner_radius = radius;

    w.hovered.weak_bg_fill = HOVER_SURFACE;
    w.hovered.bg_fill = Color32::from_rgb(44, 44, 66);
    w.hovered.bg_stroke = Stroke::new(1.0, ACCENT);
    w.hovered.fg_stroke = Stroke::new(1.5, Color32::WHITE);
    w.hovered.corner_radius = radius;
    w.hovered.expansion = 0.0;

    // `fg_stroke` here is also egui's "strong text" colour, so it must stay light; a pressed plain
    // button therefore gets a deep purple fill rather than the pastel one.
    w.active.weak_bg_fill = ACCENT_DEEP;
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
    s.item_spacing = Vec2::new(10.0, 10.0);
    s.button_padding = Vec2::new(14.0, 8.0);
    s.interact_size.y = 34.0;
    s.icon_width = 18.0;
    s.icon_width_inner = 10.0;
    s.slider_rail_height = 6.0;
    s.window_margin = Margin::same(18);
    s.menu_margin = Margin::same(10);
    s.indent = 16.0;

    style.text_styles.insert(TextStyle::Heading, FontId::proportional(18.0));
    style.text_styles.insert(TextStyle::Body, FontId::proportional(15.0));
    style.text_styles.insert(TextStyle::Button, FontId::proportional(15.0));
    style.text_styles.insert(TextStyle::Small, FontId::proportional(12.0));
}

/// A large accent-filled button for the action the user came to do. Colours come from widget
/// state, so hover and press respond (an explicit `Button::fill` would not).
pub fn primary_button(ui: &mut Ui, text: &str, min_size: Vec2) -> Response {
    ui.scope(|ui| {
        let radius = CornerRadius::same(12);
        let w = &mut ui.visuals_mut().widgets;
        for (state, fill) in
            [(&mut w.inactive, ACCENT), (&mut w.hovered, ACCENT_HOVER), (&mut w.active, ACCENT_PRESSED)]
        {
            state.weak_bg_fill = fill;
            state.bg_fill = fill;
            state.bg_stroke = Stroke::NONE;
            state.fg_stroke = Stroke::new(1.0, ON_ACCENT);
            state.corner_radius = radius;
        }
        ui.add(Button::new(RichText::new(text).strong().color(ON_ACCENT)).min_size(min_size))
    })
    .inner
}

#[derive(Clone, Copy)]
pub enum Arrow {
    Left,
    Right,
}

/// A square button with a painted triangle. A glyph such as "◀" would come from the fallback font
/// and sit off-centre by that font's baseline offset; a polygon is centred exactly.
pub fn arrow_button(ui: &mut Ui, arrow: Arrow, side: f32) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(side), Sense::click());
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&response);
        ui.painter().rect(rect, visuals.corner_radius, visuals.weak_bg_fill, visuals.bg_stroke, StrokeKind::Inside);
        let half_height = side * 0.19;
        let half_width = side * 0.14;
        // Centre the triangle's bounding box (not its centroid) on the button.
        let (tip_x, base_x) = match arrow {
            Arrow::Left => (-half_width, half_width),
            Arrow::Right => (half_width, -half_width),
        };
        let points = [vec2(tip_x, 0.0), vec2(base_x, -half_height), vec2(base_x, half_height)]
            .map(|offset| rect.center() + offset)
            .to_vec();
        ui.painter().add(Shape::convex_polygon(points, visuals.fg_stroke.color, Stroke::NONE));
    }
    response
}

/// egui sizes a slider's knob from the widget height (`height / 2.5` radius), so at the 34px control
/// height the knob would be 27px across. Sliders get their own, shorter height instead.
const SLIDER_HEIGHT: f32 = 20.0;

pub fn slider(ui: &mut Ui, slider: Slider<'_>) -> Response {
    ui.scope(|ui| {
        ui.spacing_mut().interact_size.y = SLIDER_HEIGHT;
        ui.add(slider)
    })
    .inner
}

/// Frame for one group of settings: a bordered, rounded card one step above the side panel.
pub fn card_frame() -> Frame {
    Frame::NONE
        .fill(RAISED)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(12))
}

/// A section label inside a card: small, uppercase, accent-coloured.
pub fn card_title(title: &str) -> RichText {
    RichText::new(title.to_uppercase()).small().strong().color(ACCENT_TEXT)
}

/// Frame for the toolbar and export bar: one step lighter than the side panel, hairline-bordered.
pub fn bar_frame(style: &Style) -> Frame {
    Frame::side_top_panel(style).fill(RAISED).inner_margin(Margin::symmetric(16, 10))
}

/// Frame for the settings side panel.
pub fn side_frame(style: &Style) -> Frame {
    Frame::side_top_panel(style).fill(PANEL).inner_margin(Margin::symmetric(16, 14))
}

/// Frame for the viewer area: no padding, so the canvas runs edge to edge.
pub fn canvas_frame() -> Frame {
    Frame::NONE.fill(CANVAS)
}
