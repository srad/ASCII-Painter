//! ANSI escape output. SGR sequences are emitted only when a colour changes.

use std::fmt::Write;
use std::sync::OnceLock;

use crate::color::{ANSI16, Palette, ansi256};
use crate::settings::PaletteKind;
use crate::types::{AsciiFrame, GridSize, Rgb};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnsiDepth {
    TrueColor,
    Ansi256,
    Ansi16,
}

impl From<PaletteKind> for AnsiDepth {
    fn from(kind: PaletteKind) -> Self {
        match kind {
            PaletteKind::Ansi256 => Self::Ansi256,
            PaletteKind::Ansi16 => Self::Ansi16,
            PaletteKind::TrueColor | PaletteKind::Custom => Self::TrueColor,
        }
    }
}

fn palette256() -> &'static Palette {
    static P: OnceLock<Palette> = OnceLock::new();
    P.get_or_init(|| Palette::new(ansi256()).expect("non-empty"))
}

fn palette16() -> &'static Palette {
    static P: OnceLock<Palette> = OnceLock::new();
    P.get_or_init(|| Palette::new(ANSI16.to_vec()).expect("non-empty"))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Layer {
    Foreground,
    Background,
}

fn sgr(out: &mut String, layer: Layer, color: Rgb, depth: AnsiDepth) {
    let fg = layer == Layer::Foreground;
    match depth {
        AnsiDepth::TrueColor => {
            let _ = write!(out, "{};2;{};{};{}", if fg { 38 } else { 48 }, color.r, color.g, color.b);
        }
        AnsiDepth::Ansi256 => {
            let _ = write!(out, "{};5;{}", if fg { 38 } else { 48 }, palette256().index_of(color));
        }
        AnsiDepth::Ansi16 => {
            let i = palette16().index_of(color) as u32;
            let base = match (fg, i < 8) {
                (true, true) => 30,
                (true, false) => 90 - 8,
                (false, true) => 40,
                (false, false) => 100 - 8,
            };
            let _ = write!(out, "{}", base + i);
        }
    }
}

pub fn to_ansi(frame: &AsciiFrame, depth: AnsiDepth) -> String {
    let mut out = String::with_capacity(frame.cells.len() * 4);
    for row in 0..frame.grid.rows {
        let mut current: Option<(Rgb, Rgb)> = None;
        for cell in frame.row(row) {
            let fg_changed = current.is_none_or(|(fg, _)| fg != cell.fg);
            let bg_changed = current.is_none_or(|(_, bg)| bg != cell.bg);
            if fg_changed || bg_changed {
                out.push_str("\x1b[");
                if fg_changed {
                    sgr(&mut out, Layer::Foreground, cell.fg, depth);
                }
                if bg_changed {
                    if fg_changed {
                        out.push(';');
                    }
                    sgr(&mut out, Layer::Background, cell.bg, depth);
                }
                out.push('m');
                current = Some((cell.fg, cell.bg));
            }
            out.push(cell.ch);
        }
        out.push_str("\x1b[0m\n");
    }
    out
}

/// Video `.ans` files start with this APC header (ignored by terminals); each frame is then
/// prefixed with `ESC[H` (cursor home).
pub const FRAME_PREFIX: &str = "\x1b[H";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VideoHeader {
    pub fps: f64,
    pub grid: GridSize,
}

impl VideoHeader {
    pub fn encode(&self) -> String {
        format!("\x1b_asciipainter;fps={};cols={};rows={}\x1b\\", self.fps, self.grid.cols, self.grid.rows)
    }

    /// Parse a header at the start of `text`; returns it and the byte length it occupies.
    pub fn parse(text: &str) -> Option<(Self, usize)> {
        let body = text.strip_prefix("\x1b_asciipainter;")?;
        let end = body.find("\x1b\\")?;
        let mut fps = None;
        let mut cols = None;
        let mut rows = None;
        for pair in body[..end].split(';') {
            match pair.split_once('=')? {
                ("fps", v) => fps = v.parse().ok(),
                ("cols", v) => cols = v.parse().ok(),
                ("rows", v) => rows = v.parse().ok(),
                _ => {}
            }
        }
        let header = Self { fps: fps?, grid: GridSize { cols: cols?, rows: rows? } };
        Some((header, "\x1b_asciipainter;".len() + end + 2))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Cell;

    #[test]
    fn colour_codes_are_emitted_only_on_change() {
        let (white, black, red) = (Rgb::WHITE, Rgb::BLACK, Rgb::new(255, 0, 0));
        let cells = vec![
            Cell { ch: 'a', fg: white, bg: black },
            Cell { ch: 'b', fg: white, bg: black },
            Cell { ch: 'c', fg: red, bg: black },
        ];
        let frame = AsciiFrame { grid: GridSize { cols: 3, rows: 1 }, cells, auto_levels: None };
        assert_eq!(
            to_ansi(&frame, AnsiDepth::TrueColor),
            "\x1b[38;2;255;255;255;48;2;0;0;0mab\x1b[38;2;255;0;0mc\x1b[0m\n"
        );
        assert_eq!(to_ansi(&frame, AnsiDepth::Ansi16), "\x1b[97;40mab\x1b[91mc\x1b[0m\n");
        assert_eq!(to_ansi(&frame, AnsiDepth::Ansi256), "\x1b[38;5;15;48;5;0mab\x1b[38;5;9mc\x1b[0m\n");
    }

    #[test]
    fn video_header_roundtrips() {
        let header = VideoHeader { fps: 29.97, grid: GridSize { cols: 120, rows: 40 } };
        let text = header.encode() + FRAME_PREFIX;
        let (parsed, len) = VideoHeader::parse(&text).unwrap();
        assert_eq!(parsed, header);
        assert_eq!(&text[len..], FRAME_PREFIX);
    }
}
