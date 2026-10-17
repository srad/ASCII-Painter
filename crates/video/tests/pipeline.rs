//! End-to-end tests against real ffmpeg. Skipped (with a message) when ffmpeg is not available.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use ascii_painter_core::render::ansi::VideoHeader;
use ascii_painter_core::render::text::FRAME_SEPARATOR;
use ascii_painter_core::{ConvertInput, Converter, Settings, Size2};
use ascii_painter_video::decode::{FrameReader, decode_size, extract_frame};
use ascii_painter_video::{
    ExportEvent, ExportJob, ExportRequest, FfmpegPaths, TimeRange, VideoError, VideoFormat, probe,
};

fn tools() -> Option<FfmpegPaths> {
    match FfmpegPaths::locate(None) {
        Ok(paths) => Some(paths),
        Err(e) => {
            eprintln!("skipping: {e}");
            None
        }
    }
}

fn workdir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ascii-painter-it-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Windows may keep freshly written files locked for a moment (virus scanning); retry.
fn cleanup(dir: PathBuf) {
    for _ in 0..30 {
        if std::fs::remove_dir_all(&dir).is_ok() || !dir.exists() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("could not remove {}", dir.display());
}

fn ffmpeg(paths: &FfmpegPaths, args: &[&str]) {
    let status =
        Command::new(&paths.ffmpeg).args(["-hide_banner", "-loglevel", "error", "-y"]).args(args).status().unwrap();
    assert!(status.success(), "ffmpeg {args:?}");
}

/// 2 s, 320×240 @ 25 fps with a sine soundtrack.
fn test_clip(paths: &FfmpegPaths, dir: &Path) -> PathBuf {
    let clip = dir.join("clip.mp4");
    ffmpeg(
        paths,
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=d=2:s=320x240:r=25",
            "-f",
            "lavfi",
            "-i",
            "sine=d=2",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-shortest",
            clip.to_str().unwrap(),
        ],
    );
    clip
}

fn small_settings() -> Settings {
    let mut settings = Settings::default();
    settings.grid.columns = 41; // odd pixel width after rendering, exercising the even padding
    settings
}

fn export(
    paths: &FfmpegPaths,
    input: &Path,
    output: &Path,
    format: VideoFormat,
    settings: Settings,
) -> Result<PathBuf, VideoError> {
    let info = probe(paths, input).unwrap();
    let job = ExportJob::start(
        paths.clone(),
        ExportRequest {
            input: input.into(),
            output: output.into(),
            format,
            settings,
            range: TimeRange::default(),
            info,
        },
    );
    loop {
        match job.recv().expect("job ends with Finished") {
            ExportEvent::Finished(result) => return result,
            ExportEvent::Progress(_) => {}
        }
    }
}

#[test]
fn mp4_export_keeps_frames_audio_and_even_size() {
    let Some(paths) = tools() else { return };
    let dir = workdir("mp4");
    let clip = test_clip(&paths, &dir);
    let out = dir.join("out.mp4");
    export(&paths, &clip, &out, VideoFormat::Mp4, small_settings()).unwrap();

    let info = probe(&paths, &out).unwrap();
    assert_eq!(info.frames, Some(50));
    assert!(info.has_audio);
    assert_eq!(info.display.w % 2, 0);
    assert_eq!(info.display.h % 2, 0);
    let converter = Converter::new(small_settings()).unwrap();
    let raw = converter.output_size(converter.grid_for(Size2 { w: 320, h: 240 }));
    assert_eq!(info.display, Size2 { w: raw.w + raw.w % 2, h: raw.h + raw.h % 2 });
    assert!(raw.w % 2 == 1 || raw.h % 2 == 1, "test should exercise padding");
    cleanup(dir);
}

#[test]
fn text_and_ansi_exports_contain_every_frame() {
    let Some(paths) = tools() else { return };
    let dir = workdir("text");
    let clip = test_clip(&paths, &dir);

    let txt = dir.join("out.txt");
    export(&paths, &clip, &txt, VideoFormat::Text, small_settings()).unwrap();
    let text = std::fs::read_to_string(&txt).unwrap();
    assert_eq!(text.split(FRAME_SEPARATOR).count(), 50);

    let ans = dir.join("out.ans");
    export(&paths, &clip, &ans, VideoFormat::Ansi, small_settings()).unwrap();
    let ansi = std::fs::read_to_string(&ans).unwrap();
    let (header, _) = VideoHeader::parse(&ansi).unwrap();
    assert_eq!(header.fps, 25.0);
    assert_eq!(header.grid.cols, 41);
    assert_eq!(ansi.matches("\x1b[H").count(), 50);
    cleanup(dir);
}

#[test]
fn rotated_video_exports_upright() {
    let Some(paths) = tools() else { return };
    let dir = workdir("rotate");
    let clip = test_clip(&paths, &dir);
    let rotated = dir.join("rotated.mp4");
    ffmpeg(&paths, &["-display_rotation", "90", "-i", clip.to_str().unwrap(), "-c", "copy", rotated.to_str().unwrap()]);
    let info = probe(&paths, &rotated).unwrap();
    assert_eq!(info.display, Size2 { w: 240, h: 320 });

    // Decoded frames really are portrait (ffmpeg autorotates).
    let size = decode_size(info.display, Size2 { w: 1000, h: 1000 });
    let mut reader = FrameReader::spawn(&paths, &rotated, TimeRange::default(), size, info.fps).unwrap();
    let frame = reader.next_frame().unwrap().unwrap();
    assert_eq!(frame.dimensions(), (240, 320));
    drop(reader);

    let out = dir.join("out.mp4");
    export(&paths, &rotated, &out, VideoFormat::Mp4, small_settings()).unwrap();
    let out_info = probe(&paths, &out).unwrap();
    assert!(out_info.display.h > out_info.display.w);
    cleanup(dir);
}

#[test]
fn animated_gif_exports_all_frames() {
    let Some(paths) = tools() else { return };
    let dir = workdir("gif");
    let gif = dir.join("anim.gif");
    ffmpeg(&paths, &["-f", "lavfi", "-i", "testsrc2=d=1:s=160x120:r=10", gif.to_str().unwrap()]);
    assert_eq!(ascii_painter_core::load::classify(&gif), ascii_painter_core::load::InputKind::Animated);
    let txt = dir.join("out.txt");
    export(&paths, &gif, &txt, VideoFormat::Text, small_settings()).unwrap();
    let text = std::fs::read_to_string(&txt).unwrap();
    assert_eq!(text.split(FRAME_SEPARATOR).count(), 10);
    cleanup(dir);
}

#[test]
fn preview_frame_matches_export_frame() {
    let Some(paths) = tools() else { return };
    let dir = workdir("preview");
    let clip = test_clip(&paths, &dir);
    let info = probe(&paths, &clip).unwrap();
    let converter = Converter::new(small_settings()).unwrap();
    let grid = converter.grid_for(info.display);
    let size = decode_size(info.display, converter.sample_size(grid));

    let preview = extract_frame(&paths, &clip, 0.0, size).unwrap();
    let mut reader = FrameReader::spawn(&paths, &clip, TimeRange::default(), size, info.fps).unwrap();
    let exported = reader.next_frame().unwrap().unwrap();
    drop(reader);
    assert_eq!(preview, exported);
    let render = |image| converter.render(&converter.convert(ConvertInput { image, grid, previous: None }));
    assert_eq!(render(&preview), render(&exported));
    cleanup(dir);
}

#[test]
fn cancel_removes_partial_output() {
    let Some(paths) = tools() else { return };
    let dir = workdir("cancel");
    let clip = dir.join("long.mp4");
    ffmpeg(
        &paths,
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=d=60:s=640x480:r=30",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            clip.to_str().unwrap(),
        ],
    );
    let out = dir.join("out.mp4");
    let info = probe(&paths, &clip).unwrap();
    let mut settings = Settings::default();
    settings.grid.columns = 300;
    let job = ExportJob::start(
        paths.clone(),
        ExportRequest {
            input: clip.clone(),
            output: out.clone(),
            format: VideoFormat::Mp4,
            settings,
            range: TimeRange::default(),
            info,
        },
    );
    // Wait for real progress so ffmpeg is mid-encode, then cancel.
    loop {
        match job.recv().unwrap() {
            ExportEvent::Progress(p) if p.done >= 5 => break,
            ExportEvent::Finished(r) => panic!("finished before cancel: {r:?}"),
            _ => {}
        }
    }
    job.cancel();
    let result = loop {
        if let ExportEvent::Finished(r) = job.recv().unwrap() {
            break r;
        }
    };
    assert!(matches!(result, Err(VideoError::Cancelled)), "{result:?}");
    std::thread::sleep(Duration::from_millis(100));
    assert!(!out.exists(), "partial output left behind");
    cleanup(dir);
}

#[test]
fn trimmed_range_exports_only_that_section() {
    let Some(paths) = tools() else { return };
    let dir = workdir("trim");
    let clip = test_clip(&paths, &dir);
    let info = probe(&paths, &clip).unwrap();
    let range = TimeRange { start: Some(0.5), end: Some(1.5) };
    assert_eq!(info.frames_in(range), Some(25));
    let txt = dir.join("out.txt");
    let job = ExportJob::start(
        paths.clone(),
        ExportRequest {
            input: clip,
            output: txt.clone(),
            format: VideoFormat::Text,
            settings: small_settings(),
            range,
            info,
        },
    );
    let result = loop {
        if let ExportEvent::Finished(r) = job.recv().unwrap() {
            break r;
        }
    };
    result.unwrap();
    let frames = std::fs::read_to_string(&txt).unwrap().split(FRAME_SEPARATOR).count();
    assert!((24..=26).contains(&frames), "{frames} frames");
    cleanup(dir);
}
