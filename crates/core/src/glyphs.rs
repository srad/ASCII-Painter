//! Glyph coverage: rasterised font characters, procedural block/braille shapes and their
//! downsampled feature vectors used for matching.

use std::collections::HashMap;

use ab_glyph::{Font, FontRef, PxScale, ScaleFont, point};
use unicode_width::UnicodeWidthChar;

use crate::resample::resample;
use crate::settings::RampOrder;
use crate::types::{CellSize, Size2};

pub const FONT_BYTES: &[u8] = include_bytes!("../../../assets/DejaVuSansMono.ttf");

fn font() -> FontRef<'static> {
    FontRef::try_from_slice(FONT_BYTES).expect("embedded font is valid")
}

/// One matchable glyph: coverage at feature resolution plus the least-squares terms that only
/// depend on the glyph (see `mapping::shape`).
#[derive(Clone, Debug)]
pub struct Glyph {
    pub ch: char,
    pub coverage: Vec<f32>,
    /// Mean coverage of the full-resolution bitmap.
    pub density: f32,
    /// Position on the brightness ramp, `0..=1`.
    pub level: f32,
    pub sww: f32,
    pub swu: f32,
    pub suu: f32,
}

impl Glyph {
    fn new(ch: char, coverage: Vec<f32>, density: f32) -> Self {
        let n = coverage.len() as f32;
        let sw: f32 = coverage.iter().sum();
        let sww: f32 = coverage.iter().map(|w| w * w).sum();
        Self { ch, coverage, density, level: 0.0, sww, swu: sw - sww, suu: n - 2.0 * sw + sww }
    }
}

#[derive(Clone, Debug)]
pub struct GlyphSet {
    pub feature: Size2,
    /// Sorted by ramp level, lightest first.
    pub glyphs: Vec<Glyph>,
    index: HashMap<char, usize>,
}

impl GlyphSet {
    fn new(feature: Size2, glyphs: Vec<Glyph>) -> Self {
        let index = glyphs.iter().enumerate().map(|(i, g)| (g.ch, i)).collect();
        Self { feature, glyphs, index }
    }

    /// A set with no glyphs, for modes that build characters directly (braille).
    pub fn empty(feature: Size2) -> Self {
        Self::new(feature, Vec::new())
    }

    pub fn index_of(&self, ch: char) -> Option<usize> {
        self.index.get(&ch).copied()
    }

    pub fn max_density(&self) -> f32 {
        self.glyphs.iter().map(|g| g.density).fold(0.0, f32::max)
    }

    pub fn levels(&self) -> Vec<f32> {
        self.glyphs.iter().map(|g| g.level).collect()
    }
}

/// Full-resolution coverage bitmaps used by the raster renderer.
#[derive(Clone, Debug)]
pub struct Bitmaps {
    pub cell: CellSize,
    map: HashMap<char, Vec<f32>>,
}

impl Bitmaps {
    pub fn get(&self, ch: char) -> Option<&[f32]> {
        self.map.get(&ch).map(Vec::as_slice)
    }

    pub fn extend(&mut self, other: Bitmaps) {
        debug_assert_eq!(self.cell, other.cell);
        self.map.extend(other.map);
    }
}

/// Cell size of the embedded font at `font_px`.
pub fn font_cell(font_px: f32) -> CellSize {
    let font = font();
    let scaled = font.as_scaled(PxScale::from(font_px));
    let w = scaled.h_advance(font.glyph_id('M')).round().max(1.0) as u32;
    let h = (scaled.ascent() - scaled.descent()).round().max(1.0) as u32;
    CellSize { w, h }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CharsetCheck {
    pub accepted: Vec<char>,
    /// Characters dropped because they are not single-width or missing from the font.
    pub rejected: Vec<char>,
}

pub fn validate_charset(text: &str) -> CharsetCheck {
    let font = font();
    let mut check = CharsetCheck::default();
    for ch in text.chars() {
        if check.accepted.contains(&ch) || check.rejected.contains(&ch) {
            continue;
        }
        let single_width = ch.width() == Some(1);
        let in_font = ch == ' ' || font.glyph_id(ch).0 != 0;
        if single_width && in_font { check.accepted.push(ch) } else { check.rejected.push(ch) }
    }
    check
}

fn feature_of(bitmap: &[f32], cell: CellSize, feature: Size2) -> Vec<f32> {
    let planar: Vec<[f32; 1]> = bitmap.iter().map(|&v| [v]).collect();
    resample(&planar, Size2 { w: cell.w, h: cell.h }, feature).into_iter().map(|[v]| v).collect()
}

fn mean(values: &[f32]) -> f32 {
    values.iter().sum::<f32>() / values.len().max(1) as f32
}

/// Rasterise `chars` (already validated) and build their matching data.
pub fn font_glyphs(chars: &[char], font_px: f32, feature: Size2, order: RampOrder) -> (GlyphSet, Bitmaps) {
    let font = font();
    let scaled = font.as_scaled(PxScale::from(font_px));
    let cell = font_cell(font_px);
    let baseline = scaled.ascent().round();
    let mut map = HashMap::new();
    let mut glyphs = Vec::new();
    for &ch in chars {
        let mut bitmap = vec![0.0f32; (cell.w * cell.h) as usize];
        let id = font.glyph_id(ch);
        let x0 = (cell.w as f32 - scaled.h_advance(id)) / 2.0;
        let glyph = id.with_scale_and_position(PxScale::from(font_px), point(x0, baseline));
        if let Some(outline) = font.outline_glyph(glyph) {
            let bounds = outline.px_bounds();
            outline.draw(|x, y, c| {
                let px = bounds.min.x as i32 + x as i32;
                let py = bounds.min.y as i32 + y as i32;
                if px >= 0 && py >= 0 && (px as u32) < cell.w && (py as u32) < cell.h {
                    let i = (py as u32 * cell.w + px as u32) as usize;
                    bitmap[i] = bitmap[i].max(c.clamp(0.0, 1.0));
                }
            });
        }
        glyphs.push(Glyph::new(ch, feature_of(&bitmap, cell, feature), mean(&bitmap)));
        map.insert(ch, bitmap);
    }
    assign_levels(&mut glyphs, order);
    (GlyphSet::new(feature, glyphs), Bitmaps { cell, map })
}

fn assign_levels(glyphs: &mut [Glyph], order: RampOrder) {
    if order == RampOrder::ByDensity {
        glyphs.sort_by(|a, b| a.density.total_cmp(&b.density));
    }
    let n = glyphs.len();
    let (lo, hi) = glyphs.iter().fold((f32::MAX, f32::MIN), |(lo, hi), g| (lo.min(g.density), hi.max(g.density)));
    for (i, g) in glyphs.iter_mut().enumerate() {
        g.level = match order {
            RampOrder::ByDensity if hi - lo > 1e-6 => (g.density - lo) / (hi - lo),
            _ if n > 1 => i as f32 / (n - 1) as f32,
            _ => 0.0,
        };
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockKind {
    Half,
    Quadrant,
}

/// Quadrant characters indexed by bitmask: top-left 1, top-right 2, bottom-left 4, bottom-right 8.
pub const QUADRANTS: [char; 16] = [' ', '▘', '▝', '▀', '▖', '▌', '▞', '▛', '▗', '▚', '▐', '▜', '▄', '▙', '▟', '█'];

/// Exact block shapes; their coverage at feature resolution is binary.
pub fn block_glyphs(kind: BlockKind, cell: CellSize) -> (GlyphSet, Bitmaps) {
    let (feature, shapes): (Size2, Vec<(char, Vec<f32>)>) = match kind {
        BlockKind::Half => (
            Size2 { w: 1, h: 2 },
            vec![(' ', vec![0.0, 0.0]), ('▀', vec![1.0, 0.0]), ('▄', vec![0.0, 1.0]), ('█', vec![1.0, 1.0])],
        ),
        BlockKind::Quadrant => (
            Size2 { w: 2, h: 2 },
            QUADRANTS
                .iter()
                .enumerate()
                .map(|(mask, &ch)| (ch, (0..4).map(|bit| ((mask >> bit) & 1) as f32).collect()))
                .collect(),
        ),
    };
    let mut map = HashMap::new();
    let mut glyphs = Vec::new();
    for (ch, coverage) in shapes {
        let bitmap = block_bitmap(&coverage, feature, cell);
        glyphs.push(Glyph::new(ch, coverage.clone(), mean(&bitmap)));
        map.insert(ch, bitmap);
    }
    assign_levels(&mut glyphs, RampOrder::ByDensity);
    (GlyphSet::new(feature, glyphs), Bitmaps { cell, map })
}

fn block_bitmap(coverage: &[f32], feature: Size2, cell: CellSize) -> Vec<f32> {
    let split = |cells: u32, parts: u32, i: u32| (i * cells + parts / 2) / parts;
    let mut bitmap = vec![0.0f32; (cell.w * cell.h) as usize];
    for y in 0..cell.h {
        for x in 0..cell.w {
            let fx = (0..feature.w).rfind(|&i| x >= split(cell.w, feature.w, i)).unwrap_or(0);
            let fy = (0..feature.h).rfind(|&i| y >= split(cell.h, feature.h, i)).unwrap_or(0);
            bitmap[(y * cell.w + x) as usize] = coverage[(fy * feature.w + fx) as usize];
        }
    }
    bitmap
}

/// Braille dot `(col, row)` → bit of the U+2800 offset.
pub fn braille_bit(col: u32, row: u32) -> u32 {
    match (col, row) {
        (0, 3) => 0x40,
        (1, 3) => 0x80,
        (0, r) => 1 << r,
        (_, r) => 1 << (r + 3),
    }
}

pub fn braille_char(bits: u32) -> char {
    char::from_u32(0x2800 + (bits & 0xff)).expect("braille block is valid")
}

/// Anti-aliased round dots for all 256 braille patterns.
pub fn braille_bitmaps(cell: CellSize) -> Bitmaps {
    let (dx, dy) = (cell.w as f32 / 2.0, cell.h as f32 / 4.0);
    let radius = dx.min(dy) * 0.38;
    let dot_cover = |col: u32, row: u32, x: u32, y: u32| {
        let (cx, cy) = ((col as f32 + 0.5) * dx, (row as f32 + 0.5) * dy);
        let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
        (radius + 0.5 - d).clamp(0.0, 1.0)
    };
    let map = (0..256u32)
        .map(|bits| {
            let mut bitmap = vec![0.0f32; (cell.w * cell.h) as usize];
            for col in 0..2 {
                for row in 0..4 {
                    if bits & braille_bit(col, row) == 0 {
                        continue;
                    }
                    for y in 0..cell.h {
                        for x in 0..cell.w {
                            let i = (y * cell.w + x) as usize;
                            bitmap[i] = bitmap[i].max(dot_cover(col, row, x, y));
                        }
                    }
                }
            }
            (braille_char(bits), bitmap)
        })
        .collect();
    Bitmaps { cell, map }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn space_is_lightest_and_ramp_is_monotonic() {
        let chars: Vec<char> = " .:-=+*#%@".chars().collect();
        let (set, _) = font_glyphs(&chars, 16.0, Size2 { w: 4, h: 8 }, RampOrder::ByDensity);
        assert_eq!(set.glyphs[0].ch, ' ');
        assert!(set.glyphs.windows(2).all(|w| w[0].density <= w[1].density && w[0].level <= w[1].level));
        assert_eq!(set.glyphs.last().unwrap().level, 1.0);
    }

    #[test]
    fn charset_validation_rejects_wide_control_and_duplicate_chars() {
        let check = validate_charset("ab a\t漢");
        assert_eq!(check.accepted, vec!['a', 'b', ' ']);
        assert_eq!(check.rejected, vec!['\t', '漢']);
    }

    #[test]
    fn braille_bits_follow_unicode_dot_numbering() {
        assert_eq!(braille_char(braille_bit(0, 0)), '⠁');
        assert_eq!(braille_char(braille_bit(1, 0)), '⠈');
        assert_eq!(braille_char(braille_bit(0, 3)), '⡀');
        assert_eq!(braille_char(braille_bit(1, 3)), '⢀');
        let all = (0..2).flat_map(|c| (0..4).map(move |r| braille_bit(c, r))).fold(0, |a, b| a | b);
        assert_eq!(braille_char(all), '⣿');
    }

    #[test]
    fn quadrant_bitmaps_cover_the_right_corner() {
        let cell = CellSize { w: 10, h: 20 };
        let (set, bitmaps) = block_glyphs(BlockKind::Quadrant, cell);
        assert_eq!(set.glyphs.len(), 16);
        let top_left = bitmaps.get('▘').unwrap();
        assert_eq!(top_left[0], 1.0);
        assert_eq!(top_left[(19 * 10 + 9) as usize], 0.0);
        assert!((mean(bitmaps.get('▚').unwrap()) - 0.5).abs() < 1e-6);
    }
}
