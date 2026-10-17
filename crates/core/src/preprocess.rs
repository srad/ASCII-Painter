//! Source image → linear-light sample grid with tone adjustments applied.

use image::RgbImage;
use rayon::prelude::*;

use crate::color::{Lin, linear_to_srgb, srgb_to_linear, srgb_u8_to_linear};
use crate::resample::resample_rows;
use crate::settings::{Detail, Polarity, ToneSettings};
use crate::types::Size2;

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

pub fn prepare(image: &RgbImage, size: Size2, tone: &ToneSettings, ink: Ink) -> Samples {
    let from = Size2 { w: image.width(), h: image.height() };
    let raw = image.as_raw();
    let stride = from.w as usize * 3;
    let mut px: Vec<Lin> = resample_rows(from, size, |y, row: &mut [Lin]| {
        let src = &raw[y * stride..(y + 1) * stride];
        for (dst, p) in row.iter_mut().zip(src.as_chunks::<3>().0) {
            *dst = [srgb_u8_to_linear(p[0]), srgb_u8_to_linear(p[1]), srgb_u8_to_linear(p[2])];
        }
    });

    if *tone != ToneSettings::default() {
        // Tone is adjusted on perceptual (sRGB-encoded) values, where the sliders feel even.
        px.par_iter_mut().for_each(|p| *p = p.map(linear_to_srgb));
        px.par_iter_mut().for_each(|p| *p = adjust(*p, tone));
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
        px.par_iter_mut().for_each(|p| *p = p.map(|v| srgb_to_linear(v.clamp(0.0, 1.0))));
    }
    Samples { size, px }
}

fn adjust(p: Lin, tone: &ToneSettings) -> Lin {
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
