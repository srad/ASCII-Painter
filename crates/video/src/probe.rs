//! Stream information via ffprobe.

use std::path::Path;
use std::process::Stdio;

use ascii_painter_core::Size2;
use serde::Deserialize;

use crate::VideoError;
use crate::ffmpeg::{FfmpegPaths, Process, command};

/// A frame rate as a rational, passed to ffmpeg verbatim.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fps {
    pub num: u32,
    pub den: u32,
}

impl Fps {
    pub fn parse(text: &str) -> Option<Self> {
        let (num, den) = text.split_once('/').unwrap_or((text, "1"));
        let fps = Self { num: num.trim().parse().ok()?, den: den.trim().parse().ok()? };
        (fps.num > 0 && fps.den > 0).then_some(fps)
    }

    pub fn as_f64(self) -> f64 {
        self.num as f64 / self.den as f64
    }

    pub fn arg(self) -> String {
        format!("{}/{}", self.num, self.den)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MediaInfo {
    /// Upright, square-pixel size (rotation and sample aspect ratio applied).
    pub display: Size2,
    pub fps: Fps,
    pub frames: Option<u64>,
    /// Seconds.
    pub duration: Option<f64>,
    pub has_audio: bool,
    /// PQ/HLG transfer: colours will look washed out without tone mapping.
    pub hdr: bool,
}

impl MediaInfo {
    /// Frames between `start` and `end` seconds (either open), if the length is known.
    pub fn frames_in(&self, range: crate::TimeRange) -> Option<u64> {
        let fps = self.fps.as_f64();
        let full = self.frames.or_else(|| self.duration.map(|d| (d * fps).round() as u64));
        match (range.start, range.end, self.duration) {
            (None, None, _) => full,
            (start, Some(end), _) => Some(((end - start.unwrap_or(0.0)).max(0.0) * fps).round() as u64),
            (Some(start), None, Some(d)) => Some(((d - start).max(0.0) * fps).round() as u64),
            (Some(_), None, None) => None,
        }
    }
}

#[derive(Deserialize)]
struct ProbeOutput {
    #[serde(default)]
    streams: Vec<Stream>,
    format: Option<Format>,
}

#[derive(Deserialize)]
struct Format {
    duration: Option<String>,
}

#[derive(Deserialize, Default)]
struct Disposition {
    #[serde(default)]
    attached_pic: u8,
}

#[derive(Deserialize)]
struct SideData {
    rotation: Option<f64>,
}

#[derive(Deserialize)]
struct Stream {
    codec_type: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    sample_aspect_ratio: Option<String>,
    avg_frame_rate: Option<String>,
    r_frame_rate: Option<String>,
    nb_frames: Option<String>,
    duration: Option<String>,
    color_transfer: Option<String>,
    #[serde(default)]
    disposition: Disposition,
    #[serde(default)]
    side_data_list: Vec<SideData>,
    #[serde(default)]
    tags: std::collections::HashMap<String, String>,
}

pub fn probe(paths: &FfmpegPaths, input: &Path) -> Result<MediaInfo, VideoError> {
    probe_cancellable(paths, input, &crate::Cancellation::default())
}

pub fn probe_cancellable(
    paths: &FfmpegPaths,
    input: &Path,
    cancel: &crate::Cancellation,
) -> Result<MediaInfo, VideoError> {
    let mut cmd = command(&paths.ffprobe);
    cmd.args(["-print_format", "json", "-show_streams", "-show_format"]).arg(input).stdout(Stdio::piped());
    let mut process = Process::spawn_cancellable("ffprobe", cmd, cancel)?;
    let mut json = String::new();
    std::io::Read::read_to_string(&mut process.take_stdout(), &mut json)?;
    process.finish()?;
    parse(&json)
}

fn parse(json: &str) -> Result<MediaInfo, VideoError> {
    let out: ProbeOutput = serde_json::from_str(json).map_err(|e| VideoError::Probe(e.to_string()))?;
    let has_audio = out.streams.iter().any(|s| s.codec_type.as_deref() == Some("audio"));
    let video = out
        .streams
        .into_iter()
        .find(|s| s.codec_type.as_deref() == Some("video") && s.disposition.attached_pic == 0)
        .ok_or_else(|| VideoError::Probe("no video stream".into()))?;
    let (w, h) = match (video.width, video.height) {
        (Some(w), Some(h)) if w > 0 && h > 0 => (w, h),
        _ => return Err(VideoError::Probe("video stream has no size".into())),
    };
    let sar = video
        .sample_aspect_ratio
        .as_deref()
        .and_then(|s| s.split_once(':'))
        .and_then(|(n, d)| Some((n.parse::<f64>().ok()?, d.parse::<f64>().ok()?)))
        .filter(|(n, d)| *n > 0.0 && *d > 0.0)
        .map_or(1.0, |(n, d)| n / d);
    let rotation = video
        .side_data_list
        .iter()
        .find_map(|s| s.rotation)
        .or_else(|| video.tags.get("rotate").and_then(|r| r.parse().ok()))
        .unwrap_or(0.0);
    let display_w = ((w as f64 * sar).round() as u32).max(1);
    let quarter_turn = (rotation.round() as i64).rem_euclid(180) == 90;
    let display = if quarter_turn { Size2 { w: h, h: display_w } } else { Size2 { w: display_w, h } };
    let fps = [video.avg_frame_rate.as_deref(), video.r_frame_rate.as_deref()]
        .into_iter()
        .flatten()
        .find_map(Fps::parse)
        .unwrap_or(Fps { num: 25, den: 1 });
    let duration = [video.duration.as_deref(), out.format.and_then(|f| f.duration).as_deref()]
        .into_iter()
        .flatten()
        .find_map(|d| d.parse::<f64>().ok());
    let hdr = matches!(video.color_transfer.as_deref(), Some("smpte2084" | "arib-std-b67"));
    Ok(MediaInfo {
        display,
        fps,
        frames: video.nb_frames.and_then(|n| n.parse().ok()).filter(|&n| n > 0),
        duration,
        has_audio,
        hdr,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_and_sample_aspect_give_display_size() {
        let json = r#"{"streams":[
            {"codec_type":"video","width":1920,"height":1080,"sample_aspect_ratio":"1:1",
             "avg_frame_rate":"30000/1001","nb_frames":"300","color_transfer":"bt709",
             "side_data_list":[{"side_data_type":"Display Matrix","rotation":-90}]},
            {"codec_type":"audio"}],
            "format":{"duration":"10.01"}}"#;
        let info = parse(json).unwrap();
        assert_eq!(info.display, Size2 { w: 1080, h: 1920 });
        assert_eq!(info.fps, Fps { num: 30000, den: 1001 });
        assert_eq!(info.frames, Some(300));
        assert!(info.has_audio && !info.hdr);

        let anamorphic = r#"{"streams":[{"codec_type":"video","width":720,"height":576,
            "sample_aspect_ratio":"16:11","avg_frame_rate":"0/0","r_frame_rate":"25/1"}]}"#;
        let info = parse(anamorphic).unwrap();
        assert_eq!(info.display, Size2 { w: 1047, h: 576 });
        assert_eq!(info.fps, Fps { num: 25, den: 1 });
        assert_eq!(info.frames, None);
    }

    #[test]
    fn cover_art_is_not_the_video_stream() {
        let json = r#"{"streams":[{"codec_type":"video","width":500,"height":500,"disposition":{"attached_pic":1}}]}"#;
        assert!(parse(json).is_err());
    }
}
