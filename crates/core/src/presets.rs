//! Built-in presets and user presets stored as TOML files.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::CoreError;
use crate::settings::{CharsetPreset, ColorMode, Detail, Dither, MappingMode, Settings};
use crate::types::Rgb;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    pub name: String,
    #[serde(default)]
    pub settings: Settings,
}

fn preset(name: &str, edit: impl FnOnce(&mut Settings)) -> Preset {
    let mut settings = Settings::default();
    edit(&mut settings);
    Preset { name: name.to_owned(), settings }
}

pub fn builtin() -> Vec<Preset> {
    vec![
        preset("Classic", |_| {}),
        preset("Detailed", |s| {
            s.grid.columns = 220;
            s.charset.preset = CharsetPreset::Detailed;
            s.tone.sharpen = 0.5;
        }),
        preset("Line Art", |s| {
            // Dark ink on paper: only contours become strokes.
            s.mapping = MappingMode::Shape;
            s.charset.preset = CharsetPreset::Printable;
            s.color.fg = Rgb::new(0x20, 0x20, 0x20);
            s.color.bg = Rgb::new(0xf5, 0xf5, 0xf0);
            s.tone.detail = Detail::EdgesOnly;
            s.tone.edge_boost = 0.5;
        }),
        preset("Colour Shapes", |s| {
            s.grid.columns = 200;
            s.mapping = MappingMode::Shape;
            s.charset.preset = CharsetPreset::Printable;
            s.color.mode = ColorMode::ForegroundBackground;
        }),
        preset("Colour ASCII", |s| {
            s.grid.columns = 200;
            s.charset.preset = CharsetPreset::Detailed;
            s.color.mode = ColorMode::Foreground;
            s.color.bg = Rgb::BLACK;
            s.tone.saturation = 1.2;
        }),
        preset("Braille HD", |s| {
            s.grid.columns = 220;
            s.mapping = MappingMode::Braille;
            s.dither = Dither::FloydSteinberg;
        }),
        preset("Colour Blocks", |s| {
            s.grid.columns = 240;
            s.mapping = MappingMode::HalfBlock;
            s.color.mode = ColorMode::ForegroundBackground;
        }),
        preset("Quadrant Colour", |s| {
            s.grid.columns = 200;
            s.mapping = MappingMode::Quadrant;
            s.color.mode = ColorMode::ForegroundBackground;
        }),
        preset("Retro Green Terminal", |s| {
            s.color.fg = Rgb::new(0x33, 0xff, 0x66);
            s.color.bg = Rgb::new(0x00, 0x14, 0x05);
            s.dither = Dither::Bayer4;
        }),
        preset("Newspaper", |s| {
            s.grid.columns = 200;
            s.charset.preset = CharsetPreset::Detailed;
            s.color.fg = Rgb::new(0x1a, 0x1a, 0x1a);
            s.color.bg = Rgb::new(0xf4, 0xf1, 0xe8);
            s.dither = Dither::Atkinson;
        }),
        preset("Video Stable", |s| {
            s.charset.preset = CharsetPreset::Detailed;
            s.color.mode = ColorMode::Foreground;
            s.color.bg = Rgb::BLACK;
            s.dither = Dither::Bayer8;
            s.temporal.hysteresis = 0.35;
            s.temporal.color_smoothing = 0.3;
        }),
    ]
}

/// Platform config directory for user presets.
pub fn user_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "ASCII Painter").map(|d| d.config_dir().join("presets"))
}

fn file_name(name: &str) -> String {
    let stem: String =
        name.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' || c == ' ' { c } else { '_' }).collect();
    format!("{}.toml", stem.trim())
}

/// Load every readable preset in `dir`, sorted by name. Unreadable files are reported, not fatal.
pub fn load_user(dir: &Path) -> (Vec<Preset>, Vec<String>) {
    let mut presets = Vec::new();
    let mut errors = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else { return (presets, errors) };
    for path in entries.flatten().map(|e| e.path()) {
        if path.extension().is_some_and(|e| e == "toml") {
            match fs::read_to_string(&path).map_err(CoreError::from).and_then(|t| Ok(toml::from_str::<Preset>(&t)?)) {
                Ok(p) => presets.push(Preset { settings: p.settings.sanitized(), ..p }),
                Err(e) => errors.push(format!("{}: {e}", path.display())),
            }
        }
    }
    presets.sort_by(|a, b| a.name.cmp(&b.name));
    (presets, errors)
}

pub fn save_user(dir: &Path, preset: &Preset) -> Result<PathBuf, CoreError> {
    fs::create_dir_all(dir)?;
    let path = dir.join(file_name(&preset.name));
    fs::write(&path, toml::to_string(preset)?)?;
    Ok(path)
}

pub fn delete_user(dir: &Path, name: &str) -> Result<(), CoreError> {
    fs::remove_file(dir.join(file_name(name)))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_names_are_unique_and_settings_valid() {
        let presets = builtin();
        for (i, p) in presets.iter().enumerate() {
            assert!(presets[i + 1..].iter().all(|q| q.name != p.name));
            assert_eq!(p.settings.clone().sanitized(), p.settings, "{}", p.name);
        }
    }

    #[test]
    fn user_presets_roundtrip_on_disk() {
        let dir = std::env::temp_dir().join(format!("ascii-painter-presets-{}", std::process::id()));
        let preset = builtin().remove(3);
        let saved = Preset { name: "Mine: v2/test".into(), ..preset };
        let path = save_user(&dir, &saved).unwrap();
        assert!(path.file_name().unwrap().to_str().unwrap().starts_with("Mine_ v2_test"));
        let (loaded, errors) = load_user(&dir);
        assert!(errors.is_empty());
        assert_eq!(loaded, vec![saved.clone()]);
        delete_user(&dir, &saved.name).unwrap();
        fs::remove_dir_all(&dir).unwrap();
    }
}
