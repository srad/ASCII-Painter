//! Source image → linear-light sample grid with tone adjustments applied.

use image::RgbImage;
use rayon::prelude::*;

use crate::color::{Lin, linear_to_srgb, srgb_to_linear, srgb_u8_to_linear};
use crate::resample::resample_rows;
use crate::settings::{ColorSettings, Detail, Polarity, ToneSettings};
use crate::types::{Rgb, Size2};

/// Which way "more ink" moves brightness on the output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ink {
    Light,
    Dark,
}

/// Linear-light samples, row-major.
#[derive(Clone, Debug)]
pub struct Samples {
    pub size: Size2,
    pub px: Vec<Lin>,
}

impl Samples {
    pub fn at(&self, x: u32, y: u32) -> Lin {
        self.px[y as usize * self.size.w as usize + x as usize]
    }
}

/// Chroma key: source pixels near `color` are replaced by the background `bg`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Key {
    pub color: Rgb,
    /// Normalised RGB distance below which a pixel is keyed, `0..=1`.
    pub tolerance: f32,
    pub bg: Rgb,
}

impl Key {
    pub fn from_settings(color: &ColorSettings) -> Option<Self> {
        color.chroma_key.map(|key| Self { color: key, tolerance: color.key_tolerance, bg: color.bg })
    }

    /// How much of a source pixel is keyed away: `0` = kept, `1` = replaced, soft in between.
    fn coverage(&self, p: [u8; 3]) -> f32 {
        const SOFTNESS: f32 = 0.08;
        let k = [self.color.r, self.color.g, self.color.b];
        let d2: f32 = (0..3).map(|c| (p[c] as f32 - k[c] as f32).powi(2)).sum();
        let d = d2.sqrt() / (255.0 * 3f32.sqrt());
        let t = ((d - self.tolerance) / SOFTNESS).clamp(0.0, 1.0);
        1.0 - t * t * (3.0 - 2.0 * t)
    }
}

/// Black and white input levels, perceptual `0..=1`.
pub type Levels = [f32; 2];

pub struct Prepared {
    pub samples: Samples,
    /// The auto levels this frame used, when `auto_levels` is on; feeds the next video frame.
    pub levels: Option<Levels>,
}

/// Weight of the previous frame's auto levels, so they glide instead of pumping.
const LEVELS_RETENTION: f32 = 0.8;

pub fn prepare(
    image: &RgbImage,
    size: Size2,
    tone: &ToneSettings,
    ink: Ink,
    key: Option<Key>,
    previous_levels: Option<Levels>,
) -> Prepared {
    let from = Size2 { w: image.width(), h: image.height() };
    let raw = image.as_raw();
    let stride = from.w as usize * 3;
    let (mut px, mask): (Vec<Lin>, Option<Vec<f32>>) = match key {
        None => {
            let px = resample_rows(from, size, |y, row: &mut [Lin]| {
                let src = &raw[y * stride..(y + 1) * stride];
                for (dst, p) in row.iter_mut().zip(src.as_chunks::<3>().0) {
                    *dst = [srgb_u8_to_linear(p[0]), srgb_u8_to_linear(p[1]), srgb_u8_to_linear(p[2])];
                }
            });
            (px, None)
        }
        Some(key) => {
            // Keyed at source resolution, so subject edges blend with the background, not the key
            // colour; the coverage rides along as a fourth channel.
            let bg = key.bg.to_linear();
            let rgbm = resample_rows(from, size, |y, row: &mut [[f32; 4]]| {
                let src = &raw[y * stride..(y + 1) * stride];
                for (dst, p) in row.iter_mut().zip(src.as_chunks::<3>().0) {
                    let m = key.coverage(*p);
                    let c = p.map(srgb_u8_to_linear);
                    *dst = [c[0] + (bg[0] - c[0]) * m, c[1] + (bg[1] - c[1]) * m, c[2] + (bg[2] - c[2]) * m, m];
                }
            });
            let (px, mask) = rgbm.into_iter().map(|[r, g, b, m]| ([r, g, b], m)).unzip();
            (px, Some(mask))
        }
    };

    let mut levels = None;
    if *tone != ToneSettings::default() {
        // Tone is adjusted on perceptual (sRGB-encoded) values, where the sliders feel even.
        px.par_iter_mut().for_each(|p| *p = p.map(linear_to_srgb));
        if tone.denoise > 0.0 {
            px = denoise(&px, size, tone.denoise);
        }
        let range = if tone.auto_levels {
            let current = auto_levels(&px, mask.as_deref());
            let smoothed = previous_levels.map_or(current, |prev| {
                std::array::from_fn(|i| LEVELS_RETENTION * prev[i] + (1.0 - LEVELS_RETENTION) * current[i])
            });
            levels = Some(smoothed);
            smoothed
        } else {
            [tone.black_point, tone.white_point]
        };
        px.par_iter_mut().for_each(|p| *p = adjust(*p, tone, range));
        if tone.sharpen > 0.0 {
            px = sharpen(&px, size, tone.sharpen);
        }
        match tone.detail {
            Detail::EdgesOnly => {
                // Edge strength becomes ink; edge boost acts as gain.
                let gain = 1.0 + 2.0 * tone.edge_boost;
                let edges = sobel(&px, size);
                px.par_iter_mut().zip(edges).for_each(|(p, e)| {
                    let ink_amount = (e * gain).min(1.0);
                    let v = if ink == Ink::Light { ink_amount } else { 1.0 - ink_amount };
                    *p = [v; 3];
                });
            }
            Detail::Full if tone.edge_boost > 0.0 => {
                let sign = if ink == Ink::Light { 1.0 } else { -1.0 };
                let edges = sobel(&px, size);
                px.par_iter_mut().zip(edges).for_each(|(p, e)| {
                    *p = p.map(|v| v + sign * tone.edge_boost * e);
                });
            }
            Detail::Full => {}
        }
        if let (Some(key), Some(mask)) = (key, &mask) {
            // Tone moved the keyed background; put it back exactly, so it maps to "no ink".
            let bg = [key.bg.r, key.bg.g, key.bg.b].map(|c| c as f32 / 255.0);
            px.par_iter_mut().zip(mask).for_each(|(p, &m)| {
                *p = std::array::from_fn(|c| p[c] + (bg[c] - p[c]) * m);
            });
        }
        px.par_iter_mut().for_each(|p| *p = p.map(|v| srgb_to_linear(v.clamp(0.0, 1.0))));
    }
    Prepared { samples: Samples { size, px }, levels }
}

/// 0.5 % and 99.5 % luminance percentiles, ignoring keyed samples; `[0, 1]` when there is too little
/// to measure.
fn auto_levels(px: &[Lin], mask: Option<&[f32]>) -> Levels {
    const BINS: usize = 256;
    let mut histogram = [0u32; BINS];
    let mut count = 0u32;
    for (i, &p) in px.iter().enumerate() {
        if mask.is_some_and(|m| m[i] > 0.5) {
            continue;
        }
        let l = (0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2]).clamp(0.0, 1.0);
        histogram[(l * (BINS - 1) as f32).round() as usize] += 1;
        count += 1;
    }
    if count < 16 {
        return [0.0, 1.0];
    }
    let percentile = |q: f32| {
        let target = ((q * count as f32).ceil() as u32).max(1);
        let mut seen = 0;
        for (bin, &n) in histogram.iter().enumerate() {
            seen += n;
            if seen >= target {
                return bin as f32 / (BINS - 1) as f32;
            }
        }
        1.0
    };
    let (black, white) = (percentile(0.005), percentile(0.995));
    if white - black < 0.02 { [0.0, 1.0] } else { [black, white] }
}

/// Shadows/highlights bumps peaking near 1/3 and 2/3. Each bump's slope lies in `-1..=1` and the
/// two never reach -1 together, so with the 0.95 scale the curve is monotone for every slider value.
fn tone_curve(v: f32, shadows: f32, highlights: f32) -> f32 {
    v + 0.95 * (shadows * v * (1.0 - v) * (1.0 - v) + highlights * v * v * (1.0 - v))
}

fn adjust(p: Lin, tone: &ToneSettings, [black, white]: Levels) -> Lin {
    let p = p.map(|v| tone_curve(((v - black) / (white - black)).clamp(0.0, 1.0), tone.shadows, tone.highlights));
    let luma = 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2];
    p.map(|v| {
        let mut v = luma + (v - luma) * tone.saturation;
        v = (v - 0.5) * tone.contrast + 0.5 + tone.brightness;
        v = v.clamp(0.0, 1.0).powf(1.0 / tone.gamma);
        if tone.polarity == Polarity::Inverted { 1.0 - v } else { v }
    })
}

fn clamped(px: &[Lin], size: Size2, x: i64, y: i64) -> Lin {
    let x = x.clamp(0, size.w as i64 - 1) as usize;
    let y = y.clamp(0, size.h as i64 - 1) as usize;
    px[y * size.w as usize + x]
}

fn sharpen(px: &[Lin], size: Size2, amount: f32) -> Vec<Lin> {
    (0..px.len())
        .into_par_iter()
        .map(|i| {
            let (x, y) = ((i % size.w as usize) as i64, (i / size.w as usize) as i64);
            let mut blur = [0.0f32; 3];
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let n = clamped(px, size, x + dx, y + dy);
                    for c in 0..3 {
                        blur[c] += n[c] / 9.0;
                    }
                }
            }
            std::array::from_fn(|c| px[i][c] + amount * (px[i][c] - blur[c]))
        })
        .collect()
}

/// Bilateral filter: averages similar neighbours only, so noise flattens while edges stay sharp.
fn denoise(px: &[Lin], size: Size2, amount: f32) -> Vec<Lin> {
    const RADIUS: i64 = 2;
    const SPATIAL_SIGMA: f32 = 1.5;
    let side = (2 * RADIUS + 1) as usize;
    let spatial: Vec<f32> = (0..side * side)
        .map(|i| {
            let (dx, dy) = ((i % side) as f32 - RADIUS as f32, (i / side) as f32 - RADIUS as f32);
            (-(dx * dx + dy * dy) / (2.0 * SPATIAL_SIGMA * SPATIAL_SIGMA)).exp()
        })
        .collect();
    let range_sigma = 0.02 + 0.18 * amount;
    let inv_two_range_sq = 1.0 / (2.0 * range_sigma * range_sigma);
    (0..px.len())
        .into_par_iter()
        .map(|i| {
            let (x, y) = ((i % size.w as usize) as i64, (i / size.w as usize) as i64);
            let centre = px[i];
            let (mut sum, mut total) = ([0.0f32; 3], 0.0f32);
            for dy in -RADIUS..=RADIUS {
                for dx in -RADIUS..=RADIUS {
                    let n = clamped(px, size, x + dx, y + dy);
                    let d2: f32 = (0..3).map(|c| (n[c] - centre[c]).powi(2)).sum();
                    let w = spatial[(dy + RADIUS) as usize * side + (dx + RADIUS) as usize]
                        * (-d2 * inv_two_range_sq).exp();
                    for c in 0..3 {
                        sum[c] += n[c] * w;
                    }
                    total += w;
                }
            }
            sum.map(|s| s / total)
        })
        .collect()
}

/// Sobel gradient magnitude of perceptual luminance, roughly `0..=1`.
fn sobel(px: &[Lin], size: Size2) -> Vec<f32> {
    let lum = |p: Lin| 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2];
    (0..px.len())
        .into_par_iter()
        .map(|i| {
            let (x, y) = ((i % size.w as usize) as i64, (i / size.w as usize) as i64);
            let l = |dx: i64, dy: i64| lum(clamped(px, size, x + dx, y + dy));
            let gx = l(1, -1) + 2.0 * l(1, 0) + l(1, 1) - l(-1, -1) - 2.0 * l(-1, 0) - l(-1, 1);
            let gy = l(-1, 1) + 2.0 * l(0, 1) + l(1, 1) - l(-1, -1) - 2.0 * l(0, -1) - l(1, -1);
            ((gx * gx + gy * gy).sqrt() / 4.0).min(1.0)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Horizontal grey ramp from `lo` to `hi` (sRGB bytes), one sample per pixel.
    fn ramp(lo: u8, hi: u8) -> RgbImage {
        RgbImage::from_fn(101, 4, |x, _| {
            let v = lo as f32 + (hi - lo) as f32 * x as f32 / 100.0;
            image::Rgb([v.round() as u8; 3])
        })
    }

    fn perceptual(samples: &Samples) -> Vec<f32> {
        samples.px.iter().map(|p| linear_to_srgb(p[0])).collect()
    }

    fn auto_tone() -> ToneSettings {
        ToneSettings { auto_levels: true, ..ToneSettings::default() }
    }

    #[test]
    fn auto_levels_stretch_a_narrow_ramp_to_full_range() {
        let image = ramp(51, 153);
        let size = Size2 { w: image.width(), h: image.height() };
        let prepared = prepare(&image, size, &auto_tone(), Ink::Light, None, None);
        let v = perceptual(&prepared.samples);
        let (min, max) = v.iter().fold((1.0f32, 0.0f32), |(lo, hi), &x| (lo.min(x), hi.max(x)));
        assert!(min < 0.02 && max > 0.98, "range {min}..{max}");
        let [black, white] = prepared.levels.unwrap();
        assert!((black - 0.2).abs() < 0.01 && (white - 0.6).abs() < 0.01, "levels {black}, {white}");
    }

    #[test]
    fn auto_levels_glide_from_the_previous_frame() {
        let image = ramp(51, 153);
        let size = Size2 { w: image.width(), h: image.height() };
        let prepared = prepare(&image, size, &auto_tone(), Ink::Light, None, Some([0.0, 1.0]));
        let [black, white] = prepared.levels.unwrap();
        assert!((black - 0.2 * 0.2).abs() < 0.01, "black {black}");
        assert!((white - (0.8 + 0.2 * 0.6)).abs() < 0.01, "white {white}");
    }

    #[test]
    fn tone_curve_is_monotone_at_every_slider_corner() {
        for shadows in [-1.0, 0.0, 1.0] {
            for highlights in [-1.0, 0.0, 1.0] {
                let mut last = tone_curve(0.0, shadows, highlights);
                for i in 1..=1000 {
                    let v = tone_curve(i as f32 / 1000.0, shadows, highlights);
                    assert!(v >= last, "not monotone at s={shadows} h={highlights} v={i}");
                    last = v;
                }
                assert!(tone_curve(0.0, shadows, highlights).abs() < 1e-6);
                assert!((tone_curve(1.0, shadows, highlights) - 1.0).abs() < 1e-6);
            }
        }
    }

    #[test]
    fn denoise_flattens_noise_but_keeps_edges() {
        let size = Size2 { w: 32, h: 32 };
        let mut state = 12345u32;
        let noisy: Vec<Lin> = (0..size.area())
            .map(|i| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let noise = (state >> 8) as f32 / (1u32 << 24) as f32 * 0.1 - 0.05;
                let base = if i % 32 < 16 { 0.2 } else { 0.8 };
                [base + noise; 3]
            })
            .collect();
        let out = denoise(&noisy, size, 1.0);
        let variance = |px: &[Lin]| {
            let left: Vec<f32> = px.iter().enumerate().filter(|(i, _)| i % 32 < 12).map(|(_, p)| p[0]).collect();
            let mean = left.iter().sum::<f32>() / left.len() as f32;
            left.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / left.len() as f32
        };
        assert!(variance(&out) < variance(&noisy) * 0.5);
        let row = 16 * 32;
        let step = out[row + 16][0] - out[row + 15][0];
        assert!(step > 0.6 * 0.9, "edge contrast {step}");
    }
}
