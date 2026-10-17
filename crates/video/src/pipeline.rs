//! Background export: decode → convert → render → write, with progress and cancellation.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel, sync_channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ascii_painter_core::render::ansi::{FRAME_PREFIX, VideoHeader};
use ascii_painter_core::render::text::FRAME_SEPARATOR;
use ascii_painter_core::{AsciiFrame, ConvertInput, Converter, GridSize, Settings, Size2};
use image::RgbaImage;

use crate::decode::{FrameReader, decode_size};
use crate::encode::{AudioSource, EncodeQuality, EncodeRequest, Encoder};
use crate::ffmpeg::FfmpegPaths;
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
    cancel: Arc<AtomicBool>,
    events: Receiver<ExportEvent>,
    thread: Option<JoinHandle<()>>,
}

impl ExportJob {
    pub fn start(paths: FfmpegPaths, req: ExportRequest) -> Self {
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, events) = channel();
        let flag = cancel.clone();
        let thread = std::thread::spawn(move || {
            let output = req.output.clone();
            let result = run(&paths, req, &flag, &tx);
            if result.is_err() {
                remove_partial(&output);
            }
            let _ = tx.send(ExportEvent::Finished(result));
        });
        Self { cancel, events, thread: Some(thread) }
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Events received so far, without blocking.
    pub fn poll(&self) -> Vec<ExportEvent> {
        self.events.try_iter().collect()
    }

    /// Block for the next event; `None` once the job is over.
    pub fn recv(&self) -> Option<ExportEvent> {
        self.events.recv().ok()
    }

    /// Cancel and wait until ffmpeg is gone and partial output removed.
    pub fn cancel_and_wait(mut self) {
        self.cancel();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for ExportJob {
    fn drop(&mut self) {
        self.cancel();
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
    req: ExportRequest,
    cancel: &AtomicBool,
    events: &Sender<ExportEvent>,
) -> Result<PathBuf, VideoError> {
    if req.output.canonicalize().ok().is_some_and(|o| req.input.canonicalize().ok() == Some(o)) {
        return Err(VideoError::SameFile);
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
            Sink::Mp4 { encoder: Encoder::spawn(paths, request)? }
        }
        VideoFormat::Text | VideoFormat::Ansi => Sink::Text(BufWriter::new(File::create(&req.output)?)),
    };
    if let (VideoFormat::Ansi, Sink::Text(out)) = (req.format, &mut sink) {
        out.write_all(VideoHeader { fps: info.fps.as_f64(), grid }.encode().as_bytes())?;
    }
    let mut reader = FrameReader::spawn(
        paths,
        &req.input,
        req.range,
        decode_size(info.display, converter.sample_size(grid)),
        info.fps,
    )?;

    std::thread::scope(|scope| {
        let (frame_tx, frame_rx) = sync_channel(QUEUE);
        let decoder = scope.spawn(move || -> Result<(), VideoError> {
            loop {
                if cancel.load(Ordering::Relaxed) {
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
                if cancel.load(Ordering::Relaxed) {
                    return Ok(());
                }
                if let Err(write_error) = sink.write(item) {
                    // A broken pipe usually means ffmpeg failed; its stderr is the useful error.
                    return Err(sink.finish().err().unwrap_or(write_error));
                }
            }
            if cancel.load(Ordering::Relaxed) { Ok(()) } else { sink.finish() }
        });

        let started = Instant::now();
        let mut last_report = Instant::now() - Duration::from_secs(1);
        let mut previous: Option<AsciiFrame> = None;
        let mut done = 0u64;
        for frame in frame_rx {
            if cancel.load(Ordering::Relaxed) {
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
        if cancel.load(Ordering::Relaxed) {
            return Err(VideoError::Cancelled);
        }
        decoded.and(written)?;
        if done == 0 {
            return Err(VideoError::Decode("no frames in the selected range".into()));
        }
        Ok(req.output.clone())
    })
}

/// Delete a partial output. On Windows the file can stay locked briefly after ffmpeg exits
/// (e.g. while a virus scanner inspects it), so retry for a couple of seconds.
fn remove_partial(path: &Path) {
    for attempt in 0..20 {
        match std::fs::remove_file(path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound && attempt < 19 => {
                std::thread::sleep(Duration::from_millis(100));
            }
            _ => return,
        }
    }
}

fn progress(done: u64, total: Option<u64>, elapsed: Duration) -> Progress {
    let rate = done as f64 / elapsed.as_secs_f64().max(1e-3);
    let eta = total.filter(|_| rate > 0.0).map(|t| Duration::from_secs_f64(t.saturating_sub(done) as f64 / rate));
    Progress { done, total, rate, eta }
}
