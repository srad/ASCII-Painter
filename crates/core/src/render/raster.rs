//! Character cells → pixels, blending glyph coverage in linear light.

use image::{Rgba, RgbaImage};
use rayon::prelude::*;

use crate::color::linear_to_srgb_u8;
use crate::glyphs::Bitmaps;
use crate::types::{AsciiFrame, Rgb, Size2};

pub fn output_size(frame: &AsciiFrame, bitmaps: &Bitmaps) -> Size2 {
    Size2 { w: frame.grid.cols * bitmaps.cell.w, h: frame.grid.rows * bitmaps.cell.h }
}

pub fn render(frame: &AsciiFrame, bitmaps: &Bitmaps) -> RgbaImage {
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
                    let px = if a <= 0.0 {
                        c.bg
                    } else if a >= 1.0 {
                        c.fg
                    } else {
                        let mix = |i: usize| linear_to_srgb_u8(fg[i] * a + bg[i] * (1.0 - a));
                        Rgb::new(mix(0), mix(1), mix(2))
                    };
                    let offset = (y * size.w as usize + col * cell.w as usize + x) * 4;
                    out[offset..offset + 4].copy_from_slice(&[px.r, px.g, px.b, 255]);
                }
            }
        }
    });
    image
}

/// Pad right/bottom with `fill` so both sides are even (required by yuv420p video).
pub fn pad_even(image: RgbaImage, fill: Rgb) -> RgbaImage {
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
        };
        let image = render(&frame, &bitmaps);
        assert_eq!(image.dimensions(), (6, 5));
        assert_eq!(image.get_pixel(1, 2).0, [255, 0, 0, 255]);
        assert_eq!(image.get_pixel(4, 2).0, [0, 0, 255, 255]);
        let padded = pad_even(image, blue);
        assert_eq!(padded.dimensions(), (6, 6));
        assert_eq!(padded.get_pixel(0, 5).0, [0, 0, 255, 255]);
    }
}
