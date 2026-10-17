//! Built-in presets and user presets stored as TOML files.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::CoreError;
use crate::settings::{
    CharsetPreset, ColorMode, Detail, Dither, MappingMode, PaletteKind, Polarity, RampOrder, Settings,
};
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
            // Sharpening amplifies grain too; smooth it first.
            s.tone.denoise = 0.2;
        }),
        preset("Line Art", |s| {
            // Dark ink on paper: only contours become strokes.
            s.mapping = MappingMode::Shape;
            s.charset.preset = CharsetPreset::Printable;
            s.color.fg = Rgb::new(0x20, 0x20, 0x20);
            s.color.bg = Rgb::new(0xf5, 0xf5, 0xf0);
            s.tone.detail = Detail::EdgesOnly;
            s.tone.edge_boost = 0.5;
            // Noise would otherwise become stray strokes.
            s.tone.denoise = 0.4;
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
            s.dither = Dither::BlueNoise;
            s.tone.denoise = 0.3;
            s.temporal.hysteresis = 0.35;
            s.temporal.color_smoothing = 0.3;
        }),
        preset("Green Screen", |s| {
            // Cut-out: the green backdrop becomes transparent in PNG / WebP.
            s.grid.columns = 160;
            s.charset.preset = CharsetPreset::Detailed;
            s.color.mode = ColorMode::Foreground;
            s.color.bg = Rgb::BLACK;
            s.color.chroma_key = Some(Rgb::new(0x00, 0xff, 0x00));
            s.render.transparent_background = true;
        }),
        preset("Game Boy LCD", |s| {
            s.grid.columns = 80;
            s.mapping = MappingMode::Quadrant;
            s.tone.saturation = 0.0;
            s.tone.contrast = 1.1;
            s.tone.auto_levels = true;
            s.color.mode = ColorMode::ForegroundBackground;
            s.color.palette = PaletteKind::Custom;
            s.color.custom_palette = vec![
                Rgb::new(0x0f, 0x38, 0x0f),
                Rgb::new(0x30, 0x62, 0x30),
                Rgb::new(0x8b, 0xac, 0x0f),
                Rgb::new(0x9b, 0xbc, 0x0f),
            ];
        }),
        preset("CGA Pop", |s| {
            s.grid.columns = 160;
            s.mapping = MappingMode::Quadrant;
            s.tone.contrast = 1.2;
            s.tone.saturation = 1.35;
            s.tone.auto_levels = true;
            s.color.mode = ColorMode::ForegroundBackground;
            s.color.palette = PaletteKind::Custom;
            s.color.custom_palette = vec![
                Rgb::new(0x00, 0x00, 0x00),
                Rgb::new(0x55, 0xff, 0xff),
                Rgb::new(0xff, 0x55, 0xff),
                Rgb::new(0xff, 0xff, 0xff),
            ];
        }),
        preset("Macintosh 1-Bit", |s| {
            s.grid.columns = 160;
            s.charset.preset = CharsetPreset::Custom;
            s.charset.custom = " █".to_owned();
            s.charset.order = RampOrder::AsWritten;
            s.dither = Dither::Atkinson;
            s.color.fg = Rgb::new(0x18, 0x18, 0x18);
            s.color.bg = Rgb::new(0xf5, 0xf2, 0xe8);
            s.tone.auto_levels = true;
            s.tone.contrast = 1.1;
            s.tone.denoise = 0.1;
        }),
        preset("Neon Outline", |s| {
            s.grid.columns = 180;
            s.mapping = MappingMode::Shape;
            s.charset.preset = CharsetPreset::Printable;
            s.color.fg = Rgb::new(0x31, 0xf7, 0xff);
            s.color.bg = Rgb::BLACK;
            s.tone.detail = Detail::EdgesOnly;
            s.tone.edge_boost = 0.85;
            s.tone.denoise = 0.4;
            s.render.transparent_background = true;
        }),
        preset("ANSI 16 Terminal", |s| {
            s.grid.columns = 80;
            s.grid.cell_aspect_override = Some(0.5);
            s.mapping = MappingMode::Shape;
            s.charset.preset = CharsetPreset::Printable;
            s.color.mode = ColorMode::ForegroundBackground;
            s.color.palette = PaletteKind::Ansi16;
            s.tone.auto_levels = true;
            s.tone.saturation = 1.2;
            s.tone.denoise = 0.15;
            s.tone.sharpen = 0.2;
        }),
        preset("Synthwave Poster", |s| {
            s.grid.columns = 120;
            s.mapping = MappingMode::Quadrant;
            s.tone.contrast = 1.25;
            s.tone.saturation = 1.5;
            s.tone.shadows = -0.15;
            s.tone.highlights = 0.05;
            s.tone.auto_levels = true;
            s.color.mode = ColorMode::ForegroundBackground;
            s.color.palette = PaletteKind::Custom;
            s.color.custom_palette = vec![
                Rgb::new(0x07, 0x00, 0x1f),
                Rgb::new(0x2d, 0x0a, 0x57),
                Rgb::new(0x7b, 0x2c, 0xbf),
                Rgb::new(0xff, 0x2a, 0x6d),
                Rgb::new(0x05, 0xd9, 0xe8),
                Rgb::new(0xf9, 0xf8, 0x71),
            ];
        }),
        preset("X-Ray Braille", |s| {
            s.grid.columns = 200;
            s.mapping = MappingMode::Braille;
            s.dither = Dither::BlueNoise;
            s.color.fg = Rgb::new(0xc7, 0xfa, 0xff);
            s.color.bg = Rgb::new(0x02, 0x08, 0x17);
            s.tone.polarity = Polarity::Inverted;
            s.tone.auto_levels = true;
            s.tone.contrast = 1.2;
            s.tone.gamma = 1.1;
            s.tone.denoise = 0.15;
            s.tone.edge_boost = 0.25;
        }),
        preset("Ink Stamp", |s| {
            s.grid.columns = 140;
            s.charset.preset = CharsetPreset::Custom;
            s.charset.custom = " █".to_owned();
            s.charset.order = RampOrder::AsWritten;
            s.dither = Dither::BlueNoise;
            s.color.fg = Rgb::new(0x2b, 0x17, 0x0f);
            s.color.bg = Rgb::new(0xf3, 0xe1, 0xb8);
            s.tone.contrast = 1.6;
            s.tone.black_point = 0.2;
            s.tone.white_point = 0.8;
            s.tone.denoise = 0.25;
        }),
        preset("Blueprint", |s| {
            s.grid.columns = 180;
            s.mapping = MappingMode::Shape;
            s.charset.preset = CharsetPreset::Custom;
            s.charset.custom = " ._-/\\|+X#".to_owned();
            s.color.fg = Rgb::new(0xbd, 0xef, 0xff);
            s.color.bg = Rgb::new(0x06, 0x2c, 0x4b);
            s.tone.detail = Detail::EdgesOnly;
            s.tone.contrast = 1.15;
            s.tone.gamma = 1.05;
            s.tone.edge_boost = 0.65;
            s.tone.denoise = 0.4;
        }),
        preset("Typewriter Sepia", |s| {
            s.grid.columns = 120;
            s.charset.preset = CharsetPreset::Custom;
            s.charset.custom = " .,:;i1tfLCG08@".to_owned();
            s.charset.order = RampOrder::AsWritten;
            s.dither = Dither::Atkinson;
            s.color.fg = Rgb::new(0x3a, 0x26, 0x18);
            s.color.bg = Rgb::new(0xea, 0xd8, 0xb0);
            s.tone.auto_levels = true;
            s.tone.contrast = 1.1;
            s.tone.gamma = 1.05;
            s.tone.denoise = 0.15;
            s.tone.sharpen = 0.2;
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
            assert!(
                presets[i + 1..].iter().all(|q| !q.name.eq_ignore_ascii_case(&p.name)),
                "duplicate built-in preset name: {}",
                p.name
            );
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
