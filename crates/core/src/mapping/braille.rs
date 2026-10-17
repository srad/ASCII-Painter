//! Braille: each cell holds a 2×4 grid of on/off dots.

use rayon::prelude::*;

use super::dither::{nearest_level, quantize};
use super::{MapContext, cell_samples, mean_color, previous_cells};
use crate::color::{Lin, luminance};
use crate::glyphs::{braille_bit, braille_char};
use crate::preprocess::Samples;
use crate::settings::ColorMode;
use crate::types::{AsciiFrame, GridSize, Size2};

pub const FEATURE: Size2 = Size2 { w: 2, h: 4 };

pub fn map(samples: &Samples, grid: GridSize, ctx: &MapContext, previous: Option<&AsciiFrame>) -> AsciiFrame {
    let mode = ctx.settings.color.mode;
    let bg_fixed = ctx.settings.color.bg.to_linear();
    let prev = previous_cells(previous, grid);

    // Mono and foreground modes decide dots from ink amount over the whole grid (dither-able);
    // with free colours the threshold is each cell's own mean, so detail survives in flat-lit areas.
    let dots: Option<Vec<bool>> = (mode != ColorMode::ForegroundBackground).then(|| {
        let model = ctx.ink();
        let field: Vec<f32> = samples.px.par_iter().map(|p| model.amount(*p)).collect();
        let levels = [0.0, 1.0];
        quantize(&field, samples.size, &levels, ctx.dither(), |_, v| nearest_level(&levels, v))
            .into_iter()
            .map(|k| k == 1)
            .collect()
    });

    let cells = (0..grid.cell_count())
        .into_par_iter()
        .map(|i| {
            let (col, row) = (i as u32 % grid.cols, i as u32 / grid.cols);
            let px = cell_samples(samples, FEATURE, col, row);
            let mean_lum = luminance(mean_color(&px));
            let mut bits = 0u32;
            let (mut on, mut off): (Vec<Lin>, Vec<Lin>) = (Vec::new(), Vec::new());
            for sy in 0..FEATURE.h {
                for sx in 0..FEATURE.w {
                    let p = px[(sy * FEATURE.w + sx) as usize];
                    let lit = match &dots {
                        Some(d) => {
                            let (x, y) = (col * FEATURE.w + sx, row * FEATURE.h + sy);
                            d[(y * samples.size.w + x) as usize]
                        }
                        None => luminance(p) > mean_lum + 1e-3,
                    };
                    if lit {
                        bits |= braille_bit(sx, sy);
                        on.push(p);
                    } else {
                        off.push(p);
                    }
                }
            }
            let all = mean_color(&px);
            let fg = if on.is_empty() { all } else { mean_color(&on) };
            let bg = match mode {
                ColorMode::ForegroundBackground if !off.is_empty() => mean_color(&off),
                ColorMode::ForegroundBackground => all,
                _ => bg_fixed,
            };
            ctx.finish((braille_char(bits), fg, bg), prev.map(|p| &p[i]))
        })
        .collect();
    AsciiFrame { grid, cells, auto_levels: None }
}
