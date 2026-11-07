//! Versioned application preferences, deliberately independent of either UI framework.
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;

use ascii_painter_core::Settings;
use ascii_painter_video::EncodeQuality;
use serde::{Deserialize, Serialize};

use crate::viewer::ViewMode;

const VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Preferences {
    pub schema_version: u32,
    pub settings: Settings,
    pub preset_name: String,
    pub ffmpeg_dir: Option<PathBuf>,
    pub view_mode: ViewMode,
    pub video_quality: EncodeQuality,
    pub window: WindowGeometry,
    pub sidebar_width: f32,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            schema_version: VERSION,
            settings: Settings::default(),
            preset_name: "Classic".into(),
            ffmpeg_dir: None,
            view_mode: ViewMode::default(),
            video_quality: EncodeQuality::default(),
            window: WindowGeometry::default(),
            sidebar_width: 340.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct WindowGeometry {
    pub position: Option<(i32, i32)>,
    pub size: (f32, f32),
    pub maximized: bool,
    pub fullscreen: bool,
}

impl Default for WindowGeometry {
    fn default() -> Self {
        Self { position: None, size: (1400.0, 900.0), maximized: false, fullscreen: false }
    }
}

impl Preferences {
    fn sanitize(&mut self) {
        self.settings = self.settings.clone().sanitized();
        if self.preset_name.trim().is_empty() {
            self.preset_name = "Classic".into();
        }
        self.sidebar_width =
            if self.sidebar_width.is_finite() { self.sidebar_width.clamp(280.0, 560.0) } else { 340.0 };
        let (w, h) = self.window.size;
        self.window.size = if w.is_finite() && h.is_finite() {
            (w.clamp(900.0, 7680.0), h.clamp(600.0, 4320.0))
        } else {
            WindowGeometry::default().size
        };
    }
}

pub fn config_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "ASCII Painter").map(|p| p.config_dir().join("preferences.json"))
}

pub fn legacy_path() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    return directories::BaseDirs::new().map(|d| d.config_dir().join("ASCII Painter/data/app.ron"));
    #[cfg(target_os = "linux")]
    return directories::BaseDirs::new().map(|d| d.data_dir().join("asciipainter/app.ron"));
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    None
}

pub struct LoadedPreferences {
    pub value: Preferences,
    pub warning: Option<String>,
    pub writable: bool,
}

pub fn load(path: &Path, legacy: Option<&Path>) -> LoadedPreferences {
    match std::fs::read(path) {
        Ok(bytes) => {
            let parsed =
                serde_json::from_slice::<serde_json::Value>(&bytes).map_err(|e| e.to_string()).and_then(|json| {
                    if json.get("schema_version").and_then(|v| v.as_u64()) != Some(u64::from(VERSION)) {
                        return Err("unsupported preference schema".into());
                    }
                    serde_json::from_value::<Preferences>(json).map_err(|e| e.to_string())
                });
            match parsed {
                Ok(mut value) => {
                    value.sanitize();
                    LoadedPreferences { value, warning: None, writable: true }
                }
                Err(error) => LoadedPreferences {
                    value: Preferences::default(),
                    writable: false,
                    warning: Some(format!(
                        "Preferences were not loaded ({error}). Using session defaults; the existing file will not be overwritten."
                    )),
                },
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let (mut value, warning) = legacy.map(import_legacy).unwrap_or_default();
            value.sanitize();
            LoadedPreferences { value, warning, writable: true }
        }
        Err(error) => LoadedPreferences {
            value: Preferences::default(),
            writable: false,
            warning: Some(format!("Cannot read preferences: {error}. Saving is disabled for this session.")),
        },
    }
}

// Compatibility-only data: do not bring the removed UI framework back to decode its RON.
#[derive(Deserialize, Default)]
#[serde(default)]
struct LegacyPreferences {
    settings: Settings,
    preset_name: String,
    ffmpeg_dir: Option<PathBuf>,
    view_mode: ViewMode,
    video_quality: EncodeQuality,
}
#[derive(Deserialize)]
struct Point {
    x: f32,
    y: f32,
}
#[derive(Deserialize)]
struct LegacyNormal {
    outer_position_pixels: Option<Point>,
    inner_size_points: Point,
}
#[derive(Deserialize)]
struct LegacyWindow {
    normal_geometry: LegacyNormal,
    maximized: bool,
    fullscreen: bool,
}

fn import_legacy(path: &Path) -> (Preferences, Option<String>) {
    let mut value = Preferences::default();
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return (value, None),
        Err(error) => return (value, Some(format!("Cannot import previous preferences: {error}"))),
    };
    let outer = match ron::from_str::<HashMap<String, String>>(&text) {
        Ok(outer) => outer,
        Err(error) => return (value, Some(format!("Previous preferences were left untouched: {error}"))),
    };
    let mut warnings = Vec::new();
    if let Some(app) = outer.get("app") {
        match ron::from_str::<LegacyPreferences>(app) {
            Ok(old) => {
                value.settings = old.settings;
                value.preset_name = old.preset_name;
                value.ffmpeg_dir = old.ffmpeg_dir;
                value.view_mode = old.view_mode;
                value.video_quality = old.video_quality;
            }
            Err(error) => warnings.push(format!("Previous application preferences could not be imported: {error}")),
        }
    }
    if let Some(window) = outer.get("window") {
        match ron::from_str::<LegacyWindow>(window) {
            Ok(old) => {
                value.window = WindowGeometry {
                    position: old
                        .normal_geometry
                        .outer_position_pixels
                        .filter(|p| p.x.is_finite() && p.y.is_finite())
                        .map(|p| (p.x as i32, p.y as i32)),
                    size: (old.normal_geometry.inner_size_points.x, old.normal_geometry.inner_size_points.y),
                    maximized: old.maximized,
                    fullscreen: old.fullscreen,
                }
            }
            Err(_) => warnings
                .push("Previous window geometry could not be imported; application preferences were retained.".into()),
        }
    }
    (value, (!warnings.is_empty()).then(|| warnings.join("\n")))
}

/// Exactly one writer, coalescing queued snapshots; dropping it flushes and joins.
pub struct PreferenceWriter {
    sender: Option<Sender<Preferences>>,
    pub errors: Receiver<String>,
    thread: Option<JoinHandle<()>>,
}
impl PreferenceWriter {
    pub fn start(path: PathBuf, wake: rune::Waker) -> Self {
        let (sender, inbox) = mpsc::channel::<Preferences>();
        let (errors_out, errors) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("preferences".into())
            .spawn(move || {
                while let Ok(mut value) = inbox.recv() {
                    for newer in inbox.try_iter() {
                        value = newer;
                    }
                    if let Err(error) = save_atomic(&path, &value) {
                        let _ = errors_out.send(error.to_string());
                        wake();
                    }
                }
            })
            .expect("start preference writer");
        Self { sender: Some(sender), errors, thread: Some(thread) }
    }
    pub fn save(&self, value: Preferences) {
        if let Some(sender) = &self.sender {
            let _ = sender.send(value);
        }
    }
}
impl Drop for PreferenceWriter {
    fn drop(&mut self) {
        self.sender.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn save_atomic(path: &Path, value: &Preferences) -> Result<(), Box<dyn std::error::Error>> {
    let parent = path.parent().ok_or("preference file has no parent")?;
    std::fs::create_dir_all(parent)?;
    let mut temp = tempfile::Builder::new().prefix(".preferences-").tempfile_in(parent)?;
    serde_json::to_writer_pretty(&mut temp, value)?;
    temp.write_all(b"\n")?;
    temp.as_file().sync_all()?;
    temp.persist(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_app_and_window_independently_without_touching_legacy_file() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = dir.path().join("app.ron");
        let json = dir.path().join("preferences.json");
        let mut entries = HashMap::new();
        entries.insert(
            "app",
            "(preset_name:\"Imported\",settings:(grid:(columns:123)),view_mode:SideBySide,video_quality:High)",
        );
        entries.insert("window", "(normal_geometry:(outer_position_pixels:Some((x:42.0,y:56.0)),scale_factor:1.5,inner_size_points:(x:1200.0,y:800.0)),fullscreen:false,maximized:true)");
        let bytes = ron::to_string(&entries).unwrap();
        std::fs::write(&legacy, &bytes).unwrap();
        let loaded = load(&json, Some(&legacy));
        assert!(loaded.writable);
        assert!(loaded.warning.is_none(), "{:?}", loaded.warning);
        assert_eq!(loaded.value.settings.grid.columns, 123);
        assert_eq!(loaded.value.preset_name, "Imported");
        assert_eq!(loaded.value.view_mode, ViewMode::SideBySide);
        assert_eq!(loaded.value.video_quality, EncodeQuality::High);
        assert_eq!(loaded.value.window.position, Some((42, 56)));
        assert_eq!(loaded.value.window.size, (1200.0, 800.0));
        assert!(loaded.value.window.maximized);
        assert_eq!(std::fs::read_to_string(&legacy).unwrap(), bytes);
        entries.insert("window", "broken");
        std::fs::write(&legacy, ron::to_string(&entries).unwrap()).unwrap();
        let partial = load(&json, Some(&legacy));
        assert_eq!(partial.value.settings.grid.columns, 123);
        assert!(partial.warning.is_some());
        assert!(partial.writable);
    }
    #[test]
    fn corrupt_or_newer_json_is_read_only_and_never_falls_back_to_legacy() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preferences.json");
        for bytes in ["not json", "{\"schema_version\":2}", "{\"settings\":{}}"] {
            std::fs::write(&path, bytes).unwrap();
            let loaded = load(&path, None);
            assert!(!loaded.writable);
            assert!(loaded.warning.is_some());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), bytes);
        }
    }
    #[test]
    fn writer_flushes_latest_snapshot_atomically_on_drop() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preferences.json");
        let writer = PreferenceWriter::start(path.clone(), std::sync::Arc::new(|| {}));
        for columns in [41, 53, 79] {
            let mut p = Preferences::default();
            p.settings.grid.columns = columns;
            writer.save(p);
        }
        drop(writer);
        assert_eq!(load(&path, None).value.settings.grid.columns, 79);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1, "no staging files remain");
    }
    #[test]
    fn geometry_and_sidebar_are_bounded() {
        let mut p = Preferences::default();
        p.window.size = (4.0, 2.0);
        p.sidebar_width = f32::NAN;
        p.sanitize();
        assert_eq!(p.window.size, (900.0, 600.0));
        assert_eq!(p.sidebar_width, 340.0);
        p.window.size = (f32::INFINITY, 700.0);
        p.sanitize();
        assert_eq!(p.window.size, (1400.0, 900.0));
    }
}
