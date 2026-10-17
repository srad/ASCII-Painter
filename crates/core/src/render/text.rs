//! Plain text: characters only, colour is dropped.

use crate::types::AsciiFrame;

/// Separates frames of a video in a `.txt` export.
pub const FRAME_SEPARATOR: char = '\u{c}';

pub fn to_text(frame: &AsciiFrame) -> String {
    let mut out = String::with_capacity(frame.cells.len() + frame.grid.rows as usize);
    for row in 0..frame.grid.rows {
        out.extend(frame.row(row).iter().map(|c| c.ch));
        out.push('\n');
    }
    out
}
