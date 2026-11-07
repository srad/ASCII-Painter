//! Application state and services. Views never perform conversion or filesystem I/O.
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicU32, Ordering},
    mpsc::{self, Receiver, TryRecvError},
};
use std::time::Duration;

use ascii_painter_core::{
    Converter, GridSize, Settings, Size2,
    load::{InputKind, classify, load_oriented},
    presets::{self, Preset},
};
use ascii_painter_video::{
    Cancellation, ExportEvent, ExportJob, ExportRequest, FfmpegPaths, MediaInfo, OutputPolicy, Progress, TimeRange,
    VideoError,
};
use rune::{AppHandle, TextEdit, Tx, Waker, WindowHandle};

use crate::controls::{ColorEditor, Number, NumericDraft};
use crate::export::{
    ExportIntent, ExportSource, OutputFormat, default_output_path, export_still, normalize_output_path,
};
use crate::jobs::Job;
use crate::preferences::{self, PreferenceWriter, Preferences};
use crate::viewer::{ViewerImage, ViewerState};
use crate::worker::{PreviewRequest, PreviewResponse, PreviewSource, PreviewWorker, fit_within};

#[cfg(test)]
#[path = "state_tests.rs"]
mod state_tests;

pub struct VideoSource {
    pub info: MediaInfo,
    pub time: f64,
    pub start: f64,
    pub end: f64,
}
pub enum SourceKind {
    Image(Arc<image::DynamicImage>),
    Video(VideoSource),
}
pub struct Source {
    pub path: PathBuf,
    pub kind: SourceKind,
}
impl Source {
    pub fn is_video(&self) -> bool {
        matches!(self.kind, SourceKind::Video(_))
    }
    pub fn display_size(&self) -> Size2 {
        match &self.kind {
            SourceKind::Image(image) => Size2 { w: image.width(), h: image.height() },
            SourceKind::Video(video) => video.info.display,
        }
    }
}
struct Loaded {
    source: Source,
    original: Option<Arc<ViewerImage>>,
}
#[derive(Default)]
pub struct Preview {
    pub original: Option<Arc<ViewerImage>>,
    pub ascii: Option<Arc<ViewerImage>>,
    pub grid: Option<GridSize>,
    pub output_size: Option<Size2>,
    pub reduced: bool,
    pub rejected: Vec<char>,
    pub waiting: bool,
    pub error: Option<String>,
}
pub struct Toast {
    pub text: String,
    pub error: bool,
    pub until: f64,
}
pub struct ExportState {
    job: Option<Job<Result<PathBuf, String>>>,
    progress_rx: Receiver<Progress>,
    pub progress: Option<Progress>,
    pub cancelling: bool,
}
impl ExportState {
    pub fn fixture(progress: Progress) -> Self {
        let (_, progress_rx) = mpsc::channel();
        Self { job: None, progress_rx, progress: Some(progress), cancelling: false }
    }
}

pub struct State {
    pub prefs: Preferences,
    seen_settings: Settings,
    pub presets: Vec<Preset>,
    pub preset_errors: Vec<String>,
    pub new_preset: TextEdit,
    pub charset: TextEdit,
    pub output: TextEdit,
    pub output_path: PathBuf,
    pub format: OutputFormat,
    pub ffmpeg: Result<FfmpegPaths, String>,
    pub source: Option<Source>,
    pub preview: Preview,
    pub viewer: ViewerState,
    pub viewport_size: [f32; 2],
    pub texture_limit: AtomicU32,
    pub numbers: HashMap<Number, NumericDraft>,
    pub color: Option<ColorEditor>,
    pub open_select: Option<String>,
    pub collapsed: HashSet<&'static str>,
    pub sidebar_start: f32,
    pub overwrite: Option<ExportIntent>,
    pub export: Option<ExportState>,
    pub status: Option<String>,
    pub toast: Option<Toast>,
    pub warning: Option<String>,
    pub native_dialog: bool,
    dialog_epoch: u64,
    pub now: f64,
    pub closing: bool,
    pub window: Option<WindowHandle>,
    pub app_handle: Option<AppHandle<State>>,
    pub startup_error: Option<String>,
    pub fatal: Arc<std::sync::Mutex<Option<String>>>,
    wake: Waker,
    worker: Option<PreviewWorker>,
    generation: u64,
    dirty_at: Option<f64>,
    load_generation: u64,
    loading: Vec<(u64, Job<Result<Loaded, String>>)>,
    pending_load: Option<(u64, PathBuf)>,
    cleanup: Option<Job<()>>,
    pub initial_file: Option<PathBuf>,
    pub services_enabled: bool,
    pub preference_path: Option<PathBuf>,
    pub preset_dir: Option<PathBuf>,
    writer: Option<PreferenceWriter>,
    saved: Preferences,
    save_at: Option<f64>,
    requested_title: Option<String>,
}

impl Default for State {
    /// Side-effect-free defaults for deterministic tooling and tests.
    fn default() -> Self {
        let prefs = Preferences::default();
        let mut state = Self {
            seen_settings: prefs.settings.clone(),
            saved: prefs.clone(),
            charset: TextEdit::new(prefs.settings.charset.custom.clone()),
            prefs,
            presets: presets::builtin(),
            preset_errors: Vec::new(),
            new_preset: TextEdit::default(),
            output: TextEdit::default(),
            output_path: PathBuf::new(),
            format: OutputFormat::Png,
            ffmpeg: Err("FFmpeg has not been located".into()),
            source: None,
            preview: Preview::default(),
            viewer: ViewerState::default(),
            viewport_size: [1400.0, 900.0],
            texture_limit: AtomicU32::new(4096),
            numbers: HashMap::new(),
            color: None,
            open_select: None,
            collapsed: HashSet::new(),
            sidebar_start: 340.0,
            overwrite: None,
            export: None,
            status: None,
            toast: None,
            warning: None,
            native_dialog: false,
            dialog_epoch: 0,
            now: 0.0,
            closing: false,
            window: None,
            app_handle: None,
            startup_error: None,
            fatal: Arc::default(),
            wake: Arc::new(|| {}),
            worker: None,
            generation: 0,
            dirty_at: None,
            load_generation: 0,
            loading: Vec::new(),
            pending_load: None,
            cleanup: None,
            initial_file: None,
            services_enabled: false,
            preference_path: None,
            preset_dir: None,
            writer: None,
            save_at: None,
            requested_title: None,
        };
        crate::controls::sync_numbers(&mut state);
        state
    }
}

impl State {
    pub fn native(initial_file: Option<PathBuf>) -> Self {
        let mut s = Self { services_enabled: true, ..Default::default() };
        let isolated = std::env::var_os("ASCII_PAINTER_CONFIG_DIR").map(PathBuf::from);
        if let Some(path) = isolated.as_ref().map(|p| p.join("preferences.json")).or_else(preferences::config_path) {
            let legacy = if isolated.is_some() { None } else { preferences::legacy_path() };
            let loaded = preferences::load(&path, legacy.as_deref());
            s.prefs = loaded.value;
            s.warning = loaded.warning;
            if loaded.writable {
                s.preference_path = Some(path);
            }
        }
        s.viewer.mode = s.prefs.view_mode;
        s.viewport_size = [s.prefs.window.size.0, s.prefs.window.size.1];
        s.seen_settings = s.prefs.settings.clone();
        s.saved = s.prefs.clone();
        s.charset = TextEdit::new(s.prefs.settings.charset.custom.clone());
        s.preset_dir = isolated.map(|p| p.join("presets")).or_else(presets::user_dir);
        s.reload_presets();
        s.locate_ffmpeg();
        s.initial_file = initial_file;
        crate::controls::sync_numbers(&mut s);
        s
    }

    pub fn blocked(&self) -> bool {
        self.closing || self.native_dialog || self.overwrite.is_some() || self.color.is_some()
    }
    pub fn is_video(&self) -> bool {
        self.source.as_ref().is_some_and(Source::is_video)
    }
    pub fn video(&self) -> Option<&VideoSource> {
        match &self.source.as_ref()?.kind {
            SourceKind::Video(v) => Some(v),
            _ => None,
        }
    }
    pub fn video_mut(&mut self) -> Option<&mut VideoSource> {
        match &mut self.source.as_mut()?.kind {
            SourceKind::Video(v) => Some(v),
            _ => None,
        }
    }
    pub fn locate_ffmpeg(&mut self) {
        self.ffmpeg = FfmpegPaths::locate(self.prefs.ffmpeg_dir.as_deref()).map_err(|e| e.to_string());
    }

    pub fn dirty(&mut self) {
        self.generation += 1;
        self.dirty_at = Some(self.now + 0.15);
        if let Some(worker) = &self.worker {
            worker.cancel();
        }
        self.preview.waiting = self.source.is_some() && self.worker.is_some();
    }

    pub fn open(&mut self, path: PathBuf) {
        if self.blocked() {
            return;
        }
        self.load_generation += 1;
        self.generation += 1;
        self.dirty_at = None;
        for (_, job) in &self.loading {
            job.cancel.cancel();
        }
        if let Some(worker) = &self.worker {
            worker.cancel();
        }
        self.status = Some(format!("Opening {}…", path.display()));
        self.preview.waiting = false;
        self.pending_load = Some((self.load_generation, path));
        self.start_pending_load();
    }

    fn start_pending_load(&mut self) {
        if self.closing || !self.loading.is_empty() {
            return;
        }
        let Some((generation, path)) = self.pending_load.take() else { return };
        let ffmpeg = self.ffmpeg.clone();
        let limit = self.texture_limit.load(Ordering::Relaxed).min(4096);
        self.loading.push((
            generation,
            Job::spawn("load-media", self.wake.clone(), move |cancel| load_source(path, ffmpeg, limit, cancel)),
        ));
    }

    pub fn set_output_path(&mut self, path: PathBuf) {
        self.output = TextEdit::new(path.to_string_lossy().into_owned());
        self.output_path = path;
    }
    pub fn output_edited(&mut self) {
        self.output_path = PathBuf::from(self.output.text.trim());
    }
    pub fn change_format(&mut self, format: OutputFormat) {
        self.format = format;
        if !self.output_path.as_os_str().is_empty() {
            match normalize_output_path(&self.output_path, format) {
                Ok(path) => self.set_output_path(path),
                Err(error) => self.error(error),
            }
        }
    }
    pub fn error(&mut self, error: impl Into<String>) {
        let text = error.into();
        self.status = Some(text.clone());
        self.toast = Some(Toast { text, error: true, until: self.now + 12.0 });
    }

    pub fn export_blocker(&self) -> Option<String> {
        if !self.loading.is_empty() || self.pending_load.is_some() {
            return Some("Wait until the media finishes opening".into());
        }
        if self.source.is_none() {
            return Some("Open an image or video first".into());
        }
        if self.export.is_some() {
            return Some("An export is already running".into());
        }
        if self.output_path.as_os_str().is_empty() {
            return Some("Choose an output file".into());
        }
        if self.is_video() && self.ffmpeg.is_err() {
            return self.ffmpeg.as_ref().err().cloned();
        }
        if let Some(source) = &self.source
            && self.format == OutputFormat::Mp4
        {
            let c = Converter::new(self.prefs.settings.clone()).ok()?;
            let size = ascii_painter_video::pipeline::mp4_size(&c, c.grid_for(source.display_size()));
            if size.w > ascii_painter_video::encode::MAX_SIDE || size.h > ascii_painter_video::encode::MAX_SIDE {
                return Some("MP4 dimensions are too large. Reduce columns or font size.".into());
            }
        }
        None
    }
    pub fn request_export(&mut self) {
        if self.blocked() {
            return;
        }
        crate::controls::commit_all(self);
        match normalize_output_path(&self.output_path, self.format) {
            Ok(path) => self.set_output_path(path),
            Err(error) => {
                self.error(error);
                return;
            }
        }
        if let Some(reason) = self.export_blocker() {
            self.error(reason);
            return;
        }
        let source = self.source.as_ref().unwrap();
        let export_source = match &source.kind {
            SourceKind::Image(image) => ExportSource::Image(image.clone()),
            SourceKind::Video(video) => ExportSource::Video {
                paths: self.ffmpeg.as_ref().unwrap().clone(),
                info: video.info.clone(),
                range: TimeRange {
                    start: (video.start > 0.0).then_some(video.start),
                    end: (video.end > 0.0).then_some(video.end),
                },
            },
        };
        match ExportIntent::capture(
            source.path.clone(),
            self.output_path.clone(),
            export_source,
            self.prefs.settings.clone(),
            self.format,
            self.prefs.video_quality,
        ) {
            Ok(intent) if intent.needs_confirmation() => {
                self.open_select = None;
                self.overwrite = Some(intent);
            }
            Ok(intent) => self.start_export(intent),
            Err(error) => self.error(error),
        }
    }
    pub fn confirm_export(&mut self) {
        if let Some(intent) = self.overwrite.take() {
            self.start_export(intent);
        }
    }
    fn start_export(&mut self, intent: ExportIntent) {
        let stage = match intent.stage() {
            Ok(stage) => stage,
            Err(error) => {
                self.error(error);
                return;
            }
        };
        let (progress_tx, progress_rx) = mpsc::channel();
        let wake = self.wake.clone();
        let job = Job::spawn("export", wake.clone(), move |cancel| {
            let result = match &intent.source {
                ExportSource::Image(image) => export_still(image, intent.settings.clone(), intent.format, &stage),
                ExportSource::Video { paths, info, range } => {
                    let request = ExportRequest {
                        input: intent.input.clone(),
                        output: stage.to_path_buf(),
                        info: info.clone(),
                        settings: intent.settings.clone(),
                        format: intent.format.video().unwrap(),
                        range: *range,
                        quality: intent.quality,
                    };
                    let job = ExportJob::start_cancellable_with_policy(
                        paths.clone(),
                        request,
                        cancel.clone(),
                        OutputPolicy::ReplaceExisting,
                    ); // replacement applies only to the GUI's owned outer stage
                    let mut result = Err("Export worker stopped without a result".into());
                    while let Some(event) = job.recv() {
                        match event {
                            ExportEvent::Progress(progress) => {
                                let _ = progress_tx.send(progress);
                                wake();
                            }
                            ExportEvent::Finished(finished) => {
                                result = finished.map(|_| ()).map_err(|e| e.to_string());
                                break;
                            }
                        }
                    }
                    job.wait();
                    result
                }
            };
            result?;
            if cancel.is_cancelled() {
                return Err(VideoError::Cancelled.to_string());
            }
            intent.publish(stage, &cancel)
        });
        self.export = Some(ExportState { job: Some(job), progress_rx, progress: None, cancelling: false });
        self.status = Some("Exporting…".into());
    }
    pub fn cancel_export(&mut self) {
        if let Some(export) = &mut self.export {
            export.cancelling = true;
            if let Some(job) = &export.job {
                job.cancel.cancel();
            }
            self.status = Some("Cancelling…".into());
        }
    }

    pub fn reload_presets(&mut self) {
        self.presets = presets::builtin();
        if let Some(dir) = &self.preset_dir {
            let (user, errors) = presets::load_user(dir);
            self.presets.extend(user);
            self.preset_errors = errors;
        }
    }
    pub fn choose_preset(&mut self, name: &str) {
        if let Some(preset) = self.presets.iter().find(|p| p.name == name) {
            self.prefs.settings = preset.settings.clone();
            self.prefs.preset_name = preset.name.clone();
            self.charset = TextEdit::new(self.prefs.settings.charset.custom.clone());
        }
    }
    pub fn cycle_preset(&mut self, direction: i32) {
        let len = self.presets.len();
        if len == 0 {
            return;
        }
        let next = match self.presets.iter().position(|p| p.name == self.prefs.preset_name) {
            Some(index) => (index as i64 + i64::from(direction)).rem_euclid(len as i64) as usize,
            None => {
                if direction < 0 {
                    len - 1
                } else {
                    0
                }
            }
        };
        self.choose_preset(&self.presets[next].name.clone());
    }
    pub fn save_preset(&mut self) {
        let name = self.new_preset.text.trim();
        if name.is_empty() || presets::builtin().iter().any(|p| p.name.eq_ignore_ascii_case(name)) {
            return;
        }
        if let Some(dir) = &self.preset_dir {
            let preset = Preset { name: name.to_owned(), settings: self.prefs.settings.clone() };
            match presets::save_user(dir, &preset) {
                Ok(_) => {
                    self.prefs.preset_name = preset.name;
                    self.new_preset = TextEdit::default();
                    self.reload_presets();
                }
                Err(error) => self.error(error.to_string()),
            }
        }
    }
    pub fn delete_preset(&mut self) {
        if presets::builtin().iter().any(|p| p.name == self.prefs.preset_name) {
            return;
        }
        if let Some(dir) = &self.preset_dir {
            match presets::delete_user(dir, &self.prefs.preset_name) {
                Ok(()) => {
                    self.reload_presets();
                    self.prefs.preset_name = "Custom".into();
                }
                Err(error) => self.error(error.to_string()),
            }
        }
    }

    pub fn begin_dialog(&mut self) -> Option<u64> {
        if self.blocked() {
            return None;
        }
        self.native_dialog = true;
        self.dialog_epoch += 1;
        self.open_select = None;
        Some(self.dialog_epoch)
    }
    pub fn finish_dialog(&mut self, epoch: u64) -> bool {
        if self.closing || !self.native_dialog || self.dialog_epoch != epoch {
            return false;
        }
        self.native_dialog = false;
        self.overwrite.is_none() && self.color.is_none()
    }

    fn accept_preview(&mut self, response: PreviewResponse) -> bool {
        if response.generation != self.generation || self.closing {
            return false;
        }
        self.preview.waiting = false;
        match response.result {
            Ok(output) => {
                self.preview.ascii = Some(output.ascii);
                if output.original.is_some() {
                    self.preview.original = output.original;
                }
                self.preview.grid = Some(output.grid);
                self.preview.output_size = Some(output.output_size);
                self.preview.reduced = output.reduced;
                self.preview.rejected = output.rejected;
                self.preview.error = None;
            }
            Err(error) => self.preview.error = Some(error),
        }
        true
    }
}

pub fn setup(s: &mut State, wake: Waker) {
    s.wake = wake.clone();
    if s.services_enabled {
        s.worker = Some(PreviewWorker::spawn(wake.clone()));
    }
    if let Some(path) = &s.preference_path {
        s.writer = Some(PreferenceWriter::start(path.clone(), wake));
    }
    if let Some(file) = s.initial_file.take() {
        s.open(file);
    }
}

pub fn poll(s: &mut State, now: f64, tx: &mut Tx) -> bool {
    s.now = now;
    if s.closing {
        if s.cleanup.as_ref().is_none_or(Job::is_finished) {
            s.cleanup.take();
            tx.quit();
        } else {
            tx.wake_after(Duration::from_millis(50));
        }
        return false;
    }
    if let Some(handle) = &s.app_handle
        && let Some(error) = handle.window_error()
    {
        *s.fatal.lock().unwrap() = Some(error.clone());
        s.startup_error = Some(error);
        close(s, tx);
        return true;
    }
    let mut changed = crate::controls::poll_edits(s);
    if s.prefs.settings != s.seen_settings {
        s.prefs.settings = s.prefs.settings.clone().sanitized();
        s.seen_settings = s.prefs.settings.clone();
        s.dirty();
        changed = true;
    }
    let mut index = 0;
    while index < s.loading.len() {
        let result = match s.loading[index].1.poll() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err("Media worker stopped unexpectedly".into())),
        };
        if let Some(result) = result {
            let (generation, _) = s.loading.remove(index);
            if generation == s.load_generation {
                match result {
                    Ok(loaded) => {
                        if !OutputFormat::options(loaded.source.is_video()).contains(&s.format) {
                            s.format = if loaded.source.is_video() { OutputFormat::Mp4 } else { OutputFormat::Png };
                        }
                        s.set_output_path(default_output_path(&loaded.source.path, s.format));
                        s.preview = Preview { original: loaded.original, ..Default::default() };
                        s.source = Some(loaded.source);
                        s.viewer.reset_view();
                        s.status = None;
                        s.dirty();
                    }
                    Err(error) => {
                        s.error(error);
                        s.dirty();
                    }
                }
                changed = true;
            }
        } else {
            index += 1;
        }
    }
    let title = s
        .source
        .as_ref()
        .and_then(|source| source.path.file_name())
        .map(|name| format!("ASCII Painter — {}", name.to_string_lossy()))
        .unwrap_or_else(|| "ASCII Painter".into());
    if s.requested_title.as_ref() != Some(&title) {
        tx.set_title(title.clone());
        s.requested_title = Some(title);
    }
    s.start_pending_load();
    if s.dirty_at.is_some_and(|at| now >= at) {
        s.dirty_at = None;
        if let (Some(worker), Some(source)) = (&s.worker, &s.source) {
            let source = match &source.kind {
                SourceKind::Image(image) => Some(PreviewSource::Image(image.clone())),
                SourceKind::Video(video) => s.ffmpeg.as_ref().ok().map(|paths| PreviewSource::Video {
                    paths: paths.clone(),
                    path: source.path.clone(),
                    info: video.info.clone(),
                    time: video.time,
                }),
            };
            if let Some(source) = source {
                worker.request(PreviewRequest {
                    generation: s.generation,
                    source_generation: s.load_generation,
                    settings: s.prefs.settings.clone(),
                    source,
                    max_side: s.texture_limit.load(Ordering::Relaxed),
                });
                s.preview.waiting = true;
            } else {
                s.preview.waiting = false;
            }
        }
    }
    if let Some(response) = s.worker.as_ref().and_then(PreviewWorker::poll) {
        changed |= s.accept_preview(response);
    }
    if s.worker.as_ref().is_some_and(PreviewWorker::is_finished) {
        s.worker.take();
        s.preview.waiting = false;
        s.preview.error =
            Some("Preview worker stopped unexpectedly. Reopen the application to restart previews.".into());
        changed = true;
    }
    let mut finished = None;
    if let Some(export) = &mut s.export {
        for progress in export.progress_rx.try_iter() {
            export.progress = Some(progress);
            changed = true;
        }
        finished = export.job.as_ref().and_then(|job| match job.poll() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err("Export worker stopped unexpectedly".into())),
        });
    }
    if let Some(result) = finished {
        s.export.take();
        changed = true;
        match result {
            Ok(path) => {
                let text = format!("Saved {}", path.display());
                s.status = Some(text.clone());
                s.toast = Some(Toast { text, error: false, until: now + 6.0 });
            }
            Err(error) if error == "cancelled" => s.status = Some("Export cancelled".into()),
            Err(error) => s.error(error),
        }
    }
    if s.toast.as_ref().is_some_and(|t| now >= t.until) {
        s.toast = None;
        changed = true;
    }
    if let Some(toast) = &s.toast {
        let fade_in = toast.until - now - 0.4;
        if fade_in > 0.0 {
            tx.wake_after(Duration::from_secs_f64(fade_in));
        } else {
            tx.wake_after(Duration::from_millis(16));
            changed = true;
        }
    }
    if let Some(writer) = &s.writer {
        let errors: Vec<_> = writer.errors.try_iter().collect();
        if !errors.is_empty() {
            s.warning = Some(format!("Preferences could not be saved: {}", errors.join("; ")));
            changed = true;
        }
    }
    if let Some(window) = &s.window {
        let g = window.geometry();
        if let Some(size) = g.normal_size {
            s.prefs.window.size = size;
        }
        s.prefs.window.position = g.normal_position;
        s.prefs.window.maximized = g.maximized;
        s.prefs.window.fullscreen = g.fullscreen;
    }
    s.prefs.view_mode = s.viewer.mode;
    if s.prefs != s.saved {
        s.saved = s.prefs.clone();
        s.save_at = Some(now + 0.5);
    }
    if s.save_at.is_some_and(|at| now >= at) {
        s.save_at = None;
        if let Some(writer) = &s.writer {
            writer.save(s.prefs.clone());
        }
    }
    crate::controls::sync_numbers(s);
    for due in [s.dirty_at, s.save_at, s.toast.as_ref().map(|t| t.until)].into_iter().flatten() {
        tx.wake_after(Duration::from_secs_f64((due - now).max(0.001)));
    }
    if !s.loading.is_empty()
        || s.preview.waiting
        || s.export.is_some()
        || (s.app_handle.is_some() && s.window.is_none())
    {
        tx.wake_after(Duration::from_millis(100));
    }
    changed
}

pub fn close(s: &mut State, tx: &mut Tx) {
    if s.closing {
        return;
    }
    crate::controls::commit_all(s);
    s.closing = true;
    s.pending_load = None;
    s.dialog_epoch += 1;
    s.overwrite = None;
    s.color = None;
    if let Some(window) = &s.window {
        let g = window.geometry();
        if let Some(size) = g.normal_size {
            s.prefs.window.size = size;
        }
        s.prefs.window.position = g.normal_position;
        s.prefs.window.maximized = g.maximized;
        s.prefs.window.fullscreen = g.fullscreen;
    }
    s.prefs.view_mode = s.viewer.mode;
    if let Some(writer) = &s.writer {
        writer.save(s.prefs.clone());
    }
    for (_, job) in &s.loading {
        job.cancel.cancel();
    }
    if let Some(worker) = &s.worker {
        worker.cancel();
    }
    if let Some(job) = s.export.as_ref().and_then(|e| e.job.as_ref()) {
        job.cancel.cancel();
    }
    let (loads, worker, export, writer) =
        (std::mem::take(&mut s.loading), s.worker.take(), s.export.take(), s.writer.take());
    s.cleanup = Some(Job::spawn("shutdown", s.wake.clone(), move |_| {
        drop(loads);
        drop(worker);
        drop(export);
        drop(writer);
    }));
    tx.wake_after(Duration::from_millis(50));
}

fn load_source(
    path: PathBuf,
    ffmpeg: Result<FfmpegPaths, String>,
    max_side: u32,
    cancel: Cancellation,
) -> Result<Loaded, String> {
    if cancel.is_cancelled() {
        return Err("cancelled".into());
    }
    match classify(&path) {
        InputKind::Still => {
            let image = load_oriented(&path).map_err(|e| e.to_string())?;
            if cancel.is_cancelled() {
                return Err("cancelled".into());
            }
            let size = fit_within(Size2 { w: image.width(), h: image.height() }, max_side);
            let rgba = image.to_rgba8();
            let rgba = if rgba.dimensions() == (size.w, size.h) {
                rgba
            } else {
                image::imageops::resize(&rgba, size.w, size.h, image::imageops::FilterType::Triangle)
            };
            Ok(Loaded {
                source: Source { path, kind: SourceKind::Image(Arc::new(image)) },
                original: Some(ViewerImage::new(rgba)),
            })
        }
        InputKind::Animated | InputKind::Other => {
            let paths = ffmpeg.map_err(|error| format!("Not a still image; video support needs FFmpeg: {error}"))?;
            let info =
                ascii_painter_video::probe::probe_cancellable(&paths, &path, &cancel).map_err(|e| e.to_string())?;
            let end = info.duration.unwrap_or(0.0);
            Ok(Loaded {
                source: Source { path, kind: SourceKind::Video(VideoSource { info, time: 0.0, start: 0.0, end }) },
                original: None,
            })
        }
    }
}
