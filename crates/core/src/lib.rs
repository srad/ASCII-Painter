//! Image → ASCII conversion.
//!
//! [`Converter`] turns an RGB image into an [`AsciiFrame`] (characters plus colours) and renders
//! that frame as pixels, plain text or ANSI escapes. Everything here is pure computation; video
//! I/O lives in `ascii-painter-video`.

pub mod color;
mod converter;
pub mod glyphs;
pub mod load;
pub mod logging;
pub mod mapping;
pub mod preprocess;
pub mod presets;
pub mod render;
pub mod resample;
pub mod settings;
pub mod types;

pub use converter::{ConvertInput, Converter};
pub use settings::Settings;
pub use types::{AsciiFrame, Cell, CellSize, GridSize, Rgb, Size2};

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("image: {0}")]
    Image(#[from] image::ImageError),
    #[error("i/o: {0}")]
    Io(#[from] std::io::Error),
    #[error("preset file: {0}")]
    TomlRead(#[from] toml::de::Error),
    #[error("preset file: {0}")]
    TomlWrite(#[from] toml::ser::Error),
    #[error("the charset has no usable characters")]
    EmptyCharset,
}
