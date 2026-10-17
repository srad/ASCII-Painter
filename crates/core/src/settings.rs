//! Every tunable of a conversion. All structs default missing fields so older preset files keep loading.

use serde::{Deserialize, Serialize};

use crate::types::Rgb;

pub const MIN_COLUMNS: u32 = 8;
pub const MAX_COLUMNS: u32 = 2000;

#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub grid: GridSettings,
    pub tone: ToneSettings,
    pub mapping: MappingMode,
    pub charset: CharsetSettings,
    pub dither: Dither,
    pub color: ColorSettings,
    pub temporal: TemporalSettings,
    pub render: RenderSettings,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GridSettings {
    pub columns: u32,
    /// Cell width ÷ height. `None` uses the embedded font's cell, which matches PNG/MP4 output;
    /// set it (≈0.5) when the text will be shown in a terminal with a different font.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cell_aspect_override: Option<f32>,
}

impl Default for GridSettings {
    fn default() -> Self {
        Self { columns: 160, cell_aspect_override: None }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Polarity {
    #[default]
    Normal,
    Inverted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Detail {
    /// The image itself.
    #[default]
    Full,
    /// Only its edges (Sobel magnitude), for line-art looks.
    EdgesOnly,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ToneSettings {
    pub detail: Detail,
    /// Added to perceptual brightness, `-1..=1`.
    pub brightness: f32,
    /// Scales around mid-grey, `0..=3`.
    pub contrast: f32,
    /// Perceptual gamma, `0.2..=5`; values above 1 brighten mid-tones.
    pub gamma: f32,
    /// `0` = greyscale, `1` = unchanged, up to `3`.
    pub saturation: f32,
    /// Unsharp-mask amount, `0..=3`.
    pub sharpen: f32,
    /// Sobel edge emphasis, `0..=2`.
    pub edge_boost: f32,
    pub polarity: Polarity,
    /// Input level mapped to black, `0..=1`.
    pub black_point: f32,
    /// Input level mapped to white, `0..=1`, above `black_point`.
    pub white_point: f32,
    /// Lifts (positive) or deepens (negative) the shadows, `-1..=1`.
    pub shadows: f32,
    /// Lifts (positive) or deepens (negative) the highlights, `-1..=1`.
    pub highlights: f32,
    /// Derive black and white points from the image itself, overriding the manual ones.
    pub auto_levels: bool,
    /// Edge-preserving noise reduction, `0..=1`.
    pub denoise: f32,
}

impl Default for ToneSettings {
    fn default() -> Self {
        Self {
            detail: Detail::Full,
            brightness: 0.0,
            contrast: 1.0,
            gamma: 1.0,
            saturation: 1.0,
            sharpen: 0.0,
            edge_boost: 0.0,
            polarity: Polarity::Normal,
            black_point: 0.0,
            white_point: 1.0,
            shadows: 0.0,
            highlights: 0.0,
            auto_levels: false,
            denoise: 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum MappingMode {
    /// Cell brightness → character density.
    #[default]
    Ramp,
    /// Character whose shape best matches the cell's structure.
    Shape,
    /// `▀`/`▄` blocks: two pixels per cell.
    HalfBlock,
    /// Quadrant blocks: 2×2 pixels per cell.
    Quadrant,
    /// Braille dots: 2×4 pixels per cell.
    Braille,
}

impl MappingMode {
    pub const ALL: [MappingMode; 5] = [Self::Ramp, Self::Shape, Self::HalfBlock, Self::Quadrant, Self::Braille];

    pub fn label(self) -> &'static str {
        match self {
            Self::Ramp => "Brightness ramp",
            Self::Shape => "Shape matching",
            Self::HalfBlock => "Half blocks ▀▄",
            Self::Quadrant => "Quadrant blocks ▚",
            Self::Braille => "Braille ⣿",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum CharsetPreset {
    #[default]
    Standard,
    Detailed,
    Simple,
    Shades,
    Printable,
    Custom,
}

impl CharsetPreset {
    pub const ALL: [CharsetPreset; 6] =
        [Self::Standard, Self::Detailed, Self::Simple, Self::Shades, Self::Printable, Self::Custom];

    pub fn label(self) -> &'static str {
        match self {
            Self::Standard => "Standard",
            Self::Detailed => "Detailed (70)",
            Self::Simple => "Simple",
            Self::Shades => "Shades ░▒▓█",
            Self::Printable => "All printable ASCII",
            Self::Custom => "Custom",
        }
    }

    /// The characters of a built-in set; `None` for [`CharsetPreset::Custom`].
    pub fn chars(self) -> Option<String> {
        Some(match self {
            Self::Standard => " .:-=+*#%@".to_owned(),
            Self::Detailed => " .'`^\",:;Il!i><~+_-?][}{1)(|\\/tfjrxnuvczXYUJCLQ0OZmwqpdbkhao*#MW&8%B@$".to_owned(),
            Self::Simple => " .oO@".to_owned(),
            Self::Shades => " ░▒▓█".to_owned(),
            Self::Printable => (' '..='~').collect(),
            Self::Custom => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum RampOrder {
    /// Sort characters by their measured ink coverage.
    #[default]
    ByDensity,
    /// Keep the written order, lightest ink first.
    AsWritten,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CharsetSettings {
    pub preset: CharsetPreset,
    pub custom: String,
    pub order: RampOrder,
}

impl Default for CharsetSettings {
    fn default() -> Self {
        Self { preset: CharsetPreset::Standard, custom: " .:-=+*#%@".to_owned(), order: RampOrder::ByDensity }
    }
}

impl CharsetSettings {
    pub fn chars(&self) -> String {
        self.preset.chars().unwrap_or_else(|| self.custom.clone())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Dither {
    #[default]
    None,
    FloydSteinberg,
    Atkinson,
    Bayer4,
    Bayer8,
    BlueNoise,
}

impl Dither {
    pub const ALL: [Dither; 6] =
        [Self::None, Self::FloydSteinberg, Self::Atkinson, Self::Bayer4, Self::Bayer8, Self::BlueNoise];

    pub fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::FloydSteinberg => "Floyd–Steinberg",
            Self::Atkinson => "Atkinson",
            Self::Bayer4 => "Bayer 4×4 (video-stable)",
            Self::Bayer8 => "Bayer 8×8 (video-stable)",
            Self::BlueNoise => "Blue noise (video-stable)",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ColorMode {
    /// Fixed foreground and background colours.
    #[default]
    Mono,
    /// Per-cell foreground colour over a fixed background.
    Foreground,
    /// Per-cell foreground and background colours.
    ForegroundBackground,
}

impl ColorMode {
    pub const ALL: [ColorMode; 3] = [Self::Mono, Self::Foreground, Self::ForegroundBackground];

    pub fn label(self) -> &'static str {
        match self {
            Self::Mono => "Mono",
            Self::Foreground => "Colour text",
            Self::ForegroundBackground => "Colour text + background",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PaletteKind {
    #[default]
    TrueColor,
    Ansi256,
    Ansi16,
    Custom,
}

impl PaletteKind {
    pub const ALL: [PaletteKind; 4] = [Self::TrueColor, Self::Ansi256, Self::Ansi16, Self::Custom];

    pub fn label(self) -> &'static str {
        match self {
            Self::TrueColor => "True colour",
            Self::Ansi256 => "ANSI 256",
            Self::Ansi16 => "ANSI 16",
            Self::Custom => "Custom",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ColorSettings {
    pub mode: ColorMode,
    pub fg: Rgb,
    pub bg: Rgb,
    pub palette: PaletteKind,
    pub custom_palette: Vec<Rgb>,
    /// Source colour replaced by the background (green screen); `None` = off.
    pub chroma_key: Option<Rgb>,
    /// How far from `chroma_key` a colour may be and still be keyed, `0..=1`.
    pub key_tolerance: f32,
}

impl Default for ColorSettings {
    fn default() -> Self {
        Self {
            mode: ColorMode::Mono,
            fg: Rgb::new(230, 230, 230),
            bg: Rgb::new(12, 12, 12),
            palette: PaletteKind::TrueColor,
            custom_palette: vec![Rgb::BLACK, Rgb::WHITE],
            chroma_key: None,
            key_tolerance: 0.25,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TemporalSettings {
    /// `0` = off. Higher values keep the previous frame's glyph longer (less flicker).
    pub hysteresis: f32,
    /// `0` = off, up to `0.95`: exponential smoothing of cell colours between frames.
    pub color_smoothing: f32,
}

impl Default for TemporalSettings {
    fn default() -> Self {
        Self { hysteresis: 0.15, color_smoothing: 0.0 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RenderSettings {
    /// Font size of PNG/MP4 output, in pixels.
    pub font_px: f32,
    /// Leave the background transparent in PNG/WebP output (fixed-background colour modes only).
    pub transparent_background: bool,
}

impl Default for RenderSettings {
    fn default() -> Self {
        Self { font_px: 14.0, transparent_background: false }
    }
}

/// A setting whose relevance depends on the other settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    Charset,
    Dither,
    Foreground,
    Background,
    Palette,
    Hysteresis,
    ColorSmoothing,
    TransparentBackground,
}

impl Settings {
    /// Whether a control affects the output under the current settings; the GUI disables it otherwise.
    pub fn relevant(&self, control: Control) -> bool {
        let mode = self.mapping;
        let color = self.color.mode;
        match control {
            Control::Charset => matches!(mode, MappingMode::Ramp | MappingMode::Shape),
            Control::Dither => match mode {
                MappingMode::Ramp | MappingMode::Braille => color != ColorMode::ForegroundBackground,
                MappingMode::HalfBlock | MappingMode::Quadrant => color == ColorMode::Mono,
                MappingMode::Shape => false,
            },
            Control::Foreground => color == ColorMode::Mono,
            Control::Background => color != ColorMode::ForegroundBackground,
            Control::Palette => color != ColorMode::Mono,
            Control::Hysteresis => mode != MappingMode::Braille,
            Control::ColorSmoothing => color != ColorMode::Mono,
            Control::TransparentBackground => color != ColorMode::ForegroundBackground,
        }
    }

    /// Whether rendered pixels leave the background transparent.
    pub fn transparent(&self) -> bool {
        self.render.transparent_background && self.relevant(Control::TransparentBackground)
    }

    /// Clamp every numeric field into its documented range.
    pub fn sanitized(mut self) -> Self {
        self.grid.columns = self.grid.columns.clamp(MIN_COLUMNS, MAX_COLUMNS);
        self.grid.cell_aspect_override = self.grid.cell_aspect_override.map(|a| a.clamp(0.2, 2.0));
        let t = &mut self.tone;
        t.brightness = t.brightness.clamp(-1.0, 1.0);
        t.contrast = t.contrast.clamp(0.0, 3.0);
        t.gamma = t.gamma.clamp(0.2, 5.0);
        t.saturation = t.saturation.clamp(0.0, 3.0);
        t.sharpen = t.sharpen.clamp(0.0, 3.0);
        t.edge_boost = t.edge_boost.clamp(0.0, 2.0);
        t.black_point = t.black_point.clamp(0.0, 0.99);
        t.white_point = t.white_point.clamp(t.black_point + 0.01, 1.0);
        t.shadows = t.shadows.clamp(-1.0, 1.0);
        t.highlights = t.highlights.clamp(-1.0, 1.0);
        t.denoise = t.denoise.clamp(0.0, 1.0);
        self.color.key_tolerance = self.color.key_tolerance.clamp(0.0, 1.0);
        self.temporal.hysteresis = self.temporal.hysteresis.clamp(0.0, 1.0);
        self.temporal.color_smoothing = self.temporal.color_smoothing.clamp(0.0, 0.95);
        self.render.font_px = self.render.font_px.clamp(4.0, 64.0);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_toml_fills_missing_fields_with_defaults() {
        let settings: Settings = toml::from_str("mapping = \"Shape\"\n[grid]\ncolumns = 99\n").unwrap();
        assert_eq!(settings.mapping, MappingMode::Shape);
        assert_eq!(settings.grid.columns, 99);
        assert_eq!(settings.tone, ToneSettings::default());
        assert_eq!(settings.color, ColorSettings::default());
    }

    #[test]
    fn settings_roundtrip_through_toml() {
        let mut settings = Settings::default();
        settings.color.fg = Rgb::new(1, 2, 3);
        settings.grid.cell_aspect_override = Some(0.5);
        settings.color.chroma_key = Some(Rgb::new(0, 255, 0));
        settings.tone.auto_levels = true;
        let text = toml::to_string(&settings).unwrap();
        assert_eq!(toml::from_str::<Settings>(&text).unwrap(), settings);
        settings.color.chroma_key = None;
        let text = toml::to_string(&settings).unwrap();
        assert_eq!(toml::from_str::<Settings>(&text).unwrap(), settings);
    }

    #[test]
    fn sanitized_keeps_white_point_above_black_point() {
        let mut settings = Settings::default();
        settings.tone.black_point = 0.6;
        settings.tone.white_point = 0.4;
        let t = settings.sanitized().tone;
        assert!(t.white_point >= t.black_point + 0.01 - 1e-6, "{t:?}");
    }
}
