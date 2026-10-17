use serde::{Deserialize, Serialize};

/// An sRGB colour, serialised as `"#rrggbb"`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const BLACK: Rgb = Rgb { r: 0, g: 0, b: 0 };
    pub const WHITE: Rgb = Rgb { r: 255, g: 255, b: 255 };

    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    pub fn to_hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }

    pub fn parse_hex(text: &str) -> Option<Self> {
        let hex = text.trim().strip_prefix('#').unwrap_or(text.trim());
        if hex.len() != 6 || !hex.is_ascii() {
            return None;
        }
        let channel = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
        Some(Self::new(channel(0)?, channel(2)?, channel(4)?))
    }
}

impl TryFrom<String> for Rgb {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Rgb::parse_hex(&value).ok_or_else(|| format!("invalid colour {value:?}, expected #rrggbb"))
    }
}

impl From<Rgb> for String {
    fn from(value: Rgb) -> Self {
        value.to_hex()
    }
}

/// A width × height in pixels or samples.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Size2 {
    pub w: u32,
    pub h: u32,
}

impl Size2 {
    pub fn area(self) -> usize {
        self.w as usize * self.h as usize
    }
}

/// Number of character cells horizontally and vertically.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GridSize {
    pub cols: u32,
    pub rows: u32,
}

impl GridSize {
    pub fn cell_count(self) -> usize {
        self.cols as usize * self.rows as usize
    }
}

/// Pixel size of one rendered character cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CellSize {
    pub w: u32,
    pub h: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    pub ch: char,
    pub fg: Rgb,
    pub bg: Rgb,
}

/// The result of a conversion: one cell per grid position, row-major.
#[derive(Clone, Debug, PartialEq)]
pub struct AsciiFrame {
    pub grid: GridSize,
    pub cells: Vec<Cell>,
    /// Black and white levels chosen by auto levels, carried to the next video frame.
    pub auto_levels: Option<[f32; 2]>,
}

impl AsciiFrame {
    pub fn row(&self, row: u32) -> &[Cell] {
        let cols = self.grid.cols as usize;
        let start = row as usize * cols;
        &self.cells[start..start + cols]
    }
}
