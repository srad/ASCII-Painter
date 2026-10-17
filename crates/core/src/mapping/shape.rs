//! Structural matching: every cell is compared with every glyph's coverage pattern.
//!
//! A rendered cell is modelled as `p_i ≈ w_i·fg + (1 − w_i)·bg`, where `w_i` is the glyph's
//! coverage at sample `i`. Depending on the colour mode, fg and bg are fixed or solved by least
//! squares per glyph, and the glyph with the smallest residual wins. The same solver serves font
//! glyphs (shape mode) and exact block shapes (half/quadrant modes).

use rayon::prelude::*;

use super::dither::{nearest_level, quantize};
use super::{MapContext, mean_color, previous_cells, sample_size};
use crate::color::Lin;
use crate::glyphs::{Glyph, GlyphSet};
use crate::preprocess::Samples;
use crate::settings::{ColorMode, Dither};
use crate::types::{AsciiFrame, GridSize};

/// Best fit of one glyph to one cell.
#[derive(Clone, Copy)]
struct Fit {
    error: f32,
    fg: Lin,
    bg: Lin,
}

/// Per-cell sums that do not depend on the glyph.
struct CellStats {
    /// Channel-major samples: `planes[c][i]`.
    planes: [Vec<f32>; 3],
    sum: Lin,
    sum_sq: f32,
}

impl CellStats {
    fn new(px: &[Lin]) -> Self {
        let planes = std::array::from_fn(|c| px.iter().map(|p| p[c]).collect::<Vec<f32>>());
        let sum = std::array::from_fn(|c| px.iter().map(|p| p[c]).sum());
        let sum_sq = px.iter().map(|p| p.iter().map(|v| v * v).sum::<f32>()).sum();
        Self { planes, sum, sum_sq }
    }

    /// Residual of the flat (single colour) fit — the cell's own variation.
    fn flat_error(&self, n: f32) -> f32 {
        self.sum_sq - self.sum.iter().map(|s| s * s / n).sum::<f32>()
    }
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn clamp01(c: Lin) -> Lin {
    c.map(|v| v.clamp(0.0, 1.0))
}

/// Σ‖p − w·fg − (1 − w)·bg‖² expanded into precomputed sums.
fn residual(stats: &CellStats, glyph: &Glyph, swp: Lin, fg: Lin, bg: Lin) -> f32 {
    (0..3)
        .map(|c| {
            let sup = stats.sum[c] - swp[c];
            -2.0 * (fg[c] * swp[c] + bg[c] * sup)
                + fg[c] * fg[c] * glyph.sww
                + 2.0 * fg[c] * bg[c] * glyph.swu
                + bg[c] * bg[c] * glyph.suu
        })
        .sum::<f32>()
        + stats.sum_sq
}

fn fit_free(stats: &CellStats, glyph: &Glyph, n: f32) -> Fit {
    let swp: Lin = std::array::from_fn(|c| dot(&glyph.coverage, &stats.planes[c]));
    let det = glyph.sww * glyph.suu - glyph.swu * glyph.swu;
    let (fg, bg) = if det.abs() < 1e-6 {
        let mean = stats.sum.map(|s| s / n);
        (mean, mean)
    } else {
        let fg = std::array::from_fn(|c| (glyph.suu * swp[c] - glyph.swu * (stats.sum[c] - swp[c])) / det);
        let bg = std::array::from_fn(|c| (glyph.sww * (stats.sum[c] - swp[c]) - glyph.swu * swp[c]) / det);
        (clamp01(fg), clamp01(bg))
    };
    Fit { error: residual(stats, glyph, swp, fg, bg), fg, bg }
}

fn fit_fixed_bg(stats: &CellStats, glyph: &Glyph, bg: Lin) -> Fit {
    let swp: Lin = std::array::from_fn(|c| dot(&glyph.coverage, &stats.planes[c]));
    let sw = glyph.sww + glyph.swu;
    let fg =
        if glyph.sww < 1e-6 { bg } else { clamp01(std::array::from_fn(|c| bg[c] + (swp[c] - bg[c] * sw) / glyph.sww)) };
    Fit { error: residual(stats, glyph, swp, fg, bg), fg, bg }
}

/// Weight of the mean-ink term in mono matching. Every glyph is textured, so pure SSD turns
/// smooth gradients into stripes of `|`/`h`; matching the mean as well keeps tone faithful.
const MONO_MEAN_WEIGHT: f32 = 1.0;

/// Mono: compare ink amounts with coverage directly; colours are fixed.
fn fit_mono(ink: &[f32], ink_sq: f32, ink_sum: f32, glyph: &Glyph) -> Fit {
    let n = ink.len() as f32;
    let mean_gap = (ink_sum - (glyph.sww + glyph.swu)) / n;
    let error = ink_sq - 2.0 * dot(ink, &glyph.coverage) + glyph.sww + MONO_MEAN_WEIGHT * n * mean_gap * mean_gap;
    Fit { error, fg: [0.0; 3], bg: [0.0; 3] }
}

pub fn map(samples: &Samples, grid: GridSize, ctx: &MapContext, previous: Option<&AsciiFrame>) -> AsciiFrame {
    let glyphs: &GlyphSet = ctx.glyphs;
    let feature = glyphs.feature;
    let n = feature.area() as f32;
    let mode = ctx.settings.color.mode;
    let bg_fixed = ctx.settings.color.bg.to_linear();
    let model = ctx.ink();
    let prev = previous_cells(previous, grid);
    let hysteresis = ctx.settings.temporal.hysteresis;

    // Mono ink field over the whole sample grid, optionally dithered to on/off for block shapes.
    let ink_field: Option<Vec<f32>> = (mode == ColorMode::Mono).then(|| {
        let field: Vec<f32> = samples.px.par_iter().map(|p| model.amount(*p)).collect();
        match ctx.dither() {
            Dither::None => field,
            dither => {
                let levels = [0.0, 1.0];
                quantize(&field, sample_size(grid, feature), &levels, dither, |_, v| nearest_level(&levels, v))
                    .into_iter()
                    .map(|k| levels[k])
                    .collect()
            }
        }
    });

    let cells = (0..grid.cell_count())
        .into_par_iter()
        .map(|i| {
            let (col, row) = (i as u32 % grid.cols, i as u32 / grid.cols);
            let mut px = Vec::with_capacity(feature.area());
            let mut ink = Vec::with_capacity(feature.area());
            for sy in 0..feature.h {
                for sx in 0..feature.w {
                    let (x, y) = (col * feature.w + sx, row * feature.h + sy);
                    px.push(samples.at(x, y));
                    if let Some(field) = &ink_field {
                        ink.push(field[(y * samples.size.w + x) as usize]);
                    }
                }
            }
            let stats = CellStats::new(&px);
            let ink_sq: f32 = ink.iter().map(|v| v * v).sum();
            let ink_sum: f32 = ink.iter().sum();
            let fit = |g: &Glyph| match mode {
                ColorMode::Mono => fit_mono(&ink, ink_sq, ink_sum, g),
                ColorMode::Foreground => fit_fixed_bg(&stats, g, bg_fixed),
                ColorMode::ForegroundBackground => fit_free(&stats, g, n),
            };
            let fits: Vec<Fit> = glyphs.glyphs.iter().map(fit).collect();
            let mut best = (0..fits.len()).min_by(|&a, &b| fits[a].error.total_cmp(&fits[b].error)).unwrap_or(0);

            // Hysteresis: keep last frame's glyph unless the new one is clearly better.
            if let Some(kept) = prev.and_then(|p| glyphs.index_of(p[i].ch)) {
                let variation = match mode {
                    ColorMode::Mono => ink_sq - ink_sum * ink_sum / n,
                    _ => stats.flat_error(n),
                };
                if fits[kept].error - fits[best].error <= hysteresis * (variation + 0.02 * n) {
                    best = kept;
                }
            }
            let chosen = fits[best];
            let fg = if mode == ColorMode::Foreground && glyphs.glyphs[best].sww < 1e-6 {
                mean_color(&px)
            } else {
                chosen.fg
            };
            ctx.finish((glyphs.glyphs[best].ch, fg, chosen.bg), prev.map(|p| &p[i]))
        })
        .collect();
    AsciiFrame { grid, cells }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glyphs::{BlockKind, block_glyphs, font_glyphs};
    use crate::settings::{MappingMode, RampOrder, Settings};
    use crate::types::{CellSize, Rgb, Size2};

    /// A thin anti-aliased stroke: marked samples are partially covered, as a 1–2 px line is
    /// after downsampling to feature resolution.
    fn draw_cell(feature: Size2, ink: impl Fn(u32, u32) -> bool) -> Samples {
        let px = (0..feature.h)
            .flat_map(|y| (0..feature.w).map(move |x| (x, y)))
            .map(|(x, y)| if ink(x, y) { [0.4; 3] } else { [0.0; 3] })
            .collect();
        Samples { size: feature, px }
    }

    fn settings(mode: ColorMode) -> Settings {
        let mut s = Settings { mapping: MappingMode::Shape, ..Settings::default() };
        s.color.mode = mode;
        s.color.fg = Rgb::WHITE;
        s.color.bg = Rgb::BLACK;
        s.temporal.hysteresis = 0.0;
        s
    }

    fn best_char(set: &GlyphSet, settings: &Settings, samples: &Samples) -> char {
        let ctx = MapContext { settings, glyphs: set, palette: None };
        map(samples, GridSize { cols: 1, rows: 1 }, &ctx, None).cells[0].ch
    }

    #[test]
    fn line_cells_match_line_glyphs() {
        let chars: Vec<char> = (' '..='~').collect();
        let feature = Size2 { w: 4, h: 8 };
        let (set, _) = font_glyphs(&chars, 16.0, feature, RampOrder::ByDensity);
        for mode in [ColorMode::Mono, ColorMode::ForegroundBackground] {
            let s = settings(mode);
            let vertical = draw_cell(feature, |x, _| x == 1 || x == 2);
            assert_eq!(best_char(&set, &s, &vertical), '|', "{mode:?}");
            let horizontal = draw_cell(feature, |_, y| y == 4);
            assert!(['-', '—', '~'].contains(&best_char(&set, &s, &horizontal)), "{mode:?}");
            let diagonal = draw_cell(feature, |x, y| x == 3 - y / 2);
            assert_eq!(best_char(&set, &s, &diagonal), '/', "{mode:?}");
        }
    }

    #[test]
    fn flat_cells_pick_density_ordered_glyphs() {
        let chars: Vec<char> = (' '..='~').collect();
        let feature = Size2 { w: 4, h: 8 };
        let (set, _) = font_glyphs(&chars, 16.0, feature, RampOrder::ByDensity);
        let s = settings(ColorMode::Mono);
        let flat = |v: f32| Samples { size: feature, px: vec![[v; 3]; feature.area()] };
        assert_eq!(best_char(&set, &s, &flat(0.0)), ' ');
        let dark = set.index_of(best_char(&set, &s, &flat(0.05))).unwrap();
        let bright = set.index_of(best_char(&set, &s, &flat(0.6))).unwrap();
        assert!(set.glyphs[dark].density < set.glyphs[bright].density);
    }

    #[test]
    fn half_blocks_take_exact_colours() {
        let (set, _) = block_glyphs(BlockKind::Half, CellSize { w: 8, h: 16 });
        let red = [1.0, 0.0, 0.0];
        let blue = [0.0, 0.0, 1.0];
        let samples = Samples { size: Size2 { w: 1, h: 2 }, px: vec![red, blue] };
        let s = settings(ColorMode::ForegroundBackground);
        let ctx = MapContext { settings: &s, glyphs: &set, palette: None };
        let cell = map(&samples, GridSize { cols: 1, rows: 1 }, &ctx, None).cells[0];
        let (top, bottom) = match cell.ch {
            '▀' => (cell.fg, cell.bg),
            '▄' => (cell.bg, cell.fg),
            other => panic!("unexpected {other}"),
        };
        assert_eq!(top, Rgb::new(255, 0, 0));
        assert_eq!(bottom, Rgb::new(0, 0, 255));
    }
}
