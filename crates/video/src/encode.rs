//! Rendered RGBA frames into an H.264/AAC MP4.

use std::io::Write;
use std::path::Path;
use std::process::{ChildStdin, Stdio};

use ascii_painter_core::Size2;
use image::RgbaImage;

use crate::decode::input_args;
use crate::ffmpeg::{FfmpegPaths, Process, command};
use crate::probe::Fps;
use crate::{TimeRange, VideoError};

/// Largest supported output side; H.264 levels and most players stop around here.
pub const MAX_SIDE: u32 = 8192;

/// Where the soundtrack comes from: the source file, trimmed like the video.
#[derive(Clone, Copy, Debug)]
pub struct AudioSource<'a> {
    pub input: &'a Path,
    pub range: TimeRange,
}

pub struct EncodeRequest<'a> {
    pub output: &'a Path,
    /// Must be even in both dimensions (yuv420p).
    pub size: Size2,
    pub fps: Fps,
    pub audio: Option<AudioSource<'a>>,
}

pub struct Encoder {
    process: Process,
    stdin: ChildStdin,
    size: Size2,
}

impl Encoder {
    pub fn spawn(paths: &FfmpegPaths, req: EncodeRequest) -> Result<Self, VideoError> {
        let size = req.size;
        if size.w % 2 == 1 || size.h % 2 == 1 || size.w > MAX_SIDE || size.h > MAX_SIDE {
            return Err(VideoError::Encode(format!("unsupported output size {}×{}", size.w, size.h)));
        }
        let mut cmd = command(&paths.ffmpeg);
        cmd.args(["-y", "-f", "rawvideo", "-pix_fmt", "rgba", "-s"])
            .arg(format!("{}x{}", size.w, size.h))
            .arg("-r")
            .arg(req.fps.arg())
            .args(["-i", "-"]);
        if let Some(audio) = req.audio {
            cmd.args(input_args(audio.input, audio.range)).args(["-map", "0:v", "-map", "1:a:0?"]);
        }
        cmd.args([
            "-c:v",
            "libx264",
            "-preset",
            "medium",
            "-crf",
            "18",
            "-vf",
            "scale=out_color_matrix=bt709:out_range=tv",
            "-pix_fmt",
            "yuv420p",
            "-colorspace",
            "bt709",
            "-color_primaries",
            "bt709",
            "-color_trc",
            "bt709",
            "-color_range",
            "tv",
        ]);
        if req.audio.is_some() {
            // Re-encode: stream copy fails for codecs MP4 cannot hold (Vorbis, PCM, …).
            cmd.args(["-c:a", "aac", "-b:a", "192k", "-shortest"]);
        }
        cmd.args(["-movflags", "+faststart"]).arg(req.output).stdin(Stdio::piped()).stdout(Stdio::null());
        let mut process = Process::spawn("ffmpeg", cmd)?;
        let stdin = process.take_stdin();
        Ok(Self { process, stdin, size })
    }

    pub fn write_frame(&mut self, frame: &RgbaImage) -> Result<(), VideoError> {
        if frame.dimensions() != (self.size.w, self.size.h) {
            return Err(VideoError::Encode(format!(
                "frame is {}×{}, encoder expects {}×{}",
                frame.width(),
                frame.height(),
                self.size.w,
                self.size.h
            )));
        }
        self.stdin.write_all(frame.as_raw())?;
        Ok(())
    }

    /// Close the input and wait for ffmpeg to finalise the file.
    pub fn finish(self) -> Result<(), VideoError> {
        drop(self.stdin);
        self.process.finish()
    }
}
