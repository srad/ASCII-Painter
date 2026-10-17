//! Background preview conversion. Requests are latest-wins: a burst of setting changes only
//! converts the newest one.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use ascii_painter_core::glyphs::font_cell;
use ascii_painter_core::load::flatten;
use ascii_painter_core::{ConvertInput, Converter, GridSize, Rgb, Settings, Size2};
use ascii_painter_video::decode::{decode_size, extract_frame};
use ascii_painter_video::{FfmpegPaths, MediaInfo};
use image::{DynamicImage, RgbImage, RgbaImage};

#[derive(Clone)]
pub enum PreviewSource {
    Image(Arc<DynamicImage>),
    Video { paths: FfmpegPaths, path: PathBuf, info: MediaInfo, time: f64 },
}

pub struct PreviewRequest {
    pub generation: u64,
    pub settings: Settings,
    pub source: PreviewSource,
    /// Largest texture side the GUI can show.
    pub max_side: u32,
}

pub struct PreviewOutput {
    pub ascii: RgbaImage,
    /// A new original frame (video only, when the time changed).
    pub original: Option<RgbImage>,
    pub grid: GridSize,
    /// Pixel size of a real export.
    pub output_size: Size2,
    /// The preview was rendered with a smaller font to fit `max_side`.
    pub reduced: bool,
    pub rejected: Vec<char>,
}

pub struct PreviewResponse {
    pub generation: u64,
    pub result: Result<PreviewOutput, String>,
}

pub struct PreviewWorker {
    requests: Sender<PreviewRequest>,
    responses: Receiver<PreviewResponse>,
}

impl PreviewWorker {
    pub fn spawn(ctx: eframe::egui::Context) -> Self {
        let (requests, inbox) = channel::<PreviewRequest>();
        let (outbox, responses) = channel();
        std::thread::Builder::new()
            .name("preview".into())
            .spawn(move || {
                let mut cache = Cache::default();
                while let Ok(mut request) = inbox.recv() {
                    while let Ok(newer) = inbox.try_recv() {
                        request = newer;
                    }
                    let generation = request.generation;
                    let result = process(&mut cache, request);
                    if outbox.send(PreviewResponse { generation, result }).is_err() {
                        return;
                    }
                    ctx.request_repaint();
                }
            })
            .expect("spawn preview thread");
        Self { requests, responses }
    }

    pub fn request(&self, request: PreviewRequest) {
        let _ = self.requests.send(request);
    }

    pub fn poll(&self) -> Option<PreviewResponse> {
        self.responses.try_iter().last()
    }
}

/// Decoded inputs reused across requests.
#[derive(Default)]
struct Cache {
    /// (image identity, background) → flattened RGB.
    still: Option<(usize, Rgb, Arc<RgbImage>)>,
    /// (path, time, size) → frame scaled for conversion.
    sampled: Option<(PathBuf, u64, Size2, Arc<RgbImage>)>,
    /// (path, time) of the last original frame sent to the GUI.
    shown: Option<(PathBuf, u64)>,
}

/// Fit `size` inside a square of `max` keeping aspect.
pub fn fit_within(size: Size2, max: u32) -> Size2 {
    if size.w <= max && size.h <= max {
        return size;
    }
    let scale = max as f64 / size.w.max(size.h) as f64;
    Size2 { w: ((size.w as f64 * scale) as u32).max(1), h: ((size.h as f64 * scale) as u32).max(1) }
}

/// Largest font size (in half-pixel steps) whose render of `grid` fits in `limit` pixels.
/// Cell sizes are rounded, so this is searched rather than computed.
fn reduced_font_px(font_px: f32, grid: GridSize, limit: u32) -> f32 {
    let mut px = font_px;
    while px > 1.0 {
        let cell = font_cell(px);
        if grid.cols * cell.w <= limit && grid.rows * cell.h <= limit {
            break;
        }
        px -= 0.5;
    }
    px
}

fn process(cache: &mut Cache, request: PreviewRequest) -> Result<PreviewOutput, String> {
    let converter = Converter::new(request.settings.clone()).map_err(|e| e.to_string())?;
    let mut original = None;
    let (image, display) = match &request.source {
        PreviewSource::Image(source) => {
            let id = Arc::as_ptr(source) as usize;
            let bg = request.settings.color.bg;
            let rgb = match &cache.still {
                Some((cached, cached_bg, rgb))
                    if *cached == id && (*cached_bg == bg || !source.color().has_alpha()) =>
                {
                    rgb.clone()
                }
                _ => {
                    let rgb = Arc::new(flatten(source.as_ref().clone(), bg));
                    cache.still = Some((id, bg, rgb.clone()));
                    rgb
                }
            };
            let display = Size2 { w: rgb.width(), h: rgb.height() };
            (rgb, display)
        }
        PreviewSource::Video { paths, path, info, time } => {
            let key = (time * 1000.0).round() as u64;
            let grid = converter.grid_for(info.display);
            let size = decode_size(info.display, converter.sample_size(grid));
            let frame = match &cache.sampled {
                Some((p, t, s, frame)) if p == path && *t == key && *s == size => frame.clone(),
                _ => {
                    let frame = Arc::new(extract_frame(paths, path, *time, size).map_err(|e| e.to_string())?);
                    cache.sampled = Some((path.clone(), key, size, frame.clone()));
                    frame
                }
            };
            if cache.shown.as_ref() != Some(&(path.clone(), key)) {
                let size = fit_within(info.display, request.max_side.min(2048));
                original = Some(extract_frame(paths, path, *time, size).map_err(|e| e.to_string())?);
                cache.shown = Some((path.clone(), key));
            }
            (frame, info.display)
        }
    };

    let grid = converter.grid_for(display);
    let frame = converter.convert(ConvertInput { image: &image, grid, previous: None });
    let output_size = converter.output_size(grid);
    let limit = request.max_side.min(8192);
    let (ascii, reduced) = if output_size.w > limit || output_size.h > limit {
        (converter.render_at(&frame, reduced_font_px(converter.settings().render.font_px, grid, limit)), true)
    } else {
        (converter.render(&frame), false)
    };
    Ok(PreviewOutput { ascii, original, grid, output_size, reduced, rejected: converter.rejected_chars().to_vec() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_within_keeps_aspect() {
        assert_eq!(fit_within(Size2 { w: 4000, h: 2000 }, 1000), Size2 { w: 1000, h: 500 });
        assert_eq!(fit_within(Size2 { w: 10, h: 20 }, 1000), Size2 { w: 10, h: 20 });
    }

    #[test]
    fn oversized_previews_are_reduced_to_the_texture_limit() {
        let mut settings = Settings::default();
        settings.grid.columns = 2000;
        let image = Arc::new(DynamicImage::ImageRgb8(RgbImage::new(400, 300)));
        let request = PreviewRequest { generation: 1, settings, source: PreviewSource::Image(image), max_side: 2048 };
        let out = process(&mut Cache::default(), request).unwrap();
        assert!(out.reduced);
        assert!(out.ascii.width() <= 2048 && out.ascii.height() <= 2048);
        assert!(out.output_size.w > 2048);
    }
}
