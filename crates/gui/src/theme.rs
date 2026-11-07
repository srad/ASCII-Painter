use rune::{Color, El, TextStyle, col, text};
use rune_kit::{Kit, PaletteSeed, ScrollBarStyle};

pub const PRIMARY_ACTION_HEIGHT: f32 = 40.0;
pub const OPEN_ACTION_WIDTH: f32 = 144.0;
pub const EXPORT_ACTION_WIDTH: f32 = 180.0;

// Linear sRGB constants. Authored colours: #17151e, #211e2a, #100f15, #ac8cff.
pub const PANEL: Color = Color { r: 0.00857, g: 0.00750, b: 0.01298, a: 1.0 };
pub const RAISED: Color = Color { r: 0.01521, g: 0.01298, b: 0.02315, a: 1.0 };
pub const CANVAS: Color = Color { r: 0.00518, g: 0.00478, b: 0.00750, a: 1.0 };
pub const TEXT: Color = Color { r: 0.862, g: 0.839, b: 0.922, a: 1.0 };
pub const MUTED: Color = Color { r: 0.342, g: 0.309, b: 0.418, a: 1.0 };
pub const ACCENT: Color = Color { r: 0.413, g: 0.262, b: 1.0, a: 1.0 };
pub const BORDER: Color = Color { r: 0.043, g: 0.035, b: 0.064, a: 1.0 };
pub const DANGER: Color = Color { r: 1.0, g: 0.22, b: 0.28, a: 1.0 };
pub const SUCCESS: Color = Color { r: 0.18, g: 0.70, b: 0.40, a: 1.0 };

pub fn kit() -> Kit {
    let mut kit = Kit::from_palette(PaletteSeed { bg: PANEL, fg: TEXT, accent: ACCENT });
    kit.c.bg_icon_button_contrast = ACCENT;
    kit.c.submit_fg = PANEL;
    kit.c.submit_icon = PANEL;
    kit
}
pub fn settings_scrollbar_style(kit: &Kit) -> ScrollBarStyle {
    ScrollBarStyle::from_kit(kit)
        .thumb(rgb(ascii_painter_core::Rgb { r: 0x82, g: 0x60, b: 0xae }))
        .thumb_hover(rgb(ascii_painter_core::Rgb { r: 0x9c, g: 0x7a, b: 0xce }))
}
pub const fn body() -> TextStyle {
    TextStyle::new(13.0, 400.0)
}
pub const fn small() -> TextStyle {
    TextStyle::new(11.0, 400.0)
}
pub const fn heading() -> TextStyle {
    TextStyle::new(19.0, 600.0)
}
pub const fn hero() -> TextStyle {
    TextStyle::new(64.0, 600.0).mono()
}
pub fn muted(value: impl Into<String>) -> El {
    text(value).font(small()).color(MUTED)
}
pub fn label(value: impl Into<String>) -> El {
    text(value).font(body()).color(TEXT)
}
pub fn card(title: &str, body: El) -> El {
    col()
        .w_full()
        .gap(12.0)
        .padding(14.0)
        .radius(12.0)
        .fill(RAISED)
        .border(1.0, BORDER)
        .child(text(title).font(TextStyle::new(11.0, 600.0).tracking(1.1)).color(MUTED))
        .child(body)
}
pub fn rgb(rgb: ascii_painter_core::Rgb) -> Color {
    Color {
        r: rune::color::srgb_decode(rgb.r as f32 / 255.0),
        g: rune::color::srgb_decode(rgb.g as f32 / 255.0),
        b: rune::color::srgb_decode(rgb.b as f32 / 255.0),
        a: 1.0,
    }
}
