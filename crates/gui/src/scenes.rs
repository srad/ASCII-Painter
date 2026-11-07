//! Deterministic in-memory fixtures for Rune's named snapshot scenes. No user files or
//! preferences are read or written. Image/ASCII surfaces use the production converter.
use crate::{
    app::{ExportState, Preview, Source, SourceKind, State, Toast, VideoSource},
    controls,
    export::{ExportIntent, ExportSource, OutputFormat},
    viewer::ViewerImage,
};
use ascii_painter_core::{ConvertInput, Converter, Size2};
use ascii_painter_video::{Fps, MediaInfo, Progress};
use image::{DynamicImage, Rgb, RgbImage};
use std::sync::Arc;

type Scene = (&'static str, fn(&mut State));
pub const SCENES: &[Scene] = &[
    ("empty", |_| {}),
    ("image", image),
    ("video", video),
    ("advanced", advanced),
    ("overwrite", overwrite),
    ("progress", progress),
    ("success", success),
    ("failure", failure),
];

fn pixels() -> RgbImage {
    RgbImage::from_fn(640, 420, |x, y| {
        let (u, v) = (x as f32 / 640.0, y as f32 / 420.0);
        let mountain = 0.63 - 0.15 * (u * 9.0).sin() - 0.08 * (u * 17.0).cos();
        let near = 0.82 - 0.10 * (u * 12.0 + 2.0).sin();
        if v > near {
            Rgb([24, (32.0 + 25.0 * u) as u8, 39])
        } else if v > mountain {
            Rgb([(62.0 + 50.0 * v) as u8, 61, (107.0 + 40.0 * u) as u8])
        } else if (u - 0.71).powi(2) + ((v - 0.29) * 0.66).powi(2) < 0.055f32.powi(2) {
            Rgb([255, 221, 171])
        } else {
            Rgb([(31.0 + 132.0 * v) as u8, (35.0 + 63.0 * v) as u8, (75.0 + 45.0 * v) as u8])
        }
    })
}

pub fn image(s: &mut State) {
    let rgb = pixels();
    let dynamic = DynamicImage::ImageRgb8(rgb.clone());
    s.prefs.settings.grid.columns = 100;
    s.prefs.settings.color.mode = ascii_painter_core::settings::ColorMode::Foreground;
    let converter = Converter::new(s.prefs.settings.clone()).unwrap();
    let size = Size2 { w: rgb.width(), h: rgb.height() };
    let grid = converter.grid_for(size);
    let frame = converter.convert(ConvertInput { image: &rgb, grid, previous: None });
    s.preview = Preview {
        original: Some(ViewerImage::new(dynamic.to_rgba8())),
        ascii: Some(ViewerImage::new(converter.render(&frame))),
        grid: Some(grid),
        output_size: Some(converter.output_size(grid)),
        ..Default::default()
    };
    s.source = Some(Source { path: "demo/sunset.png".into(), kind: SourceKind::Image(Arc::new(dynamic)) });
    s.set_output_path("demo/sunset_ascii.png".into());
    controls::sync_numbers(s);
}
pub fn video(s: &mut State) {
    image(s);
    let info = MediaInfo {
        display: Size2 { w: 640, h: 420 },
        fps: Fps { num: 30, den: 1 },
        frames: Some(360),
        duration: Some(12.0),
        has_audio: true,
        hdr: false,
    };
    s.source = Some(Source {
        path: "demo/sunset.mp4".into(),
        kind: SourceKind::Video(VideoSource { info, time: 3.2, start: 1.0, end: 10.0 }),
    });
    s.format = OutputFormat::Mp4;
    s.set_output_path("demo/sunset_ascii.mp4".into());
    controls::sync_numbers(s);
}
fn advanced(s: &mut State) {
    image(s);
    s.prefs.settings.charset.preset = ascii_painter_core::settings::CharsetPreset::Custom;
    s.prefs.settings.color.palette = ascii_painter_core::settings::PaletteKind::Custom;
    s.prefs.settings.color.chroma_key = Some(ascii_painter_core::Rgb::new(0, 255, 0));
    s.prefs.settings.tone.auto_levels = true;
    controls::sync_numbers(s);
}
fn overwrite(s: &mut State) {
    image(s);
    let SourceKind::Image(image) = &s.source.as_ref().unwrap().kind else { unreachable!() };
    // A simulated confirmation, not a claim that any file exists on the host.
    s.overwrite = Some(ExportIntent::fixture(ExportSource::Image(image.clone()), s.prefs.settings.clone()));
}
fn progress(s: &mut State) {
    video(s);
    s.export = Some(ExportState::fixture(Progress {
        done: 124,
        total: Some(270),
        rate: 42.5,
        eta: Some(std::time::Duration::from_secs(4)),
    }));
}
fn success(s: &mut State) {
    image(s);
    s.toast = Some(Toast { text: "Saved demo/sunset_ascii.png".into(), error: false, until: 6.0 });
}
fn failure(s: &mut State) {
    image(s);
    s.error("Could not export: the destination is not writable.");
}
