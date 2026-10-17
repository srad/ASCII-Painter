//! Sample grid → character cells.

pub mod braille;
pub mod dither;
pub mod ramp;
pub mod shape;

use crate::color::{Lin, Palette, luminance};
use crate::glyphs::GlyphSet;
use crate::preprocess::{Ink, Samples};
use crate::settings::{ColorMode, Control, Dither, Settings};
use crate::types::{AsciiFrame, Cell, GridSize, Rgb, Size2};

pub struct MapContext<'a> {
    pub settings: &'a Settings,
    /// Glyphs to match; unused by braille.
    pub glyphs: &'a GlyphSet,
    pub palette: Option<&'a Palette>,
}

impl MapContext<'_> {
    pub fn dither(&self) -> Dither {
        if self.settings.relevant(Control::Dither) { self.settings.dither } else { Dither::None }
    }

    pub fn ink(&self) -> InkModel {
        InkModel::new(self.settings)
    }

    /// Final colours: temporal smoothing against the previous frame's cell, then palette.
    fn finish(&self, cell: (char, Lin, Lin), previous: Option<&Cell>) -> Cell {
        let (ch, mut fg, mut bg) = cell;
        let color = &self.settings.color;
        if color.mode == ColorMode::Mono {
            return Cell { ch, fg: color.fg, bg: color.bg };
        }
        let s = self.settings.temporal.color_smoothing;
        if let (Some(prev), true) = (previous, s > 0.0) {
            let mix = |old: Rgb, new: Lin| -> Lin {
                let old = old.to_linear();
                std::array::from_fn(|c| old[c] * s + new[c] * (1.0 - s))
            };
            fg = mix(prev.fg, fg);
            bg = mix(prev.bg, bg);
        }
        let quantize = |c: Lin| self.palette.map_or_else(|| Rgb::from_linear(c), |p| p.nearest(c));
        Cell { ch, fg: quantize(fg), bg: quantize(bg) }
    }
}

/// How sample luminance maps to "ink" (glyph coverage) for modes with a fixed background.
#[derive(Clone, Copy, Debug)]
pub struct InkModel {
    bg: f32,
    span: f32,
    pub direction: Ink,
}

impl InkModel {
    pub fn new(settings: &Settings) -> Self {
        let color = &settings.color;
        let bg = color.bg.luminance();
        match color.mode {
            ColorMode::Mono => {
                let fg = color.fg.luminance();
                let span = fg - bg;
                if span.abs() < 1e-3 {
                    Self { bg: 0.0, span: 1.0, direction: Ink::Light }
                } else {
                    Self { bg, span, direction: if span > 0.0 { Ink::Light } else { Ink::Dark } }
                }
            }
            ColorMode::Foreground if bg >= 0.5 => Self { bg, span: -bg, direction: Ink::Dark },
            ColorMode::Foreground => Self { bg, span: 1.0 - bg, direction: Ink::Light },
            ColorMode::ForegroundBackground => Self { bg: 0.0, span: 1.0, direction: Ink::Light },
        }
    }

    /// Ink amount `0..=1` for a linear colour.
    pub fn amount(&self, c: Lin) -> f32 {
        ((luminance(c) - self.bg) / self.span).clamp(0.0, 1.0)
    }
}

/// Sample-grid size needed for `grid` at `feature` samples per cell.
pub fn sample_size(grid: GridSize, feature: Size2) -> Size2 {
    Size2 { w: grid.cols * feature.w, h: grid.rows * feature.h }
}

/// The samples of one cell, row-major at feature resolution.
pub fn cell_samples(samples: &Samples, feature: Size2, col: u32, row: u32) -> Vec<Lin> {
    let mut out = Vec::with_capacity(feature.area());
    for sy in 0..feature.h {
        for sx in 0..feature.w {
            out.push(samples.at(col * feature.w + sx, row * feature.h + sy));
        }
    }
    out
}

pub fn mean_color(px: &[Lin]) -> Lin {
    let n = px.len().max(1) as f32;
    let mut sum = [0.0f32; 3];
    for p in px {
        for c in 0..3 {
            sum[c] += p[c];
        }
    }
    sum.map(|v| v / n)
}

/// Previous frame's cells, only when they line up with the current grid.
pub fn previous_cells(previous: Option<&AsciiFrame>, grid: GridSize) -> Option<&[Cell]> {
    previous.filter(|p| p.grid == grid).map(|p| p.cells.as_slice())
}
