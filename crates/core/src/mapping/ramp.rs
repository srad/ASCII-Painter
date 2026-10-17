//! Brightness ramp: each cell becomes the glyph whose ink density matches its brightness.

use rayon::prelude::*;

use super::dither::{nearest_level, quantize};
use super::{MapContext, cell_samples, mean_color, previous_cells};
use crate::color::{Lin, luminance};
use crate::preprocess::Samples;
use crate::settings::ColorMode;
use crate::types::{AsciiFrame, GridSize, Size2};

/// Per-cell target before quantisation.
struct Target {
    ink: f32,
    fg: Lin,
    bg: Lin,
}

pub fn map(samples: &Samples, grid: GridSize, ctx: &MapContext, previous: Option<&AsciiFrame>) -> AsciiFrame {
    let glyphs = ctx.glyphs;
    let feature = glyphs.feature;
    let ink = ctx.ink();
    let mode = ctx.settings.color.mode;
    let bg_fixed = ctx.settings.color.bg.to_linear();

    let targets: Vec<Target> = (0..grid.cell_count())
        .into_par_iter()
        .map(|i| {
            let px = cell_samples(samples, feature, i as u32 % grid.cols, i as u32 / grid.cols);
            let mean = mean_color(&px);
            match mode {
                ColorMode::Mono => Target { ink: ink.amount(mean), fg: mean, bg: bg_fixed },
                ColorMode::Foreground => Target { ink: ink.amount(mean), fg: mean, bg: bg_fixed },
                ColorMode::ForegroundBackground => split_target(&px, mean),
            }
        })
        .collect();

    let levels = glyphs.levels();
    let step = if levels.len() > 1 { 1.0 / (levels.len() - 1) as f32 } else { 1.0 };
    let prev = previous_cells(previous, grid);
    let slack = ctx.settings.temporal.hysteresis * step;
    let values: Vec<f32> = targets.iter().map(|t| t.ink).collect();
    let picks = quantize(&values, Size2 { w: grid.cols, h: grid.rows }, &levels, ctx.dither(), |i, v| {
        let best = nearest_level(&levels, v);
        let kept = prev.and_then(|p| glyphs.index_of(p[i].ch));
        match kept {
            Some(k) if (levels[k] - v).abs() <= (levels[best] - v).abs() + slack => k,
            _ => best,
        }
    });

    let cells = targets
        .into_iter()
        .zip(picks)
        .enumerate()
        .map(|(i, (t, k))| ctx.finish((glyphs.glyphs[k].ch, t.fg, t.bg), prev.map(|p| &p[i])))
        .collect();
    AsciiFrame { grid, cells }
}

/// Split a cell's samples into lighter and darker halves: their means become fg and bg, and the
/// ink amount is where the cell mean sits between them.
fn split_target(px: &[Lin], mean: Lin) -> Target {
    let mean_lum = luminance(mean);
    let (light, dark): (Vec<Lin>, Vec<Lin>) = px.iter().partition(|p| luminance(**p) >= mean_lum);
    let fg = mean_color(&light);
    let bg = if dark.is_empty() { fg } else { mean_color(&dark) };
    let span = luminance(fg) - luminance(bg);
    let ink = if span > 1e-4 { ((mean_lum - luminance(bg)) / span).clamp(0.0, 1.0) } else { 0.0 };
    Target { ink, fg, bg }
}
