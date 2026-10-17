//! Terminal playback of `.ans` exports, videos and images.

use std::io::{BufWriter, Write, stdout};
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use ascii_painter_core::load::{InputKind, classify, load_still};
use ascii_painter_core::render::ansi::{FRAME_PREFIX, VideoHeader};
use ascii_painter_core::{ConvertInput, Converter, GridSize, Settings, Size2};
use ascii_painter_video::decode::{FrameReader, decode_size};
use ascii_painter_video::{FfmpegPaths, TimeRange, probe};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::{cursor, execute, terminal};

/// Typical terminal cell width ÷ height.
const TERMINAL_ASPECT: f32 = 0.5;

pub fn run(input: &Path, settings: &Settings, ffmpeg: impl FnOnce() -> Result<FfmpegPaths>) -> Result<()> {
    if input.extension().is_some_and(|e| e.eq_ignore_ascii_case("ans")) {
        let text = std::fs::read_to_string(input).with_context(|| format!("reading {}", input.display()))?;
        if let Some((header, len)) = VideoHeader::parse(&text) {
            return play_ans(&text[len..], header.fps);
        }
        print!("{text}");
        return Ok(());
    }
    match classify(input) {
        InputKind::Still => show_image(input, settings),
        InputKind::Animated | InputKind::Other => play_video(input, settings, &ffmpeg()?),
    }
}

/// Alternate screen + raw mode for the lifetime of the value, restored on drop (also on panic).
struct Screen;

impl Screen {
    fn enter() -> Result<Self> {
        terminal::enable_raw_mode()?;
        execute!(stdout(), terminal::EnterAlternateScreen, cursor::Hide)?;
        Ok(Self)
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        let _ = execute!(stdout(), cursor::Show, terminal::LeaveAlternateScreen);
        let _ = terminal::disable_raw_mode();
    }
}

fn quit_requested() -> Result<bool> {
    while event::poll(Duration::ZERO)? {
        if let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            let ctrl_c = key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL);
            if ctrl_c || matches!(key.code, KeyCode::Char('q') | KeyCode::Esc) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Raw mode does not translate `\n`, so frames need explicit carriage returns.
fn raw_lines(frame: &str) -> String {
    frame.replace('\n', "\r\n")
}

fn play_ans(body: &str, fps: f64) -> Result<()> {
    let _screen = Screen::enter()?;
    let mut out = BufWriter::new(stdout().lock());
    let period = Duration::from_secs_f64(1.0 / fps.max(1.0));
    let start = Instant::now();
    for (i, frame) in body.split(FRAME_PREFIX).filter(|f| !f.is_empty()).enumerate() {
        if quit_requested()? {
            break;
        }
        write!(out, "{FRAME_PREFIX}{}", raw_lines(frame))?;
        out.flush()?;
        if let Some(wait) = (start + period * (i as u32 + 1)).checked_duration_since(Instant::now()) {
            std::thread::sleep(wait);
        }
    }
    Ok(())
}

/// A converter whose grid fits in `max` terminal cells.
fn fitted(settings: &Settings, source: Size2, max: GridSize) -> Result<(Converter, GridSize)> {
    let mut settings = settings.clone();
    settings.grid.cell_aspect_override.get_or_insert(TERMINAL_ASPECT);
    settings.grid.columns = settings.grid.columns.min(max.cols);
    let converter = Converter::new(settings.clone())?;
    let grid = converter.grid_for(source);
    if grid.rows <= max.rows {
        return Ok((converter, grid));
    }
    settings.grid.columns = (grid.cols * max.rows / grid.rows).max(1);
    let converter = Converter::new(settings)?;
    let grid = converter.grid_for(source);
    Ok((converter, grid))
}

fn terminal_grid() -> GridSize {
    let (cols, rows) = terminal::size().unwrap_or((80, 24));
    GridSize { cols: cols as u32, rows: (rows as u32).saturating_sub(1).max(1) }
}

fn show_image(input: &Path, settings: &Settings) -> Result<()> {
    let image = load_still(input, settings.color.bg)?;
    let source = Size2 { w: image.width(), h: image.height() };
    let unlimited_rows = GridSize { cols: terminal_grid().cols, rows: u32::MAX };
    let (converter, grid) = fitted(settings, source, unlimited_rows)?;
    let frame = converter.convert(ConvertInput { image: &image, grid, previous: None });
    print!("{}", converter.to_ansi(&frame));
    Ok(())
}

fn play_video(input: &Path, settings: &Settings, paths: &FfmpegPaths) -> Result<()> {
    let info = probe(paths, input)?;
    let (converter, grid) = fitted(settings, info.display, terminal_grid())?;
    let size = decode_size(info.display, converter.sample_size(grid));
    let mut reader = FrameReader::spawn(paths, input, TimeRange::default(), size, info.fps)?;
    let _screen = Screen::enter()?;
    let mut out = BufWriter::new(stdout().lock());
    let period = Duration::from_secs_f64(1.0 / info.fps.as_f64());
    let start = Instant::now();
    let mut previous = None;
    let mut index = 0u32;
    while let Some(image) = reader.next_frame()? {
        if quit_requested()? {
            break;
        }
        index += 1;
        let due = start + period * index;
        // Behind schedule: skip converting this frame to catch up.
        if Instant::now() > due + period {
            continue;
        }
        let frame = converter.convert(ConvertInput { image: &image, grid, previous: previous.as_ref() });
        write!(out, "{FRAME_PREFIX}{}", raw_lines(&converter.to_ansi(&frame)))?;
        out.flush()?;
        previous = Some(frame);
        if let Some(wait) = due.checked_duration_since(Instant::now()) {
            std::thread::sleep(wait);
        }
    }
    Ok(())
}
