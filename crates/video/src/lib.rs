//! Video I/O through ffmpeg subprocesses: locating the (bundled) binaries, probing, decoding raw
//! frames, encoding MP4 and running whole exports in the background.

pub mod decode;
pub mod encode;
pub mod ffmpeg;
pub mod output;
pub mod pipeline;
pub mod probe;

pub use encode::EncodeQuality;
pub use ffmpeg::{Cancellation, FfmpegPaths};
pub use output::OutputPolicy;
pub use pipeline::{ExportEvent, ExportJob, ExportRequest, Progress, VideoFormat};
pub use probe::{Fps, MediaInfo, probe};

/// A section of the input in seconds; `None` means from the start / to the end.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TimeRange {
    pub start: Option<f64>,
    pub end: Option<f64>,
}

#[derive(Debug, thiserror::Error)]
pub enum VideoError {
    #[error("{0} was not found next to the application, in the configured folder, or on PATH")]
    NotFound(&'static str),
    #[error("could not start {tool}: {source}")]
    Spawn { tool: &'static str, source: std::io::Error },
    #[error("{tool} failed ({status}): {stderr}")]
    Ffmpeg { tool: &'static str, status: String, stderr: String },
    #[error("could not read media info: {0}")]
    Probe(String),
    #[error("decoding: {0}")]
    Decode(String),
    #[error("encoding: {0}")]
    Encode(String),
    #[error("i/o: {0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Core(#[from] ascii_painter_core::CoreError),
    #[error("the output file is the input file")]
    SameFile,
    #[error("cancelled")]
    Cancelled,
}
