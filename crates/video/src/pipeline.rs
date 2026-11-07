//! Background export: decode → convert → render → write, with progress and cancellation.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel, sync_channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ascii_painter_core::render::ansi::{FRAME_PREFIX, VideoHeader};
use ascii_painter_core::render::text::FRAME_SEPARATOR;
use ascii_painter_core::{AsciiFrame, ConvertInput, Converter, GridSize, Settings, Size2};
use image::RgbaImage;

use crate::decode::{FrameReader, decode_size};
use crate::encode::{AudioSource, EncodeQuality, EncodeRequest, Encoder};
use crate::ffmpeg::{Cancellation, FfmpegPaths};
use crate::output::{OutputDestination, OutputPolicy};
use crate::probe::MediaInfo;
use crate::{TimeRange, VideoError};

/// Frames buffered between stages.
const QUEUE: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoFormat {
    Mp4,
    Text,
    Ansi,
}

impl VideoFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
            Self::Text => "txt",
            Self::Ansi => "ans",
        }
    }

    pub fn from_path(path: &Path) -> Option<Self> {
        match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
            "mp4" => Some(Self::Mp4),
            "txt" => Some(Self::Text),
            "ans" => Some(Self::Ansi),
            _ => None,
        }
    }
}

/// Even-sided MP4 frame size for a converter and grid.
pub fn mp4_size(converter: &Converter, grid: GridSize) -> Size2 {
    let size = converter.output_size(grid);
    Size2 { w: size.w + size.w % 2, h: size.h + size.h % 2 }
}

pub struct ExportRequest {
    pub input: PathBuf,
    pub output: PathBuf,
    pub format: VideoFormat,
    /// A snapshot: later edits elsewhere do not affect a running export.
    pub settings: Settings,
    pub range: TimeRange,
    pub info: MediaInfo,
    /// MP4 only.
    pub quality: EncodeQuality,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Progress {
    pub done: u64,
    pub total: Option<u64>,
    /// Frames per second processed.
    pub rate: f64,
    pub eta: Option<Duration>,
}

impl Progress {
    pub fn fraction(&self) -> Option<f32> {
        self.total.filter(|&t| t > 0).map(|t| (self.done as f32 / t as f32).min(1.0))
    }
}

#[derive(Debug)]
pub enum ExportEvent {
    Progress(Progress),
    Finished(Result<PathBuf, VideoError>),
}

pub struct ExportJob {
    cancel: Cancellation,
    events: Receiver<ExportEvent>,
    thread: Option<JoinHandle<()>>,
}

impl ExportJob {
    pub fn start(paths: FfmpegPaths, req: ExportRequest) -> Self {
        Self::start_with_policy(paths, req, OutputPolicy::ReplaceExisting)
    }

    pub fn start_cancellable(paths: FfmpegPaths, req: ExportRequest, cancel: Cancellation) -> Self {
        Self::start_cancellable_with_policy(paths, req, cancel, OutputPolicy::ReplaceExisting)
    }

    pub fn start_with_policy(paths: FfmpegPaths, req: ExportRequest, policy: OutputPolicy) -> Self {
        Self::start_cancellable_with_policy(paths, req, Cancellation::default(), policy)
    }

    pub fn start_cancellable_with_policy(
        paths: FfmpegPaths,
        req: ExportRequest,
        cancel: Cancellation,
        policy: OutputPolicy,
    ) -> Self {
        Self::start_inner(paths, req, cancel, policy, PublishHooks::default())
    }

    fn start_inner(
        paths: FfmpegPaths,
        req: ExportRequest,
        cancel: Cancellation,
        policy: OutputPolicy,
        hooks: PublishHooks,
    ) -> Self {
        let (tx, events) = channel();
        let flag = cancel.clone();
        let thread = std::thread::spawn(move || {
            let result = run(&paths, req, &flag, &tx, policy, hooks);
            let _ = tx.send(ExportEvent::Finished(result));
        });
        Self { cancel, events, thread: Some(thread) }
    }

    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    /// Events received so far, without blocking.
    pub fn poll(&self) -> Vec<ExportEvent> {
        self.events.try_iter().collect()
    }

    pub fn is_finished(&self) -> bool {
        self.thread.as_ref().is_none_or(JoinHandle::is_finished)
    }

    /// Block for the next event; `None` once the job is over.
    pub fn recv(&self) -> Option<ExportEvent> {
        self.events.recv().ok()
    }

    /// Cancel and wait until ffmpeg is gone and partial output removed.
    pub fn cancel_and_wait(self) {
        self.cancel();
        self.wait();
    }

    /// Join completed work without cancelling a caller-owned cancellation scope.
    /// Call from a worker, never from the UI thread while an export is running.
    pub fn wait(mut self) {
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

// Per-operation boundaries for deterministic tests; no hooks exist in release builds.
#[derive(Default)]
struct PublishHooks {
    #[cfg(test)]
    before: Option<Box<dyn FnOnce() + Send>>,
    #[cfg(test)]
    after: Option<Box<dyn FnOnce() + Send>>,
}

impl Drop for ExportJob {
    fn drop(&mut self) {
        if let Some(t) = self.thread.take() {
            self.cancel();
            let _ = t.join();
        }
    }
}

enum Sink {
    Mp4 { encoder: Encoder },
    Text(BufWriter<File>),
}

enum Rendered {
    Pixels(RgbaImage),
    Text(String),
}

impl Sink {
    fn write(&mut self, item: Rendered) -> Result<(), VideoError> {
        match (self, item) {
            (Sink::Mp4 { encoder }, Rendered::Pixels(image)) => encoder.write_frame(&image),
            (Sink::Text(out), Rendered::Text(text)) => Ok(out.write_all(text.as_bytes())?),
            _ => unreachable!("sink and rendering are chosen from the same format"),
        }
    }

    fn finish(self) -> Result<(), VideoError> {
        match self {
            Sink::Mp4 { encoder, .. } => encoder.finish(),
            Sink::Text(mut out) => Ok(out.flush()?),
        }
    }
}

fn run(
    paths: &FfmpegPaths,
    mut req: ExportRequest,
    cancel: &Cancellation,
    events: &Sender<ExportEvent>,
    policy: OutputPolicy,
    hooks: PublishHooks,
) -> Result<PathBuf, VideoError> {
    let destination = OutputDestination::capture(&req.input, &req.output, policy)?;
    let stage = destination.stage(req.format.extension())?;
    req.output = stage.to_path_buf();
    render(paths, req, cancel, events)?;
    // render has closed writers and joined/reaped all subprocess owners.
    #[cfg(test)]
    if let Some(before) = hooks.before {
        before();
    }
    #[cfg(not(test))]
    let _ = hooks;
    if cancel.is_cancelled() {
        return Err(VideoError::Cancelled);
    }
    let result = destination.publish_cancellable(stage, cancel)?;
    #[cfg(test)]
    if let Some(after) = hooks.after {
        after();
    }
    Ok(result)
}

fn render(
    paths: &FfmpegPaths,
    req: ExportRequest,
    cancel: &Cancellation,
    events: &Sender<ExportEvent>,
) -> Result<(), VideoError> {
    if cancel.is_cancelled() {
        return Err(VideoError::Cancelled);
    }
    let converter = Converter::new(req.settings)?;
    let info = &req.info;
    let grid = converter.grid_for(info.display);
    let total = info.frames_in(req.range);
    let threads = std::thread::available_parallelism().map_or(2, |n| n.get()).saturating_sub(1).max(1);
    let pool =
        rayon::ThreadPoolBuilder::new().num_threads(threads).build().map_err(|e| VideoError::Encode(e.to_string()))?;

    let mut sink = match req.format {
        VideoFormat::Mp4 => {
            let audio = info.has_audio.then_some(AudioSource { input: &req.input, range: req.range });
            let request = EncodeRequest {
                output: &req.output,
                size: mp4_size(&converter, grid),
                fps: info.fps,
                audio,
                quality: req.quality,
            };
            Sink::Mp4 { encoder: Encoder::spawn_cancellable(paths, request, cancel)? }
        }
        VideoFormat::Text | VideoFormat::Ansi => Sink::Text(BufWriter::new(File::create(&req.output)?)),
    };
    if let (VideoFormat::Ansi, Sink::Text(out)) = (req.format, &mut sink) {
        out.write_all(VideoHeader { fps: info.fps.as_f64(), grid }.encode().as_bytes())?;
    }
    let mut reader = FrameReader::spawn_cancellable(
        paths,
        &req.input,
        req.range,
        decode_size(info.display, converter.sample_size(grid)),
        info.fps,
        cancel,
    )?;

    std::thread::scope(|scope| {
        let (frame_tx, frame_rx) = sync_channel(QUEUE);
        let decoder = scope.spawn(move || -> Result<(), VideoError> {
            loop {
                if cancel.is_cancelled() {
                    return Ok(()); // dropping the reader kills ffmpeg
                }
                match reader.next_frame()? {
                    Some(frame) => {
                        if frame_tx.send(frame).is_err() {
                            return Ok(());
                        }
                    }
                    None => return reader.finish(),
                }
            }
        });

        let (out_tx, out_rx) = sync_channel::<Rendered>(QUEUE);
        let writer = scope.spawn(move || -> Result<(), VideoError> {
            for item in out_rx {
                if cancel.is_cancelled() {
                    return Ok(());
                }
                if let Err(write_error) = sink.write(item) {
                    // A broken pipe usually means ffmpeg failed; its stderr is the useful error.
                    return Err(sink.finish().err().unwrap_or(write_error));
                }
            }
            if cancel.is_cancelled() { Ok(()) } else { sink.finish() }
        });

        let started = Instant::now();
        let mut last_report = Instant::now() - Duration::from_secs(1);
        let mut previous: Option<AsciiFrame> = None;
        let mut done = 0u64;
        for frame in frame_rx {
            if cancel.is_cancelled() {
                break;
            }
            let (ascii, rendered) = pool.install(|| {
                let ascii = converter.convert(ConvertInput { image: &frame, grid, previous: previous.as_ref() });
                let rendered = match req.format {
                    VideoFormat::Mp4 => Rendered::Pixels(converter.render_even(&ascii)),
                    VideoFormat::Text => {
                        let sep = if done == 0 { String::new() } else { FRAME_SEPARATOR.to_string() };
                        Rendered::Text(sep + &converter.to_text(&ascii))
                    }
                    VideoFormat::Ansi => Rendered::Text(format!("{FRAME_PREFIX}{}", converter.to_ansi(&ascii))),
                };
                (ascii, rendered)
            });
            previous = Some(ascii);
            if out_tx.send(rendered).is_err() {
                break;
            }
            done += 1;
            if last_report.elapsed() >= Duration::from_millis(100) {
                last_report = Instant::now();
                let _ = events.send(ExportEvent::Progress(progress(done, total, started.elapsed())));
            }
        }
        drop(out_tx);
        let decoded = decoder.join().expect("decoder thread panicked");
        let written = writer.join().expect("writer thread panicked");
        let _ = events.send(ExportEvent::Progress(progress(done, total, started.elapsed())));
        if cancel.is_cancelled() {
            return Err(VideoError::Cancelled);
        }
        decoded.and(written)?;
        if done == 0 {
            return Err(VideoError::Decode("no frames in the selected range".into()));
        }
        Ok(())
    })
}

fn progress(done: u64, total: Option<u64>, elapsed: Duration) -> Progress {
    let rate = done as f64 / elapsed.as_secs_f64().max(1e-3);
    let eta = total.filter(|_| rate > 0.0).map(|t| Duration::from_secs_f64(t.saturating_sub(done) as f64 / rate));
    Progress { done, total, rate, eta }
}

#[cfg(test)]
mod safety_tests {
    use super::*;

    fn request(input: &Path, output: &Path) -> ExportRequest {
        ExportRequest {
            input: input.into(),
            output: output.into(),
            format: VideoFormat::Text,
            settings: Settings::default(),
            range: TimeRange::default(),
            quality: EncodeQuality::default(),
            info: MediaInfo {
                display: Size2 { w: 8, h: 6 },
                fps: crate::Fps { num: 1, den: 1 },
                frames: Some(1),
                duration: Some(1.0),
                has_audio: false,
                hdr: false,
            },
        }
    }
    fn missing_tools(dir: &Path) -> FfmpegPaths {
        FfmpegPaths { ffmpeg: dir.join("missing-ffmpeg"), ffprobe: dir.join("missing-ffprobe") }
    }
    fn finish(job: ExportJob) -> Result<PathBuf, VideoError> {
        let result = loop {
            match job.events.recv_timeout(Duration::from_secs(30)) {
                Ok(ExportEvent::Finished(result)) => break result,
                Ok(ExportEvent::Progress(_)) => {}
                Err(error) => {
                    job.cancel_and_wait();
                    panic!("final event: {error}");
                }
            }
        };
        job.wait();
        result
    }

    #[test]
    fn same_path_errors_never_delete_the_source() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("source.mp4");
        std::fs::write(&input, b"irreplaceable source").unwrap();
        assert!(matches!(
            finish(ExportJob::start(missing_tools(dir.path()), request(&input, &input))),
            Err(VideoError::SameFile)
        ));
        assert_eq!(std::fs::read(&input).unwrap(), b"irreplaceable source");
    }

    #[test]
    fn hardlink_errors_never_truncate_the_source_or_delete_the_alias() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("source.mp4");
        let alias = dir.path().join("alias.txt");
        std::fs::write(&input, b"irreplaceable source").unwrap();
        std::fs::hard_link(&input, &alias).unwrap();
        assert!(finish(ExportJob::start(missing_tools(dir.path()), request(&input, &alias))).is_err());
        assert_eq!(std::fs::read(&input).unwrap(), b"irreplaceable source");
        assert_eq!(std::fs::read(&alias).unwrap(), b"irreplaceable source");
    }

    #[test]
    fn failed_conversion_leaves_an_existing_destination_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("source.mp4");
        let output = dir.path().join("keep.txt");
        std::fs::write(&input, b"source").unwrap();
        std::fs::write(&output, b"existing destination").unwrap();
        assert!(finish(ExportJob::start(missing_tools(dir.path()), request(&input, &output))).is_err());
        assert_eq!(std::fs::read(&output).unwrap(), b"existing destination");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
    }

    #[test]
    fn invalid_settings_and_pre_cancelled_jobs_only_remove_owned_stages() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("source.mp4");
        let output = dir.path().join("keep.txt");
        std::fs::write(&input, b"source").unwrap();
        std::fs::write(&output, b"existing destination").unwrap();
        let mut invalid = request(&input, &output);
        invalid.settings.charset.preset = ascii_painter_core::settings::CharsetPreset::Custom;
        invalid.settings.charset.custom.clear();
        assert!(matches!(finish(ExportJob::start(missing_tools(dir.path()), invalid)), Err(VideoError::Core(_))));
        let cancel = Cancellation::default();
        cancel.cancel();
        assert!(matches!(
            finish(ExportJob::start_cancellable(missing_tools(dir.path()), request(&input, &output), cancel)),
            Err(VideoError::Cancelled)
        ));
        assert_eq!(std::fs::read(&output).unwrap(), b"existing destination");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
    }

    fn tools() -> Option<FfmpegPaths> {
        match FfmpegPaths::locate(None) {
            Ok(paths) => Some(paths),
            Err(error) => {
                assert!(
                    std::env::var_os("ASCII_PAINTER_REQUIRE_MEDIA_TESTS").is_none(),
                    "required media tools: {error}"
                );
                eprintln!("skipping: {error}");
                None
            }
        }
    }

    fn clip(paths: &FfmpegPaths, dir: &Path) -> PathBuf {
        let input = dir.join("source.mp4");
        let result = std::process::Command::new(&paths.ffmpeg)
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=d=0.4:s=32x24:r=5",
                "-c:v",
                "libx264",
            ])
            .arg(&input)
            .output()
            .unwrap();
        assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
        input
    }

    fn media_request(paths: &FfmpegPaths, input: &Path, output: &Path) -> ExportRequest {
        let mut req = request(input, output);
        req.info = crate::probe(paths, input).unwrap();
        req.settings.grid.columns = 8;
        req.settings.render.font_px = 8.0;
        req
    }

    fn boundary() -> (Box<dyn FnOnce() + Send>, Sender<()>, Receiver<()>) {
        let (entered, observed) = channel();
        let (release, held) = channel();
        (
            Box::new(move || {
                entered.send(()).unwrap();
                held.recv_timeout(Duration::from_secs(10)).expect("release test boundary");
            }),
            release,
            observed,
        )
    }

    #[test]
    fn cancellation_before_commit_preserves_old_output_but_after_commit_reports_success() {
        let Some(paths) = tools() else { return };
        let dir = tempfile::tempdir().unwrap();
        let input = clip(&paths, dir.path());
        let output = dir.path().join("out.txt");
        for after_commit in [false, true] {
            std::fs::write(&output, b"old bytes").unwrap();
            let (hook, release, observed) = boundary();
            let hooks = if after_commit {
                PublishHooks { after: Some(hook), ..Default::default() }
            } else {
                PublishHooks { before: Some(hook), ..Default::default() }
            };
            let job = ExportJob::start_inner(
                paths.clone(),
                media_request(&paths, &input, &output),
                Cancellation::default(),
                OutputPolicy::ReplaceExisting,
                hooks,
            );
            observed.recv_timeout(Duration::from_secs(10)).unwrap();
            job.cancel();
            release.send(()).unwrap();
            let result = finish(job);
            if after_commit {
                assert!(result.is_ok());
                assert_ne!(std::fs::read(&output).unwrap(), b"old bytes");
            } else {
                assert!(matches!(result, Err(VideoError::Cancelled)));
                assert_eq!(std::fs::read(&output).unwrap(), b"old bytes");
            }
            assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
        }
    }

    #[test]
    fn destination_changes_at_the_publish_boundary_are_preserved() {
        let Some(paths) = tools() else { return };
        let dir = tempfile::tempdir().unwrap();
        let input = clip(&paths, dir.path());
        let original = std::fs::read(&input).unwrap();
        for change in ["created", "modified", "replaced", "alias"] {
            let output = dir.path().join(format!("{change}.txt"));
            if change != "created" {
                std::fs::write(&output, b"old bytes").unwrap();
            }
            let (hook, release, observed) = boundary();
            let job = ExportJob::start_inner(
                paths.clone(),
                media_request(&paths, &input, &output),
                Cancellation::default(),
                OutputPolicy::ReplaceExisting,
                PublishHooks { before: Some(hook), ..Default::default() },
            );
            observed.recv_timeout(Duration::from_secs(10)).unwrap();
            match change {
                "replaced" | "alias" => {
                    std::fs::remove_file(&output).unwrap();
                    if change == "alias" {
                        std::fs::hard_link(&input, &output).unwrap();
                    } else {
                        std::fs::write(&output, b"old bytes").unwrap();
                    }
                }
                _ => std::fs::write(&output, b"external bytes").unwrap(),
            }
            let expected = std::fs::read(&output).unwrap();
            release.send(()).unwrap();
            assert!(finish(job).is_err(), "{change}");
            assert_eq!(std::fs::read(&output).unwrap(), expected);
            assert_eq!(std::fs::read(&input).unwrap(), original);
            std::fs::remove_file(&output).unwrap();
            assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        }
    }

    #[test]
    fn dropping_a_pending_job_and_unwinding_only_clean_owned_stages() {
        let Some(paths) = tools() else { return };
        let dir = tempfile::tempdir().unwrap();
        let input = clip(&paths, dir.path());
        let output = dir.path().join("out.txt");
        std::fs::write(&output, b"old bytes").unwrap();
        let (hook, release, observed) = boundary();
        let cancel = Cancellation::default();
        let job = ExportJob::start_inner(
            paths.clone(),
            media_request(&paths, &input, &output),
            cancel.clone(),
            OutputPolicy::ReplaceExisting,
            PublishHooks { before: Some(hook), ..Default::default() },
        );
        observed.recv_timeout(Duration::from_secs(10)).unwrap();
        let owner = std::thread::spawn(move || drop(job));
        let start = Instant::now();
        while !cancel.is_cancelled() && start.elapsed() < Duration::from_secs(5) {
            std::thread::yield_now();
        }
        release.send(()).unwrap();
        owner.join().unwrap();
        assert!(cancel.is_cancelled());
        assert_eq!(std::fs::read(&output).unwrap(), b"old bytes");
        let job = ExportJob::start_inner(
            paths.clone(),
            media_request(&paths, &input, &output),
            Cancellation::default(),
            OutputPolicy::ReplaceExisting,
            PublishHooks { before: Some(Box::new(|| panic!("test unwind"))), ..Default::default() },
        );
        while job.recv().is_some() {} // panic disconnects; Drop joins the unwound worker
        job.wait();
        assert_eq!(std::fs::read(&output).unwrap(), b"old bytes");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
    }

    #[test]
    fn encoder_failure_does_not_touch_the_requested_destination() {
        let Some(paths) = tools() else { return };
        let dir = tempfile::tempdir().unwrap();
        let input = clip(&paths, dir.path());
        let output = dir.path().join("out.mp4");
        std::fs::write(&output, b"old bytes").unwrap();
        let mut req = media_request(&paths, &input, &output);
        req.format = VideoFormat::Mp4;
        req.info.fps = crate::Fps { num: 0, den: 1 }; // ffmpeg rejects this raw-video rate
        assert!(finish(ExportJob::start(paths, req)).is_err());
        assert_eq!(std::fs::read(&output).unwrap(), b"old bytes");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
    }
}
