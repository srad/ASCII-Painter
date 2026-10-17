//! The public entry point: settings in, frames and pixels out.

use image::{RgbImage, RgbaImage};

use crate::CoreError;
use crate::color::{ANSI16, Palette, ansi256};
use crate::glyphs::{
    Bitmaps, BlockKind, GlyphSet, block_glyphs, braille_bitmaps, font_cell, font_glyphs, validate_charset,
};
use crate::mapping::{self, MapContext};
use crate::preprocess::{Key, prepare};
use crate::render::raster::Background;
use crate::render::{ansi, raster, text};
use crate::settings::{MappingMode, PaletteKind, RampOrder, Settings};
use crate::types::{AsciiFrame, CellSize, GridSize, Size2};

/// Samples per cell used by each mapping mode.
fn feature_for(mode: MappingMode) -> Size2 {
    match mode {
        MappingMode::Ramp => Size2 { w: 2, h: 4 },
        MappingMode::Shape => Size2 { w: 4, h: 8 },
        MappingMode::HalfBlock => Size2 { w: 1, h: 2 },
        MappingMode::Quadrant => Size2 { w: 2, h: 2 },
        MappingMode::Braille => mapping::braille::FEATURE,
    }
}

pub struct ConvertInput<'a> {
    pub image: &'a RgbImage,
    /// Usually [`Converter::grid_for`] of the source's display size.
    pub grid: GridSize,
    /// The previous video frame, for temporal stability.
    pub previous: Option<&'a AsciiFrame>,
}

/// Matching glyphs and render bitmaps for the settings' mapping mode at `font_px`.
fn glyph_data(settings: &Settings, chars: &[char], font_px: f32) -> (GlyphSet, Bitmaps) {
    let feature = feature_for(settings.mapping);
    let cell = font_cell(font_px);
    match settings.mapping {
        MappingMode::Ramp => font_glyphs(chars, font_px, feature, settings.charset.order),
        MappingMode::Shape => font_glyphs(chars, font_px, feature, RampOrder::ByDensity),
        MappingMode::HalfBlock => block_glyphs(BlockKind::Half, cell),
        MappingMode::Quadrant => block_glyphs(BlockKind::Quadrant, cell),
        MappingMode::Braille => (GlyphSet::empty(feature), braille_bitmaps(cell)),
    }
}

pub struct Converter {
    settings: Settings,
    cell: CellSize,
    /// Validated charset (empty for block and braille modes).
    chars: Vec<char>,
    glyphs: GlyphSet,
    bitmaps: Bitmaps,
    palette: Option<Palette>,
    rejected: Vec<char>,
}

impl Converter {
    pub fn new(settings: Settings) -> Result<Self, CoreError> {
        let settings = settings.sanitized();
        let px = settings.render.font_px;
        let mut rejected = Vec::new();
        let mut chars = Vec::new();
        if matches!(settings.mapping, MappingMode::Ramp | MappingMode::Shape) {
            let check = validate_charset(&settings.charset.chars());
            if check.accepted.is_empty() {
                return Err(CoreError::EmptyCharset);
            }
            (chars, rejected) = (check.accepted, check.rejected);
        }
        let (glyphs, bitmaps) = glyph_data(&settings, &chars, px);
        let cell = bitmaps.cell;
        let palette = match settings.color.palette {
            PaletteKind::TrueColor => None,
            PaletteKind::Ansi256 => Palette::new(ansi256()),
            PaletteKind::Ansi16 => Palette::new(ANSI16.to_vec()),
            PaletteKind::Custom => Palette::new(settings.color.custom_palette.clone()),
        };
        Ok(Self { settings, cell, chars, glyphs, bitmaps, palette, rejected })
    }

    /// Render with glyphs rasterised at a different font size — same characters, smaller
    /// pixels. Used for previews whose full-size render would exceed texture limits.
    pub fn render_at(&self, frame: &AsciiFrame, font_px: f32) -> RgbaImage {
        if font_px == self.settings.render.font_px {
            return self.render(frame);
        }
        let (_, bitmaps) = glyph_data(&self.settings, &self.chars, font_px);
        raster::render(frame, &bitmaps, self.background())
    }

    fn background(&self) -> Background {
        if self.settings.transparent() { Background::Transparent } else { Background::Opaque }
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// Charset characters dropped as unusable (wide, control or missing from the font).
    pub fn rejected_chars(&self) -> &[char] {
        &self.rejected
    }

    pub fn cell_size(&self) -> CellSize {
        self.cell
    }

    /// Grid for a source of the given display size, keeping its aspect ratio.
    pub fn grid_for(&self, source: Size2) -> GridSize {
        let cols = self.settings.grid.columns;
        let aspect = self.settings.grid.cell_aspect_override.unwrap_or(self.cell.w as f32 / self.cell.h as f32);
        let rows = (cols as f32 * source.h as f32 / source.w.max(1) as f32 * aspect).round().max(1.0) as u32;
        GridSize { cols, rows }
    }

    /// Size the source is resampled to before mapping.
    pub fn sample_size(&self, grid: GridSize) -> Size2 {
        mapping::sample_size(grid, self.glyphs.feature)
    }

    /// Pixel size of [`Converter::render`] output.
    pub fn output_size(&self, grid: GridSize) -> Size2 {
        Size2 { w: grid.cols * self.cell.w, h: grid.rows * self.cell.h }
    }

    pub fn convert(&self, input: ConvertInput) -> AsciiFrame {
        let ctx = MapContext { settings: &self.settings, glyphs: &self.glyphs, palette: self.palette.as_ref() };
        let prepared = prepare(
            input.image,
            self.sample_size(input.grid),
            &self.settings.tone,
            ctx.ink().direction,
            Key::from_settings(&self.settings.color),
            input.previous.and_then(|p| p.auto_levels),
        );
        let map = match self.settings.mapping {
            MappingMode::Ramp => mapping::ramp::map,
            MappingMode::Shape | MappingMode::HalfBlock | MappingMode::Quadrant => mapping::shape::map,
            MappingMode::Braille => mapping::braille::map,
        };
        AsciiFrame { auto_levels: prepared.levels, ..map(&prepared.samples, input.grid, &ctx, input.previous) }
    }

    /// Pixels of `frame`; transparent where the settings ask for a transparent background.
    pub fn render(&self, frame: &AsciiFrame) -> RgbaImage {
        raster::render(frame, &self.bitmaps, self.background())
    }

    pub fn to_text(&self, frame: &AsciiFrame) -> String {
        text::to_text(frame)
    }

    pub fn to_ansi(&self, frame: &AsciiFrame) -> String {
        ansi::to_ansi(frame, self.settings.color.palette.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presets::builtin;

    fn gradient() -> RgbImage {
        RgbImage::from_fn(64, 48, |x, y| image::Rgb([(x * 4) as u8, (y * 5) as u8, 128]))
    }

    #[test]
    fn every_preset_converts_and_renders_at_the_promised_size() {
        let image = gradient();
        for preset in builtin() {
            let converter = Converter::new(preset.settings).unwrap();
            let grid = converter.grid_for(Size2 { w: 64, h: 48 });
            let frame = converter.convert(ConvertInput { image: &image, grid, previous: None });
            assert_eq!(frame.cells.len(), grid.cell_count(), "{}", preset.name);
            let out = converter.render(&frame);
            let size = converter.output_size(grid);
            assert_eq!(out.dimensions(), (size.w, size.h), "{}", preset.name);
            assert_eq!(converter.to_text(&frame).lines().count(), grid.rows as usize);
        }
    }

    #[test]
    fn rows_follow_source_and_cell_aspect() {
        let mut settings = Settings::default();
        settings.grid.columns = 100;
        settings.grid.cell_aspect_override = Some(0.5);
        let converter = Converter::new(settings).unwrap();
        assert_eq!(converter.grid_for(Size2 { w: 200, h: 100 }), GridSize { cols: 100, rows: 25 });
        assert_eq!(converter.grid_for(Size2 { w: 1000, h: 1 }).rows, 1);
    }

    #[test]
    fn chroma_key_blanks_the_keyed_area_whatever_the_tone() {
        // Left half: a dim grey ramp (the subject); right half: green screen.
        let pixel = |x: u32| if x < 64 { image::Rgb([(100 + x) as u8; 3]) } else { image::Rgb([0, 255, 0]) };
        let image = RgbImage::from_fn(128, 64, |x, _| pixel(x));
        for mode in [crate::settings::ColorMode::Mono, crate::settings::ColorMode::Foreground] {
            let mut settings = Settings::default();
            settings.grid.columns = 32;
            settings.color.mode = mode;
            settings.color.chroma_key = Some(crate::Rgb::new(0, 255, 0));
            settings.tone.contrast = 2.0;
            settings.tone.brightness = 0.3;
            settings.tone.auto_levels = true;
            let converter = Converter::new(settings).unwrap();
            let grid = converter.grid_for(Size2 { w: 128, h: 64 });
            let frame = converter.convert(ConvertInput { image: &image, grid, previous: None });
            for row in 0..grid.rows {
                let cells = frame.row(row);
                assert!(cells[17..].iter().all(|c| c.ch == ' '), "{mode:?} row {row}: keyed area not blank");
            }
            // Auto levels measured the subject alone, so its brightest end reaches the densest glyph.
            assert!(frame.cells.iter().any(|c| c.ch == '@'), "{mode:?}: subject not stretched");
            let levels = frame.auto_levels.unwrap();
            assert!(levels[1] < 0.7, "{mode:?}: green counted in levels {levels:?}");
        }
    }

    #[test]
    fn transparent_background_only_for_fixed_background_modes() {
        let mut settings = Settings::default();
        settings.render.transparent_background = true;
        assert!(settings.transparent());
        settings.color.mode = crate::settings::ColorMode::ForegroundBackground;
        assert!(!settings.transparent());
    }

    #[test]
    fn empty_charset_is_an_error() {
        let mut settings = Settings::default();
        settings.charset.preset = crate::settings::CharsetPreset::Custom;
        settings.charset.custom = "\t漢".into();
        assert!(matches!(Converter::new(settings), Err(CoreError::EmptyCharset)));
    }
}
