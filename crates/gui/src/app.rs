//! Application state, layout and the glue between UI, preview worker and exports.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use ascii_painter_core::load::{InputKind, classify, flatten, load_oriented};
use ascii_painter_core::presets::{self, Preset};
use ascii_painter_core::{ConvertInput, Converter, GridSize, Settings, Size2};
use ascii_painter_video::encode::MAX_SIDE;
use ascii_painter_video::pipeline::mp4_size;
use ascii_painter_video::{
    EncodeQuality, ExportEvent, ExportJob, ExportRequest, FfmpegPaths, MediaInfo, Progress, TimeRange, VideoFormat,
    probe,
};
use eframe::egui::{
    self, ColorImage, ComboBox, FontData, FontDefinitions, FontFamily, ProgressBar, RichText, ScrollArea,
    TextureFilter, TextureHandle, TextureOptions, Ui,
};
use image::{DynamicImage, RgbaImage};
use serde::{Deserialize, Serialize};

use crate::settings_ui::{PanelFacts, settings_panel};
use crate::theme;
use crate::viewer::{self, ViewMode, ViewerImages, ViewerState};
use crate::worker::{PreviewRequest, PreviewSource, PreviewWorker, fit_within};

const DEBOUNCE: Duration = Duration::from_millis(150);
/// Largest side of the "original" texture; more is never visible.
const ORIGINAL_MAX_SIDE: u32 = 4096;
const CUSTOM_PRESET: &str = "Custom";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OutputFormat {
    Png,
    Mp4,
    Text,
    Ansi,
}

impl OutputFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Mp4 => "mp4",
            Self::Text => "txt",
            Self::Ansi => "ans",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Png => "PNG image",
            Self::Mp4 => "MP4 video",
            Self::Text => "Plain text (.txt)",
            Self::Ansi => "ANSI colour (.ans)",
        }
    }

    pub fn options(kind: SourceKindTag) -> &'static [OutputFormat] {
        match kind {
            SourceKindTag::Image => &[Self::Png, Self::Text, Self::Ansi],
            SourceKindTag::Video => &[Self::Mp4, Self::Text, Self::Ansi],
        }
    }

    fn video(self) -> Option<VideoFormat> {
        match self {
            Self::Png => None,
            Self::Mp4 => Some(VideoFormat::Mp4),
            Self::Text => Some(VideoFormat::Text),
            Self::Ansi => Some(VideoFormat::Ansi),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceKindTag {
    Image,
    Video,
}

/// `<folder>/<stem>_ascii.<ext>` next to the input.
pub fn default_output_path(input: &Path, format: OutputFormat) -> PathBuf {
    let stem = input.file_stem().map_or_else(|| "output".into(), |s| s.to_string_lossy().into_owned());
    input.with_file_name(format!("{stem}_ascii.{}", format.extension()))
}

pub struct VideoSource {
    pub info: MediaInfo,
    /// Preview position, seconds.
    pub time: f64,
    pub start: f64,
    pub end: f64,
}

pub enum SourceKind {
    Image(Arc<DynamicImage>),
    Video(VideoSource),
}

pub struct Source {
    pub path: PathBuf,
    pub kind: SourceKind,
}

impl Source {
    fn tag(&self) -> SourceKindTag {
        match self.kind {
            SourceKind::Image(_) => SourceKindTag::Image,
            SourceKind::Video(_) => SourceKindTag::Video,
        }
    }

    fn display_size(&self) -> Size2 {
        match &self.kind {
            SourceKind::Image(image) => Size2 { w: image.width(), h: image.height() },
            SourceKind::Video(v) => v.info.display,
        }
    }
}

struct Loaded {
    source: Source,
    original: Option<ColorImage>,
}

#[derive(Default)]
struct Preview {
    original: Option<TextureHandle>,
    ascii: Option<TextureHandle>,
    grid: Option<GridSize>,
    output_size: Option<Size2>,
    reduced: bool,
    rejected: Vec<char>,
    error: Option<String>,
    waiting: bool,
}

enum Export {
    Video { job: ExportJob, progress: Progress },
    Still(Receiver<Result<PathBuf, String>>),
}

#[derive(Clone)]
enum Status {
    Info(String),
    Error(String),
}

/// A short-lived message in the corner of the window, shown when an export finishes.
struct Toast {
    kind: ToastKind,
    text: String,
    /// Shown on hover, e.g. the full path of the saved file.
    detail: Option<String>,
    created: Instant,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ToastKind {
    Success,
    Error,
}

impl ToastKind {
    /// How long the toast stays up; errors linger so they can be read.
    fn lifetime(self) -> Duration {
        match self {
            ToastKind::Success => Duration::from_secs(6),
            ToastKind::Error => Duration::from_secs(12),
        }
    }
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct Persisted {
    settings: Settings,
    preset_name: String,
    ffmpeg_dir: Option<PathBuf>,
    view_mode: ViewMode,
    video_quality: EncodeQuality,
}

pub struct AsciiPainterApp {
    settings: Settings,
    /// Settings as of the last change check.
    seen_settings: Settings,
    preset_name: String,
    user_presets: Vec<Preset>,
    preset_errors: Vec<String>,
    new_preset_name: String,
    ffmpeg_dir: Option<PathBuf>,
    ffmpeg: Result<FfmpegPaths, String>,
    source: Option<Source>,
    loading: Option<Receiver<Result<Loaded, String>>>,
    worker: PreviewWorker,
    generation: u64,
    dirty_since: Option<Instant>,
    preview: Preview,
    viewer: ViewerState,
    output_path: String,
    output_format: OutputFormat,
    video_quality: EncodeQuality,
    export: Option<Export>,
    status: Option<Status>,
    toast: Option<Toast>,
    confirm_overwrite: Option<PathBuf>,
}

fn install_fonts(ctx: &egui::Context) {
    // egui's default fonts lack braille and block elements; fall back to the embedded font.
    let mut fonts = FontDefinitions::default();
    fonts
        .font_data
        .insert("dejavu-mono".into(), Arc::new(FontData::from_static(ascii_painter_core::glyphs::FONT_BYTES)));
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        fonts.families.entry(family).or_default().push("dejavu-mono".into());
    }
    ctx.set_fonts(fonts);
}

fn locate_ffmpeg(dir: Option<&Path>) -> Result<FfmpegPaths, String> {
    FfmpegPaths::locate(dir).map_err(|e| e.to_string())
}

fn texture_options() -> TextureOptions {
    TextureOptions {
        magnification: TextureFilter::Nearest,
        minification: TextureFilter::Linear,
        mipmap_mode: Some(TextureFilter::Linear),
        ..TextureOptions::LINEAR
    }
}

fn color_image(rgba: &RgbaImage, max_side: u32) -> ColorImage {
    let size = Size2 { w: rgba.width(), h: rgba.height() };
    let fit = fit_within(size, max_side);
    if fit == size {
        return ColorImage::from_rgba_unmultiplied([size.w as usize, size.h as usize], rgba.as_raw());
    }
    let small = image::imageops::resize(rgba, fit.w, fit.h, image::imageops::FilterType::Triangle);
    ColorImage::from_rgba_unmultiplied([fit.w as usize, fit.h as usize], small.as_raw())
}

fn upload(ctx: &egui::Context, slot: &mut Option<TextureHandle>, name: &str, image: ColorImage) {
    match slot {
        Some(texture) => texture.set(image, texture_options()),
        None => *slot = Some(ctx.load_texture(name, image, texture_options())),
    }
}

fn load_source(path: PathBuf, ffmpeg: Result<FfmpegPaths, String>, max_side: u32) -> Result<Loaded, String> {
    match classify(&path) {
        InputKind::Still => {
            let image = load_oriented(&path).map_err(|e| e.to_string())?;
            let original = color_image(&image.to_rgba8(), max_side);
            Ok(Loaded { source: Source { path, kind: SourceKind::Image(Arc::new(image)) }, original: Some(original) })
        }
        InputKind::Animated | InputKind::Other => {
            let paths = ffmpeg.map_err(|e| format!("Not a still image, and video support needs ffmpeg: {e}"))?;
            let info = probe(&paths, &path).map_err(|e| format!("Unsupported file: {e}"))?;
            let end = info.duration.unwrap_or(0.0);
            let video = VideoSource { info, time: 0.0, start: 0.0, end };
            Ok(Loaded { source: Source { path, kind: SourceKind::Video(video) }, original: None })
        }
    }
}

fn export_still(
    image: &DynamicImage,
    settings: Settings,
    format: OutputFormat,
    path: &Path,
) -> Result<PathBuf, String> {
    let rgb = flatten(image.clone(), settings.color.bg);
    let converter = Converter::new(settings).map_err(|e| e.to_string())?;
    let grid = converter.grid_for(Size2 { w: rgb.width(), h: rgb.height() });
    let frame = converter.convert(ConvertInput { image: &rgb, grid, previous: None });
    let written = match format {
        OutputFormat::Png => converter.render(&frame).save(path).map_err(|e| e.to_string()),
        OutputFormat::Text => std::fs::write(path, converter.to_text(&frame)).map_err(|e| e.to_string()),
        OutputFormat::Ansi => std::fs::write(path, converter.to_ansi(&frame)).map_err(|e| e.to_string()),
        OutputFormat::Mp4 => Err("an image cannot be exported as MP4".into()),
    };
    written.map(|()| path.to_path_buf())
}

fn format_duration(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

impl AsciiPainterApp {
    /// `initial_file` comes from the command line ("Open with…").
    pub fn new(cc: &eframe::CreationContext<'_>, initial_file: Option<PathBuf>) -> Self {
        let mut app = Self::restore(cc);
        if let Some(path) = initial_file {
            app.open(&cc.egui_ctx, path);
        }
        app
    }

    fn restore(cc: &eframe::CreationContext<'_>) -> Self {
        install_fonts(&cc.egui_ctx);
        theme::apply(&cc.egui_ctx);
        let persisted: Persisted = cc.storage.and_then(|s| eframe::get_value(s, eframe::APP_KEY)).unwrap_or_default();
        let (user_presets, preset_errors) = presets::user_dir().map(|d| presets::load_user(&d)).unwrap_or_default();
        let settings = persisted.settings.sanitized();
        let preset_name = if persisted.preset_name.is_empty() { "Classic".into() } else { persisted.preset_name };
        Self {
            seen_settings: settings.clone(),
            settings,
            preset_name,
            user_presets,
            preset_errors,
            new_preset_name: String::new(),
            ffmpeg: locate_ffmpeg(persisted.ffmpeg_dir.as_deref()),
            ffmpeg_dir: persisted.ffmpeg_dir,
            source: None,
            loading: None,
            worker: PreviewWorker::spawn(cc.egui_ctx.clone()),
            generation: 0,
            dirty_since: None,
            preview: Preview::default(),
            viewer: ViewerState::with_mode(persisted.view_mode),
            output_path: String::new(),
            output_format: OutputFormat::Png,
            video_quality: persisted.video_quality,
            export: None,
            status: None,
            toast: None,
            confirm_overwrite: None,
        }
    }

    fn mark_dirty(&mut self) {
        self.dirty_since = Some(Instant::now());
    }

    fn max_texture_side(ctx: &egui::Context) -> u32 {
        ctx.input(|i| i.max_texture_side) as u32
    }

    // ---- input ---------------------------------------------------------------------------

    fn open(&mut self, ctx: &egui::Context, path: PathBuf) {
        let (tx, rx) = channel();
        let ffmpeg = self.ffmpeg.clone();
        let max_side = Self::max_texture_side(ctx).min(ORIGINAL_MAX_SIDE);
        let repaint = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(load_source(path, ffmpeg, max_side));
            repaint.request_repaint();
        });
        self.loading = Some(rx);
        self.status = Some(Status::Info("Loading…".into()));
    }

    fn poll_loading(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.loading else { return };
        let Ok(result) = rx.try_recv() else { return };
        self.loading = None;
        match result {
            Ok(loaded) => {
                let name = loaded.source.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!("ASCII Painter — {name}")));
                self.preview = Preview::default();
                if let Some(original) = loaded.original {
                    upload(ctx, &mut self.preview.original, "original", original);
                }
                let tag = loaded.source.tag();
                if !OutputFormat::options(tag).contains(&self.output_format) {
                    self.output_format = OutputFormat::options(tag)[0];
                }
                self.output_path = default_output_path(&loaded.source.path, self.output_format).display().to_string();
                self.status = match &loaded.source.kind {
                    SourceKind::Video(v) if v.info.hdr => {
                        Some(Status::Info("HDR video: colours are not tone-mapped and may look washed out.".into()))
                    }
                    _ => None,
                };
                self.source = Some(loaded.source);
                self.viewer.reset_view();
                self.dirty_since = Some(Instant::now() - DEBOUNCE);
            }
            Err(e) => self.status = Some(Status::Error(e)),
        }
    }

    fn handle_drops(&mut self, ctx: &egui::Context) {
        let dropped = ctx
            .input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).find(|p| !p.as_os_str().is_empty()));
        if let Some(path) = dropped {
            self.open(ctx, path);
        }
    }

    // ---- preview -------------------------------------------------------------------------

    fn schedule_preview(&mut self, ctx: &egui::Context) {
        let Some(since) = self.dirty_since else { return };
        let Some(source) = &self.source else { return };
        let waited = since.elapsed();
        if waited < DEBOUNCE {
            ctx.request_repaint_after(DEBOUNCE - waited);
            return;
        }
        let preview_source = match &source.kind {
            SourceKind::Image(image) => PreviewSource::Image(image.clone()),
            SourceKind::Video(v) => match &self.ffmpeg {
                Ok(paths) => PreviewSource::Video {
                    paths: paths.clone(),
                    path: source.path.clone(),
                    info: v.info.clone(),
                    time: v.time,
                },
                Err(_) => return,
            },
        };
        self.dirty_since = None;
        self.generation += 1;
        self.preview.waiting = true;
        self.worker.request(PreviewRequest {
            generation: self.generation,
            settings: self.settings.clone(),
            source: preview_source,
            max_side: Self::max_texture_side(ctx),
        });
    }

    fn poll_preview(&mut self, ctx: &egui::Context) {
        let Some(response) = self.worker.poll() else { return };
        if response.generation == self.generation {
            self.preview.waiting = false;
        }
        match response.result {
            Ok(out) => {
                let max_side = Self::max_texture_side(ctx);
                upload(ctx, &mut self.preview.ascii, "ascii", color_image(&out.ascii, max_side));
                if let Some(original) = out.original {
                    let rgba = DynamicImage::ImageRgb8(original).to_rgba8();
                    upload(ctx, &mut self.preview.original, "original", color_image(&rgba, max_side));
                }
                self.preview.grid = Some(out.grid);
                self.preview.output_size = Some(out.output_size);
                self.preview.reduced = out.reduced;
                self.preview.rejected = out.rejected;
                self.preview.error = None;
            }
            Err(e) => self.preview.error = Some(e),
        }
    }

    // ---- export --------------------------------------------------------------------------

    /// Why export is impossible right now, if it is.
    fn export_blocker(&self) -> Option<String> {
        let source = self.source.as_ref()?;
        if self.export.is_some() {
            return Some("An export is running.".into());
        }
        if self.output_path.trim().is_empty() {
            return Some("Choose an output file.".into());
        }
        if let SourceKind::Video(_) = source.kind {
            if let Err(e) = &self.ffmpeg {
                return Some(e.clone());
            }
            if self.output_format == OutputFormat::Mp4 {
                let converter = Converter::new(self.settings.clone()).ok()?;
                let size = mp4_size(&converter, converter.grid_for(source.display_size()));
                if size.w > MAX_SIDE || size.h > MAX_SIDE {
                    return Some(format!(
                        "MP4 frames would be {}×{} px; the limit is {MAX_SIDE}. Lower columns or font size.",
                        size.w, size.h
                    ));
                }
            }
        }
        None
    }

    fn output_file(&self) -> PathBuf {
        PathBuf::from(self.output_path.trim()).with_extension(self.output_format.extension())
    }

    fn request_export(&mut self) {
        let path = self.output_file();
        if path.exists() {
            self.confirm_overwrite = Some(path);
        } else {
            self.start_export(path);
        }
    }

    fn start_export(&mut self, path: PathBuf) {
        let Some(source) = &self.source else { return };
        self.output_path = path.display().to_string();
        let settings = self.settings.clone();
        match &source.kind {
            SourceKind::Image(image) => {
                let (tx, rx) = channel();
                let (image, format) = (image.clone(), self.output_format);
                std::thread::spawn(move || {
                    let _ = tx.send(export_still(&image, settings, format, &path));
                });
                self.export = Some(Export::Still(rx));
            }
            SourceKind::Video(video) => {
                let (Ok(paths), Some(format)) = (&self.ffmpeg, self.output_format.video()) else { return };
                let range = TimeRange {
                    start: (video.start > 0.0).then_some(video.start),
                    end: (video.end < video.info.duration.unwrap_or(f64::MAX) - 1e-3).then_some(video.end),
                };
                let request = ExportRequest {
                    input: source.path.clone(),
                    output: path,
                    format,
                    settings,
                    range,
                    info: video.info.clone(),
                    quality: self.video_quality,
                };
                let total = video.info.frames_in(range);
                let job = ExportJob::start(paths.clone(), request);
                self.export = Some(Export::Video { job, progress: Progress { total, ..Progress::default() } });
            }
        }
        self.status = Some(Status::Info("Exporting…".into()));
    }

    fn poll_export(&mut self, ctx: &egui::Context) {
        let finished = match &mut self.export {
            None => return,
            Some(Export::Still(rx)) => rx.try_recv().ok(),
            Some(Export::Video { job, progress }) => {
                let mut finished = None;
                for event in job.poll() {
                    match event {
                        ExportEvent::Progress(p) => *progress = p,
                        ExportEvent::Finished(result) => finished = Some(result.map_err(|e| e.to_string())),
                    }
                }
                finished
            }
        };
        match finished {
            Some(Ok(path)) => {
                self.status = Some(Status::Info(format!("Saved {}", path.display())));
                let name = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into());
                self.toast = Some(Toast {
                    kind: ToastKind::Success,
                    text: format!("Export saved: {name}"),
                    detail: Some(path.display().to_string()),
                    created: Instant::now(),
                });
                self.export = None;
            }
            Some(Err(e)) => {
                if e == "cancelled" {
                    self.status = Some(Status::Info("Export cancelled.".into()));
                } else {
                    self.toast = Some(Toast {
                        kind: ToastKind::Error,
                        text: format!("Export failed: {e}"),
                        detail: None,
                        created: Instant::now(),
                    });
                    self.status = Some(Status::Error(e));
                }
                self.export = None;
            }
            None => ctx.request_repaint_after(Duration::from_millis(100)),
        }
    }

    /// Draws the export result toast in the top-right corner, below the toolbar. It fades out
    /// at the end of its lifetime and can be dismissed early.
    fn toast_ui(&mut self, ctx: &egui::Context) {
        const FADE: Duration = Duration::from_millis(400);
        let Some(toast) = &self.toast else { return };
        let lifetime = toast.kind.lifetime();
        let age = toast.created.elapsed();
        if age >= lifetime {
            self.toast = None;
            return;
        }
        let opacity = ((lifetime - age).as_secs_f32() / FADE.as_secs_f32()).min(1.0);
        let (color, icon) = match toast.kind {
            ToastKind::Success => (theme::SUCCESS, "✔"),
            ToastKind::Error => (theme::ERROR, "✖"),
        };
        let mut dismissed = false;
        egui::Area::new(egui::Id::unique("export-toast"))
            .anchor(egui::Align2::RIGHT_TOP, [-20.0, 108.0])
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.set_opacity(opacity);
                theme::toast_frame(color).show(ui, |ui| {
                    ui.set_max_width(380.0);
                    ui.horizontal_top(|ui| {
                        ui.label(RichText::new(icon).color(color).strong());
                        let text = ui.label(&toast.text);
                        if let Some(detail) = &toast.detail {
                            text.on_hover_text(detail);
                        }
                        if ui.small_button("✕").clicked() {
                            dismissed = true;
                        }
                    });
                });
            });
        if dismissed {
            self.toast = None;
        } else {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
    }

    // ---- presets -------------------------------------------------------------------------

    fn apply_preset(&mut self, preset: Preset) {
        self.preset_name = preset.name;
        self.settings = preset.settings.sanitized();
    }

    fn preset_modified(&self) -> bool {
        presets::builtin()
            .iter()
            .chain(&self.user_presets)
            .find(|p| p.name == self.preset_name)
            .is_none_or(|p| p.settings != self.settings)
    }

    fn presets_ui(&mut self, ui: &mut Ui) {
        let builtin_presets = presets::builtin();
        let preset_count = builtin_presets.len() + self.user_presets.len();
        let label = if self.preset_modified() && self.preset_name != CUSTOM_PRESET {
            format!("{} (modified)", self.preset_name)
        } else {
            self.preset_name.clone()
        };
        let mut chosen = None;
        ui.horizontal(|ui| {
            ComboBox::from_id_salt("preset").selected_text(label).width(200.0).height(480.0).show_ui(ui, |ui| {
                for preset in &builtin_presets {
                    if ui.selectable_label(preset.name == self.preset_name, &preset.name).clicked() {
                        chosen = Some(preset.clone());
                    }
                }
                if !self.user_presets.is_empty() {
                    ui.separator();
                }
                for preset in &self.user_presets {
                    if ui.selectable_label(preset.name == self.preset_name, format!("★ {}", preset.name)).clicked() {
                        chosen = Some(preset.clone());
                    }
                }
            });
            // Step through the list in the combo's order; wraps around, and from a custom
            // (unlisted) state "next" starts at the first preset and "previous" at the last.
            let current = builtin_presets.iter().chain(&self.user_presets).position(|p| p.name == self.preset_name);
            let previous = theme::arrow_button(ui, theme::Arrow::Left, 34.0).on_hover_text("Previous preset");
            let next = theme::arrow_button(ui, theme::Arrow::Right, 34.0).on_hover_text("Next preset");
            let target = if next.clicked() {
                Some(current.map_or(0, |i| (i + 1) % preset_count))
            } else if previous.clicked() {
                Some(current.map_or(preset_count - 1, |i| (i + preset_count - 1) % preset_count))
            } else {
                None
            };
            if let Some(index) = target {
                chosen = builtin_presets.iter().chain(&self.user_presets).nth(index).cloned();
            }
        });
        if let Some(preset) = chosen {
            self.apply_preset(preset);
        }

        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.new_preset_name).hint_text("preset name").desired_width(150.0));
            let name = self.new_preset_name.trim().to_owned();
            let clashes = presets::builtin().iter().any(|p| p.name.eq_ignore_ascii_case(&name));
            let save = ui.add_enabled(!name.is_empty() && !clashes, egui::Button::new("Save"));
            if save.clicked() {
                self.save_preset(name);
            }
            let is_user = self.user_presets.iter().any(|p| p.name == self.preset_name);
            if ui
                .add_enabled(is_user, egui::Button::new("Delete"))
                .on_hover_text("Delete the selected saved preset")
                .clicked()
            {
                self.delete_preset();
            }
        });
        for e in &self.preset_errors {
            ui.colored_label(ui.visuals().warn_fg_color, e);
        }
    }

    fn save_preset(&mut self, name: String) {
        let Some(dir) = presets::user_dir() else {
            self.status = Some(Status::Error("No configuration folder available for presets.".into()));
            return;
        };
        let preset = Preset { name: name.clone(), settings: self.settings.clone() };
        match presets::save_user(&dir, &preset) {
            Ok(_) => {
                self.user_presets.retain(|p| p.name != name);
                self.user_presets.push(preset);
                self.user_presets.sort_by(|a, b| a.name.cmp(&b.name));
                self.preset_name = name;
                self.new_preset_name.clear();
            }
            Err(e) => self.status = Some(Status::Error(format!("Saving preset failed: {e}"))),
        }
    }

    fn delete_preset(&mut self) {
        let Some(dir) = presets::user_dir() else { return };
        match presets::delete_user(&dir, &self.preset_name) {
            Ok(()) => {
                let name = std::mem::replace(&mut self.preset_name, CUSTOM_PRESET.into());
                self.user_presets.retain(|p| p.name != name);
            }
            Err(e) => self.status = Some(Status::Error(format!("Deleting preset failed: {e}"))),
        }
    }

    // ---- layout --------------------------------------------------------------------------

    fn toolbar(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            if theme::primary_button(ui, "📂 Open…", egui::vec2(124.0, 40.0)).clicked() {
                let picked = rfd::FileDialog::new()
                    .add_filter(
                        "Images and videos",
                        &[
                            "png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff", "tga", "qoi", "ico", "pnm",
                            "hdr", "mp4", "mov", "mkv", "webm", "avi", "m4v", "wmv", "flv", "mpg", "mpeg", "ts",
                            "apng", "avif", "heic",
                        ],
                    )
                    .add_filter("All files", &["*"])
                    .pick_file();
                if let Some(path) = picked {
                    self.open(ui.ctx(), path);
                }
            }
            match &self.source {
                Some(source) => {
                    ui.label(RichText::new(source.path.display().to_string()).weak());
                }
                None => {
                    ui.label(RichText::new("or drop a file anywhere").weak());
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Fit").on_hover_text("Reset zoom (or double-click the view)").clicked() {
                    self.viewer.reset_view();
                }
                ui.label(format!("{:.0}%", self.viewer.zoom * 100.0));
                ui.separator();
                for mode in ViewMode::ALL.iter().rev() {
                    ui.selectable_value(&mut self.viewer.mode, *mode, mode.label());
                }
            });
        });
    }

    fn side_panel(&mut self, ui: &mut Ui) {
        ui.label(RichText::new("Preset").strong());
        self.presets_ui(ui);
        ui.separator();
        let facts = PanelFacts {
            is_video: matches!(self.source.as_ref().map(|s| &s.kind), Some(SourceKind::Video(_))),
            rejected_chars: &self.preview.rejected,
        };
        settings_panel(ui, &mut self.settings, &facts);
        ui.separator();
        egui::CollapsingHeader::new("ffmpeg").default_open(self.ffmpeg.is_err()).show(ui, |ui| {
            match &self.ffmpeg {
                Ok(paths) => ui.label(RichText::new(paths.ffmpeg.display().to_string()).weak()),
                Err(e) => ui.colored_label(ui.visuals().error_fg_color, e),
            };
            ui.horizontal(|ui| {
                if ui.button("Choose folder…").clicked()
                    && let Some(dir) = rfd::FileDialog::new().pick_folder()
                {
                    self.ffmpeg = locate_ffmpeg(Some(&dir));
                    self.ffmpeg_dir = Some(dir);
                }
                if self.ffmpeg_dir.is_some() && ui.button("Use bundled").clicked() {
                    self.ffmpeg_dir = None;
                    self.ffmpeg = locate_ffmpeg(None);
                }
            });
        });
    }

    fn video_controls(&mut self, ui: &mut Ui) -> bool {
        let Some(Source { kind: SourceKind::Video(video), .. }) = &mut self.source else { return false };
        let duration = video.info.duration.unwrap_or(0.0).max(0.0);
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.label("Preview at");
            let slider = egui::Slider::new(&mut video.time, 0.0..=duration)
                .custom_formatter(|v, _| {
                    format_duration(Duration::from_secs_f64(v)) + &format!(".{:01}", (v.fract() * 10.0) as u32)
                })
                .step_by(0.1);
            ui.spacing_mut().slider_width = (ui.available_width() * 0.4).max(150.0);
            changed |= theme::slider(ui, slider).changed();
            ui.separator();
            ui.label("Export from");
            ui.add(egui::DragValue::new(&mut video.start).range(0.0..=video.end).speed(0.1).suffix(" s"));
            ui.label("to");
            ui.add(egui::DragValue::new(&mut video.end).range(video.start..=duration).speed(0.1).suffix(" s"));
            if ui.small_button("start here").clicked() {
                video.start = video.time.min(video.end);
            }
            if ui.small_button("end here").clicked() {
                video.end = video.time.max(video.start);
            }
        });
        changed
    }

    fn export_bar(&mut self, ui: &mut Ui) {
        ui.add_space(4.0);
        if self.video_controls(ui) {
            self.mark_dirty();
        }
        let Some(tag) = self.source.as_ref().map(Source::tag) else {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Open an image or video to begin.").weak());
                if self.loading.is_some() {
                    ui.spinner();
                }
                self.status_label(ui);
            });
            ui.add_space(4.0);
            return;
        };
        ui.horizontal(|ui| {
            let before = self.output_format;
            ComboBox::from_id_salt("format").selected_text(self.output_format.label()).show_ui(ui, |ui| {
                for &format in OutputFormat::options(tag) {
                    ui.selectable_value(&mut self.output_format, format, format.label());
                }
            });
            if before != self.output_format && !self.output_path.trim().is_empty() {
                self.output_path = self.output_file().display().to_string();
            }
            let mut reserved = 310.0;
            if self.output_format == OutputFormat::Mp4 {
                reserved += 150.0;
                ComboBox::from_id_salt("quality")
                    .selected_text(format!("Quality: {}", self.video_quality.label()))
                    .width(140.0)
                    .show_ui(ui, |ui| {
                        for quality in EncodeQuality::ALL {
                            ui.selectable_value(&mut self.video_quality, quality, quality.label())
                                .on_hover_text(quality.description());
                        }
                    })
                    .response
                    .on_hover_text("Draft exports fastest; High gives the highest fidelity but takes longest");
            }
            ui.add(egui::TextEdit::singleline(&mut self.output_path).desired_width(ui.available_width() - reserved));
            if ui.button("Browse…").clicked() {
                let current = self.output_file();
                let mut dialog =
                    rfd::FileDialog::new().add_filter(self.output_format.label(), &[self.output_format.extension()]);
                if let Some(dir) = current.parent() {
                    dialog = dialog.set_directory(dir);
                }
                if let Some(name) = current.file_name() {
                    dialog = dialog.set_file_name(name.to_string_lossy());
                }
                if let Some(path) = dialog.save_file() {
                    self.output_path = path.display().to_string();
                }
            }
            let blocker = self.export_blocker();
            if let Some(Export::Video { job, .. }) = &self.export {
                if ui.button("Cancel").clicked() {
                    job.cancel();
                }
            } else {
                let button = ui
                    .add_enabled_ui(blocker.is_none(), |ui| {
                        theme::primary_button(ui, "Export", egui::vec2(136.0, 42.0))
                    })
                    .inner;
                let button = match &blocker {
                    Some(reason) => button.on_disabled_hover_text(reason),
                    None => button,
                };
                if button.clicked() {
                    self.request_export();
                }
            }
        });

        match &self.export {
            Some(Export::Video { progress, .. }) => {
                let mut text = format!(
                    "{} / {} frames · {:.1} fps",
                    progress.done,
                    progress.total.map_or("?".into(), |t| t.to_string()),
                    progress.rate
                );
                if let Some(eta) = progress.eta {
                    text += &format!(" · {} left", format_duration(eta));
                }
                let bar = match progress.fraction() {
                    Some(f) => ProgressBar::new(f).text(text),
                    None => ProgressBar::new(0.0).animate(true).text(text),
                };
                ui.add(bar);
            }
            Some(Export::Still(_)) => {
                ui.add(ProgressBar::new(0.0).animate(true).text("Rendering…"));
            }
            None => {}
        }

        ui.horizontal(|ui| {
            if let (Some(grid), Some(size)) = (self.preview.grid, self.preview.output_size) {
                let mut info = format!("{}×{} characters", grid.cols, grid.rows);
                if self.output_format == OutputFormat::Png || self.output_format == OutputFormat::Mp4 {
                    info += &format!(" · {}×{} px", size.w, size.h);
                }
                if self.preview.reduced {
                    info += " · preview shown at reduced font size";
                }
                if let (OutputFormat::Ansi, Some(SourceKind::Video(v))) =
                    (self.output_format, self.source.as_ref().map(|s| &s.kind))
                {
                    let frames = v.info.frames_in(TimeRange { start: Some(v.start), end: Some(v.end) }).unwrap_or(0);
                    let bytes_per_cell = if self.settings.color.mode == ascii_painter_core::settings::ColorMode::Mono {
                        1.5
                    } else {
                        12.0
                    };
                    let mb = grid.cell_count() as f64 * bytes_per_cell * frames as f64 / 1e6;
                    info += &format!(" · ≈{mb:.0} MB");
                }
                ui.label(RichText::new(info).weak());
            }
            if self.preview.waiting || self.loading.is_some() {
                ui.spinner();
            }
            self.status_label(ui);
        });
        ui.add_space(4.0);
    }

    fn status_label(&self, ui: &mut Ui) {
        match &self.status {
            Some(Status::Info(text)) => {
                ui.label(text);
            }
            Some(Status::Error(text)) => {
                ui.colored_label(ui.visuals().error_fg_color, text);
            }
            None => {}
        }
    }

    fn central(&mut self, ui: &mut Ui) {
        if let Some(error) = &self.preview.error {
            egui::Frame::NONE.inner_margin(egui::Margin::same(12)).show(ui, |ui| {
                ui.colored_label(ui.visuals().error_fg_color, error);
            });
        }
        if self.source.is_none() {
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new("Drop an image or video here, or use Open…").size(20.0).weak());
            });
            return;
        }
        let aspect = self.source.as_ref().map_or(1.0, |s| {
            let d = s.display_size();
            d.w as f32 / d.h.max(1) as f32
        });
        viewer::show(
            ui,
            &mut self.viewer,
            ViewerImages { original: self.preview.original.as_ref(), ascii: self.preview.ascii.as_ref(), aspect },
        );
    }

    fn overwrite_dialog(&mut self, ctx: &egui::Context) {
        let Some(path) = self.confirm_overwrite.clone() else { return };
        let mut decision = None;
        egui::Window::new("Replace file?")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(format!("{} already exists.", path.display()));
                ui.horizontal(|ui| {
                    if theme::primary_button(ui, "Replace", egui::vec2(110.0, 38.0)).clicked() {
                        decision = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        decision = Some(false);
                    }
                });
            });
        match decision {
            Some(true) => {
                self.confirm_overwrite = None;
                self.start_export(path);
            }
            Some(false) => self.confirm_overwrite = None,
            None => {}
        }
    }
}

/// The window's own title bar: drag to move, double-click to maximize, and the three window buttons.
fn title_bar(ui: &mut Ui, maximized: bool) {
    const HEIGHT: f32 = 30.0;
    let ctx = ui.ctx().clone();
    // Allocated first, so the label and buttons added afterwards sit on top of it.
    let (rect, drag) = ui.allocate_exact_size(egui::vec2(ui.available_width(), HEIGHT), egui::Sense::click_and_drag());
    if drag.drag_started_by(egui::PointerButton::Primary) {
        ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
    if drag.double_clicked() {
        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
    }
    let row = egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::left_to_right(egui::Align::Center));
    ui.scope_builder(row, |ui| {
        ui.label(RichText::new("ASCII Painter").small().weak());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let size = egui::vec2(44.0, HEIGHT);
            if theme::window_button(ui, theme::WindowButton::Close, size).clicked() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            let toggle = if maximized { theme::WindowButton::Restore } else { theme::WindowButton::Maximize };
            if theme::window_button(ui, toggle, size).clicked() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
            }
            if theme::window_button(ui, theme::WindowButton::Minimize, size).clicked() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            }
        });
    });
}

/// Invisible grips around the window edge. An undecorated Windows window has no native resize
/// border, so a drag on one hands the resize to the OS.
fn resize_grips(ui: &mut Ui, window: egui::Rect) {
    use egui::{CursorIcon, Rect, pos2, vec2, viewport::ResizeDirection};
    const EDGE: f32 = 6.0;
    const CORNER: f32 = 12.0;
    let corner = vec2(CORNER, CORNER);
    let grips = [
        (
            ResizeDirection::North,
            Rect::from_min_max(
                pos2(window.left() + CORNER, window.top()),
                pos2(window.right() - CORNER, window.top() + EDGE),
            ),
            CursorIcon::ResizeNorth,
        ),
        (
            ResizeDirection::South,
            Rect::from_min_max(
                pos2(window.left() + CORNER, window.bottom() - EDGE),
                pos2(window.right() - CORNER, window.bottom()),
            ),
            CursorIcon::ResizeSouth,
        ),
        (
            ResizeDirection::West,
            Rect::from_min_max(
                pos2(window.left(), window.top() + CORNER),
                pos2(window.left() + EDGE, window.bottom() - CORNER),
            ),
            CursorIcon::ResizeWest,
        ),
        (
            ResizeDirection::East,
            Rect::from_min_max(
                pos2(window.right() - EDGE, window.top() + CORNER),
                pos2(window.right(), window.bottom() - CORNER),
            ),
            CursorIcon::ResizeEast,
        ),
        (ResizeDirection::NorthWest, Rect::from_min_size(window.left_top(), corner), CursorIcon::ResizeNorthWest),
        (
            ResizeDirection::NorthEast,
            Rect::from_min_size(window.right_top() - vec2(CORNER, 0.0), corner),
            CursorIcon::ResizeNorthEast,
        ),
        (
            ResizeDirection::SouthWest,
            Rect::from_min_size(window.left_bottom() - vec2(0.0, CORNER), corner),
            CursorIcon::ResizeSouthWest,
        ),
        (
            ResizeDirection::SouthEast,
            Rect::from_min_size(window.right_bottom() - corner, corner),
            CursorIcon::ResizeSouthEast,
        ),
    ];
    for (index, (direction, rect, cursor)) in grips.into_iter().enumerate() {
        let response = ui.interact(rect, ui.scope_id().with(("resize-grip", index)), egui::Sense::drag());
        if response.hovered() || response.dragged() {
            ui.ctx().set_cursor_icon(cursor);
        }
        if response.drag_started() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::BeginResize(direction));
        }
    }
}

impl eframe::App for AsciiPainterApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.handle_drops(&ctx);
        self.poll_loading(&ctx);
        self.poll_preview(&ctx);
        self.poll_export(&ctx);

        let maximized = ctx.input(|i| {
            let viewport = i.viewport();
            viewport.maximized.unwrap_or(false) || viewport.fullscreen.unwrap_or(false)
        });
        let style = ui.style().clone();
        egui::CentralPanel::default().frame(theme::window_frame()).show(ui, |ui| {
            let window = ui.max_rect();
            egui::Panel::top("titlebar").frame(theme::title_bar_frame()).show(ui, |ui| {
                title_bar(ui, maximized);
            });
            egui::Panel::top("toolbar").frame(theme::bar_frame(&style)).show(ui, |ui| self.toolbar(ui));
            egui::Panel::bottom("export").frame(theme::bar_frame(&style)).show(ui, |ui| self.export_bar(ui));
            egui::Panel::left("settings").frame(theme::side_frame(&style)).resizable(true).default_size(340.0).show(
                ui,
                |ui| {
                    ScrollArea::vertical().show(ui, |ui| self.side_panel(ui));
                },
            );
            egui::CentralPanel::default().frame(theme::canvas_frame()).show(ui, |ui| self.central(ui));
            if !maximized {
                resize_grips(ui, window);
            }
        });
        self.overwrite_dialog(&ctx);
        self.toast_ui(&ctx);

        if self.settings != self.seen_settings {
            self.settings = std::mem::take(&mut self.settings).sanitized();
            self.seen_settings = self.settings.clone();
            self.mark_dirty();
        }
        self.schedule_preview(&ctx);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        let persisted = Persisted {
            settings: self.settings.clone(),
            preset_name: self.preset_name.clone(),
            ffmpeg_dir: self.ffmpeg_dir.clone(),
            view_mode: self.viewer.mode,
            video_quality: self.video_quality,
        };
        eframe::set_value(storage, eframe::APP_KEY, &persisted);
    }
}

impl Drop for AsciiPainterApp {
    fn drop(&mut self) {
        // Do not leave ffmpeg running (or a half-written file) behind when the window closes.
        if let Some(Export::Video { job, .. }) = self.export.take() {
            job.cancel_and_wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_output_sits_next_to_input() {
        let out = default_output_path(Path::new("/clips/holiday.mov"), OutputFormat::Mp4);
        assert_eq!(out, Path::new("/clips/holiday_ascii.mp4"));
    }

    #[test]
    fn formats_match_input_kind() {
        assert!(!OutputFormat::options(SourceKindTag::Image).contains(&OutputFormat::Mp4));
        assert!(!OutputFormat::options(SourceKindTag::Video).contains(&OutputFormat::Png));
    }

    #[test]
    fn still_export_writes_every_format() {
        let dir = std::env::temp_dir().join(format!("ascii-painter-gui-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let image = DynamicImage::ImageRgba8(RgbaImage::from_pixel(40, 30, image::Rgba([200, 100, 50, 128])));
        for format in OutputFormat::options(SourceKindTag::Image) {
            let path = dir.join(format!("out.{}", format.extension()));
            export_still(&image, Settings::default(), *format, &path).unwrap();
            assert!(std::fs::metadata(&path).unwrap().len() > 0, "{format:?}");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
