//! Raw RGB frames out of ffmpeg.

use std::io::{ErrorKind, Read};
use std::path::Path;
use std::process::{ChildStdout, Stdio};

use ascii_painter_core::Size2;
use image::RgbImage;

use crate::ffmpeg::{FfmpegPaths, Process, command};
use crate::probe::Fps;
use crate::{TimeRange, VideoError};

/// Size to decode at: the display size, shrunk to at most twice the sampling resolution so the
/// pipe carries no more pixels than the converter can use. Export and preview both use this, so
/// they see identical pixels.
pub fn decode_size(display: Size2, samples: Size2) -> Size2 {
    let max_w = samples.w.saturating_mul(2).max(1);
    let max_h = samples.h.saturating_mul(2).max(1);
    if display.w <= max_w && display.h <= max_h {
        return display;
    }
    let scale = (max_w as f64 / display.w as f64).min(max_h as f64 / display.h as f64);
    Size2 {
        w: ((display.w as f64 * scale).round() as u32).max(1),
        h: ((display.h as f64 * scale).round() as u32).max(1),
    }
}

fn range_args(range: TimeRange) -> Vec<String> {
    let mut args = Vec::new();
    if let Some(start) = range.start {
        args.extend(["-ss".into(), format!("{start:.3}")]);
    }
    if let Some(end) = range.end {
        args.extend(["-to".into(), format!("{end:.3}")]);
    }
    args
}

pub(crate) fn input_args(input: &Path, range: TimeRange) -> Vec<std::ffi::OsString> {
    let mut args: Vec<std::ffi::OsString> = range_args(range).into_iter().map(Into::into).collect();
    args.push("-i".into());
    args.push(input.into());
    args
}

fn scale_filter(size: Size2) -> String {
    // Autorotation happens before this filter; an explicit size also squares anamorphic pixels.
    format!("scale={}:{}:flags=area,setsar=1", size.w, size.h)
}

pub struct FrameReader {
    process: Process,
    stdout: ChildStdout,
    size: Size2,
}

impl FrameReader {
    pub fn spawn(
        paths: &FfmpegPaths,
        input: &Path,
        range: TimeRange,
        size: Size2,
        fps: Fps,
    ) -> Result<Self, VideoError> {
        Self::spawn_cancellable(paths, input, range, size, fps, &crate::Cancellation::default())
    }

    pub fn spawn_cancellable(
        paths: &FfmpegPaths,
        input: &Path,
        range: TimeRange,
        size: Size2,
        fps: Fps,
        cancel: &crate::Cancellation,
    ) -> Result<Self, VideoError> {
        let mut cmd = command(&paths.ffmpeg);
        cmd.args(input_args(input, range))
            .args(["-map", "0:v:0", "-an", "-sn", "-vf"])
            .arg(scale_filter(size))
            .args(["-fps_mode", "cfr", "-r"])
            .arg(fps.arg())
            .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
            .stdout(Stdio::piped());
        let mut process = Process::spawn_cancellable("ffmpeg", cmd, cancel)?;
        let stdout = process.take_stdout();
        Ok(Self { process, stdout, size })
    }

    pub fn size(&self) -> Size2 {
        self.size
    }

    /// The next frame, or `None` at end of stream.
    pub fn next_frame(&mut self) -> Result<Option<RgbImage>, VideoError> {
        let mut buf = vec![0u8; self.size.area() * 3];
        let mut filled = 0;
        while filled < buf.len() {
            match self.stdout.read(&mut buf[filled..]) {
                Ok(0) => break,
                Ok(n) => filled += n,
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) => return Err(e.into()),
            }
        }
        if filled == 0 {
            return Ok(None);
        }
        if filled < buf.len() {
            return Err(VideoError::Decode(format!("truncated frame: {filled} of {} bytes", buf.len())));
        }
        Ok(RgbImage::from_raw(self.size.w, self.size.h, buf))
    }

    /// Wait for ffmpeg to exit cleanly after the last frame.
    pub fn finish(self) -> Result<(), VideoError> {
        drop(self.stdout);
        self.process.finish()
    }
}

/// One frame at `at` seconds, scaled to `size`.
pub fn extract_frame(paths: &FfmpegPaths, input: &Path, at: f64, size: Size2) -> Result<RgbImage, VideoError> {
    extract_frame_cancellable(paths, input, at, size, &crate::Cancellation::default())
}

pub fn extract_frame_cancellable(
    paths: &FfmpegPaths,
    input: &Path,
    at: f64,
    size: Size2,
    cancel: &crate::Cancellation,
) -> Result<RgbImage, VideoError> {
    let mut cmd = command(&paths.ffmpeg);
    cmd.args(input_args(input, TimeRange { start: Some(at.max(0.0)), end: None }))
        .args(["-map", "0:v:0", "-an", "-sn", "-frames:v", "1", "-vf"])
        .arg(scale_filter(size))
        .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        .stdout(Stdio::piped());
    let mut process = Process::spawn_cancellable("ffmpeg", cmd, cancel)?;
    let stdout = process.take_stdout();
    let mut reader = FrameReader { process, stdout, size };
    let frame = reader.next_frame()?;
    reader.finish()?;
    frame.ok_or_else(|| VideoError::Decode(format!("no frame at {at:.3}s")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_size_caps_at_twice_the_samples_and_keeps_aspect() {
        let display = Size2 { w: 3840, h: 2160 };
        assert_eq!(decode_size(display, Size2 { w: 320, h: 360 }), Size2 { w: 640, h: 360 });
        let small = Size2 { w: 320, h: 240 };
        assert_eq!(decode_size(small, Size2 { w: 320, h: 360 }), small);
    }
}
