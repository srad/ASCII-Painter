//! Character cells → pixels, blending glyph coverage in linear light.

use image::{Rgba, RgbaImage};
use rayon::prelude::*;

use crate::color::{linear_to_srgb_u8, srgb_u8_to_linear};
use crate::glyphs::Bitmaps;
use crate::types::{AsciiFrame, Rgb, Size2};

pub fn output_size(frame: &AsciiFrame, bitmaps: &Bitmaps) -> Size2 {
    Size2 { w: frame.grid.cols * bitmaps.cell.w, h: frame.grid.rows * bitmaps.cell.h }
}

/// What fills the pixels a glyph does not cover.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Background {
    /// The cell's background colour.
    Opaque,
    /// Nothing: glyph pixels carry the foreground colour with their coverage as alpha.
    Transparent,
}

pub fn render(frame: &AsciiFrame, bitmaps: &Bitmaps, background: Background) -> RgbaImage {
    let cell = bitmaps.cell;
    let size = output_size(frame, bitmaps);
    let mut image = RgbaImage::new(size.w, size.h);
    let row_bytes = size.w as usize * 4 * cell.h as usize;
    image.par_chunks_mut(row_bytes).enumerate().for_each(|(row, out)| {
        for (col, c) in frame.row(row as u32).iter().enumerate() {
            let (fg, bg) = (c.fg.to_linear(), c.bg.to_linear());
            let coverage = bitmaps.get(c.ch);
            for y in 0..cell.h as usize {
                for x in 0..cell.w as usize {
                    let a = coverage.map_or(0.0, |cov| cov[y * cell.w as usize + x]);
                    let px = match background {
                        Background::Transparent => [c.fg.r, c.fg.g, c.fg.b, (a.clamp(0.0, 1.0) * 255.0).round() as u8],
                        Background::Opaque if a <= 0.0 => [c.bg.r, c.bg.g, c.bg.b, 255],
                        Background::Opaque if a >= 1.0 => [c.fg.r, c.fg.g, c.fg.b, 255],
                        Background::Opaque => {
                            let mix = |i: usize| linear_to_srgb_u8(fg[i] * a + bg[i] * (1.0 - a));
                            [mix(0), mix(1), mix(2), 255]
                        }
                    };
                    let offset = (y * size.w as usize + col * cell.w as usize + x) * 4;
                    out[offset..offset + 4].copy_from_slice(&px);
                }
            }
        }
    });
    image
}

/// Composite onto `bg` in linear light, as [`render`] blends, for outputs without alpha.
pub fn flatten(mut image: RgbaImage, bg: Rgb) -> RgbaImage {
    let bg_lin = bg.to_linear();
    image.par_chunks_mut(4).for_each(|px| {
        if px[3] == 255 {
            return;
        }
        let a = px[3] as f32 / 255.0;
        for c in 0..3 {
            px[c] = linear_to_srgb_u8(srgb_u8_to_linear(px[c]) * a + bg_lin[c] * (1.0 - a));
        }
        px[3] = 255;
    });
    image
}

/// Flatten any transparency onto `fill`, then pad right/bottom with it so both sides are even
/// (required by yuv420p video).
pub fn pad_even(image: RgbaImage, fill: Rgb) -> RgbaImage {
    let image = flatten(image, fill);
    let (w, h) = image.dimensions();
    if w % 2 == 0 && h % 2 == 0 {
        return image;
    }
    let mut padded = RgbaImage::from_pixel(w + w % 2, h + h % 2, Rgba([fill.r, fill.g, fill.b, 255]));
    image::imageops::replace(&mut padded, &image, 0, 0);
    padded
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glyphs::{BlockKind, block_glyphs};
    use crate::types::{Cell, CellSize, GridSize};

    #[test]
    fn full_block_renders_fg_and_space_renders_bg() {
        let (_, bitmaps) = block_glyphs(BlockKind::Half, CellSize { w: 3, h: 5 });
        let (red, blue) = (Rgb::new(255, 0, 0), Rgb::new(0, 0, 255));
        let frame = AsciiFrame {
            grid: GridSize { cols: 2, rows: 1 },
            cells: vec![Cell { ch: '█', fg: red, bg: blue }, Cell { ch: ' ', fg: red, bg: blue }],
            auto_levels: None,
        };
        let image = render(&frame, &bitmaps, Background::Opaque);
        assert_eq!(image.dimensions(), (6, 5));
        assert_eq!(image.get_pixel(1, 2).0, [255, 0, 0, 255]);
        assert_eq!(image.get_pixel(4, 2).0, [0, 0, 255, 255]);
        let padded = pad_even(image, blue);
        assert_eq!(padded.dimensions(), (6, 6));
        assert_eq!(padded.get_pixel(0, 5).0, [0, 0, 255, 255]);
    }

    #[test]
    fn transparent_render_flattens_back_to_the_opaque_one() {
        let chars = ['█', '.', ' '];
        let bitmaps = crate::glyphs::font_glyphs(&chars, 14.0, Size2 { w: 2, h: 4 }, Default::default()).1;
        let (fg, bg) = (Rgb::new(240, 200, 30), Rgb::new(20, 40, 90));
        let frame = AsciiFrame {
            grid: GridSize { cols: 3, rows: 1 },
            cells: chars.map(|ch| Cell { ch, fg, bg }).to_vec(),
            auto_levels: None,
        };
        let opaque = render(&frame, &bitmaps, Background::Opaque);
        let transparent = render(&frame, &bitmaps, Background::Transparent);
        let alphas: Vec<u8> = transparent.pixels().map(|p| p.0[3]).collect();
        assert!(alphas.contains(&0) && alphas.contains(&255));
        let space_x = 2 * bitmaps.cell.w + bitmaps.cell.w / 2;
        assert_eq!(transparent.get_pixel(space_x, bitmaps.cell.h / 2).0[3], 0);
        let flat = flatten(transparent, bg);
        for (a, b) in flat.pixels().zip(opaque.pixels()) {
            for c in 0..4 {
                assert!(a.0[c].abs_diff(b.0[c]) <= 1, "{a:?} vs {b:?}");
            }
        }
    }
}
