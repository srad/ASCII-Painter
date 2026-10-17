//! Colour-space helpers: sRGB ↔ linear light, luminance, Oklab and palettes.

use std::sync::OnceLock;

use crate::types::Rgb;

/// Linear-light RGB, each channel in `0.0..=1.0`.
pub type Lin = [f32; 3];

pub fn srgb_to_linear(v: f32) -> f32 {
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

pub fn linear_to_srgb(v: f32) -> f32 {
    let v = v.clamp(0.0, 1.0);
    if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}

fn decode_lut() -> &'static [f32; 256] {
    static LUT: OnceLock<[f32; 256]> = OnceLock::new();
    LUT.get_or_init(|| std::array::from_fn(|i| srgb_to_linear(i as f32 / 255.0)))
}

const ENCODE_STEPS: usize = 4096;

fn encode_lut() -> &'static [u8; ENCODE_STEPS + 1] {
    static LUT: OnceLock<[u8; ENCODE_STEPS + 1]> = OnceLock::new();
    LUT.get_or_init(|| std::array::from_fn(|i| (linear_to_srgb(i as f32 / ENCODE_STEPS as f32) * 255.0).round() as u8))
}

pub fn srgb_u8_to_linear(v: u8) -> f32 {
    decode_lut()[v as usize]
}

pub fn linear_to_srgb_u8(v: f32) -> u8 {
    encode_lut()[(v.clamp(0.0, 1.0) * ENCODE_STEPS as f32).round() as usize]
}

/// Relative luminance (Rec. 709) of a linear colour.
pub fn luminance(c: Lin) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

impl Rgb {
    pub fn to_linear(self) -> Lin {
        [srgb_u8_to_linear(self.r), srgb_u8_to_linear(self.g), srgb_u8_to_linear(self.b)]
    }

    pub fn from_linear(c: Lin) -> Self {
        Rgb::new(linear_to_srgb_u8(c[0]), linear_to_srgb_u8(c[1]), linear_to_srgb_u8(c[2]))
    }

    pub fn luminance(self) -> f32 {
        luminance(self.to_linear())
    }
}

pub fn linear_to_oklab(c: Lin) -> [f32; 3] {
    let l = 0.412_221_46 * c[0] + 0.536_332_55 * c[1] + 0.051_445_995 * c[2];
    let m = 0.211_903_5 * c[0] + 0.680_699_5 * c[1] + 0.107_396_96 * c[2];
    let s = 0.088_302_46 * c[0] + 0.281_718_85 * c[1] + 0.629_978_7 * c[2];
    let (l, m, s) = (l.cbrt(), m.cbrt(), s.cbrt());
    [
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    ]
}

/// The 16 standard xterm colours, in SGR order (30–37, then 90–97).
pub const ANSI16: [Rgb; 16] = [
    Rgb::new(0, 0, 0),
    Rgb::new(205, 0, 0),
    Rgb::new(0, 205, 0),
    Rgb::new(205, 205, 0),
    Rgb::new(0, 0, 238),
    Rgb::new(205, 0, 205),
    Rgb::new(0, 205, 205),
    Rgb::new(229, 229, 229),
    Rgb::new(127, 127, 127),
    Rgb::new(255, 0, 0),
    Rgb::new(0, 255, 0),
    Rgb::new(255, 255, 0),
    Rgb::new(92, 92, 255),
    Rgb::new(255, 0, 255),
    Rgb::new(0, 255, 255),
    Rgb::new(255, 255, 255),
];

/// The xterm 256-colour palette: 16 system colours, a 6×6×6 cube and a 24-step grey ramp.
pub fn ansi256() -> Vec<Rgb> {
    let mut colors = ANSI16.to_vec();
    let level = |v: u32| if v == 0 { 0 } else { (55 + v * 40) as u8 };
    for r in 0..6 {
        for g in 0..6 {
            for b in 0..6 {
                colors.push(Rgb::new(level(r), level(g), level(b)));
            }
        }
    }
    for i in 0..24 {
        let v = (8 + i * 10) as u8;
        colors.push(Rgb::new(v, v, v));
    }
    colors
}

/// A fixed set of colours with nearest-colour lookup in Oklab.
#[derive(Clone, Debug)]
pub struct Palette {
    colors: Vec<Rgb>,
    lab: Vec<[f32; 3]>,
}

impl Palette {
    pub fn new(colors: Vec<Rgb>) -> Option<Self> {
        if colors.is_empty() {
            return None;
        }
        let lab = colors.iter().map(|c| linear_to_oklab(c.to_linear())).collect();
        Some(Self { colors, lab })
    }

    pub fn colors(&self) -> &[Rgb] {
        &self.colors
    }

    pub fn nearest_index(&self, c: Lin) -> usize {
        let target = linear_to_oklab(c);
        let dist = |p: &[f32; 3]| (0..3).map(|i| (p[i] - target[i]).powi(2)).sum::<f32>();
        self.lab.iter().enumerate().min_by(|a, b| dist(a.1).total_cmp(&dist(b.1))).map(|(i, _)| i).unwrap_or(0)
    }

    pub fn nearest(&self, c: Lin) -> Rgb {
        self.colors[self.nearest_index(c)]
    }

    pub fn index_of(&self, c: Rgb) -> usize {
        self.colors.iter().position(|p| *p == c).unwrap_or_else(|| self.nearest_index(c.to_linear()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgb_roundtrip_is_exact_for_every_byte() {
        for v in 0..=255u8 {
            assert_eq!(linear_to_srgb_u8(srgb_u8_to_linear(v)), v);
        }
    }

    #[test]
    fn palette_nearest_picks_closest_colour() {
        let palette = Palette::new(ANSI16.to_vec()).unwrap();
        assert_eq!(palette.nearest(Rgb::new(250, 10, 10).to_linear()), Rgb::new(255, 0, 0));
        assert_eq!(palette.nearest(Rgb::new(20, 20, 20).to_linear()), Rgb::BLACK);
        assert_eq!(ansi256().len(), 256);
    }
}
