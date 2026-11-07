//! Immutable GUI export intents. Only an owned adjacent staging file is writable by a job.
use ascii_painter_core::{ConvertInput, Converter, Settings, Size2, load::flatten};
use ascii_painter_video::output::{OutputDestination, OutputPolicy};
use ascii_painter_video::{EncodeQuality, FfmpegPaths, MediaInfo, TimeRange, VideoFormat};
use image::DynamicImage;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
            Self::Text => "Plain text",
            Self::Ansi => "ANSI colour",
        }
    }
    pub fn options(video: bool) -> &'static [Self] {
        if video { &[Self::Mp4, Self::Text, Self::Ansi] } else { &[Self::Png, Self::Text, Self::Ansi] }
    }
    pub fn video(self) -> Option<VideoFormat> {
        match self {
            Self::Png => None,
            Self::Mp4 => Some(VideoFormat::Mp4),
            Self::Text => Some(VideoFormat::Text),
            Self::Ansi => Some(VideoFormat::Ansi),
        }
    }
}

pub fn default_output_path(input: &Path, format: OutputFormat) -> PathBuf {
    let mut name = input.file_stem().unwrap_or_default().to_os_string();
    name.push(format!("_ascii.{}", format.extension()));
    input.with_file_name(name)
}

/// Normalize a filesystem path without losing a dialog's non-Unicode filename.
/// Only typed text is trimmed (by State::output_edited), not native paths.
pub fn normalize_output_path(path: &Path, format: OutputFormat) -> Result<PathBuf, String> {
    let raw = path.as_os_str().as_encoded_bytes();
    let trailing_dot_component = raw.ends_with(b"/.") || (cfg!(windows) && raw.ends_with(b"\\."));
    let trailing_separator = raw.last().is_some_and(|&last| last == b'/' || (cfg!(windows) && last == b'\\'));
    if path.as_os_str().is_empty()
        || path.file_name().is_none()
        || trailing_separator
        || trailing_dot_component
        || path.is_dir()
    {
        return Err("Choose an output file, not a blank path or directory.".into());
    }
    Ok(path.with_extension(format.extension()))
}

#[derive(Clone)]
pub enum ExportSource {
    Image(Arc<DynamicImage>),
    Video { paths: FfmpegPaths, info: MediaInfo, range: TimeRange },
}

pub struct ExportIntent {
    pub input: PathBuf,
    pub output: PathBuf,
    pub source: ExportSource,
    pub settings: Settings,
    pub format: OutputFormat,
    pub quality: EncodeQuality,
    destination: Option<OutputDestination>,
}

impl ExportIntent {
    pub(crate) fn fixture(source: ExportSource, settings: Settings) -> Self {
        Self {
            input: "demo/sunset.png".into(),
            output: "demo/sunset_ascii.png".into(),
            source,
            settings,
            format: OutputFormat::Png,
            quality: EncodeQuality::default(),
            destination: None,
        }
    }
    pub fn capture(
        input: PathBuf,
        output: PathBuf,
        source: ExportSource,
        settings: Settings,
        format: OutputFormat,
        quality: EncodeQuality,
    ) -> Result<Self, String> {
        let output = normalize_output_path(&output, format)?;
        let destination = OutputDestination::capture(&input, &output, OutputPolicy::ReplaceExisting)
            .map_err(|error| error.to_string())?;
        Ok(Self { input, output, source, settings, format, quality, destination: Some(destination) })
    }
    pub fn needs_confirmation(&self) -> bool {
        self.destination.as_ref().is_some_and(OutputDestination::exists)
    }
    pub fn stage(&self) -> Result<tempfile::TempPath, String> {
        self.destination
            .as_ref()
            .ok_or("Export intent has no destination")?
            .validate()
            .map_err(|error| error.to_string())?;
        let parent = self.output.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
        tempfile::Builder::new()
            .prefix(".ascii-painter-export-")
            .suffix(&format!(".{}", self.format.extension()))
            .tempfile_in(parent)
            .map(|temp| temp.into_temp_path())
            .map_err(|e| e.to_string())
    }
    pub fn publish(
        self,
        stage: tempfile::TempPath,
        cancel: &ascii_painter_video::Cancellation,
    ) -> Result<PathBuf, String> {
        self.destination
            .ok_or("Export intent has no destination")?
            .publish_cancellable(stage, cancel)
            .map_err(|error| error.to_string())
    }
}

pub fn export_still(image: &DynamicImage, settings: Settings, format: OutputFormat, path: &Path) -> Result<(), String> {
    let rgb = flatten(image.clone(), settings.color.bg);
    let converter = Converter::new(settings).map_err(|e| e.to_string())?;
    let grid = converter.grid_for(Size2 { w: rgb.width(), h: rgb.height() });
    let frame = converter.convert(ConvertInput { image: &rgb, grid, previous: None });
    match format {
        OutputFormat::Png => converter.render(&frame).save(path).map_err(|e| e.to_string()),
        OutputFormat::Text => std::fs::write(path, converter.to_text(&frame)).map_err(|e| e.to_string()),
        OutputFormat::Ansi => std::fs::write(path, converter.to_ansi(&frame)).map_err(|e| e.to_string()),
        OutputFormat::Mp4 => Err("an image cannot be exported as MP4".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn intent(input: &Path, output: &Path) -> ExportIntent {
        ExportIntent::capture(
            input.into(),
            output.into(),
            ExportSource::Image(Arc::new(DynamicImage::new_rgb8(4, 3))),
            Settings::default(),
            OutputFormat::Png,
            EncodeQuality::default(),
        )
        .unwrap()
    }
    #[test]
    fn cancellation_and_failures_leave_existing_destination_intact() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("input.png");
        let output = dir.path().join("out.png");
        std::fs::write(&input, b"source").unwrap();
        std::fs::write(&output, b"keep this").unwrap();
        let intent = intent(&input, &output);
        assert!(intent.needs_confirmation());
        let stage = intent.stage().unwrap();
        assert_eq!(stage.extension().unwrap(), "png");
        std::fs::write(&stage, b"partial").unwrap();
        let owned = stage.to_path_buf();
        drop(stage);
        assert_eq!(std::fs::read(&output).unwrap(), b"keep this");
        assert!(!owned.exists());
    }
    #[test]
    fn unconfirmed_new_destination_cannot_be_overwritten_by_a_late_export() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("input.png");
        let output = dir.path().join("out.png");
        std::fs::write(&input, b"source").unwrap();
        let intent = intent(&input, &output);
        assert!(!intent.needs_confirmation());
        let stage = intent.stage().unwrap();
        std::fs::write(&stage, b"new").unwrap();
        std::fs::write(&output, b"late file").unwrap();
        assert!(intent.publish(stage, &ascii_painter_video::Cancellation::default()).is_err());
        assert_eq!(std::fs::read(&output).unwrap(), b"late file");
    }
    #[test]
    fn replacement_is_published_only_if_confirmed_destination_is_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.png");
        let output = dir.path().join("out.png");
        std::fs::write(&input, b"source").unwrap();
        std::fs::write(&output, b"old").unwrap();
        let unchanged = intent(&input, &output);
        let stage = unchanged.stage().unwrap();
        std::fs::write(&stage, b"new").unwrap();
        unchanged.publish(stage, &ascii_painter_video::Cancellation::default()).unwrap();
        assert_eq!(std::fs::read(&output).unwrap(), b"new");
        let changed = intent(&input, &output);
        let stage = changed.stage().unwrap();
        std::fs::write(&stage, b"ours").unwrap();
        std::fs::write(&output, b"changed outside the app").unwrap();
        assert!(changed.publish(stage, &ascii_painter_video::Cancellation::default()).is_err());
        assert_eq!(std::fs::read(&output).unwrap(), b"changed outside the app");
    }
    #[test]
    fn source_path_and_hardlink_aliases_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.png");
        let alias = dir.path().join("alias.png");
        std::fs::write(&input, b"source").unwrap();
        std::fs::hard_link(&input, &alias).unwrap();
        assert!(OutputDestination::capture(&input, &input, OutputPolicy::ReplaceExisting).is_err());
        assert!(OutputDestination::capture(&input, &alias, OutputPolicy::ReplaceExisting).is_err());
        assert_eq!(std::fs::read(&input).unwrap(), b"source");
    }
    #[test]
    fn still_exports_match_the_core_renderer_for_all_formats() {
        let dir = tempfile::tempdir().unwrap();
        let image = DynamicImage::ImageRgb8(image::RgbImage::from_fn(32, 24, |x, y| {
            image::Rgb([(x * 7) as u8, (y * 9) as u8, 90])
        }));
        let mut settings = Settings::default();
        settings.grid.columns = 16;
        let rgb = flatten(image.clone(), settings.color.bg);
        let converter = Converter::new(settings.clone()).unwrap();
        let frame = converter.convert(ConvertInput {
            image: &rgb,
            grid: converter.grid_for(Size2 { w: 32, h: 24 }),
            previous: None,
        });
        for format in OutputFormat::options(false) {
            let path = dir.path().join(format!("out.{}", format.extension()));
            export_still(&image, settings.clone(), *format, &path).unwrap();
            match format {
                OutputFormat::Png => assert_eq!(image::open(path).unwrap().into_rgba8(), converter.render(&frame)),
                OutputFormat::Text => assert_eq!(std::fs::read_to_string(path).unwrap(), converter.to_text(&frame)),
                OutputFormat::Ansi => assert_eq!(std::fs::read_to_string(path).unwrap(), converter.to_ansi(&frame)),
                _ => unreachable!(),
            }
        }
        assert_eq!(default_output_path(Path::new("a.b.jpg"), OutputFormat::Png), PathBuf::from("a.b_ascii.png"));
        assert_eq!(OutputFormat::options(true), &[OutputFormat::Mp4, OutputFormat::Text, OutputFormat::Ansi]);
    }

    #[cfg(unix)]
    #[test]
    fn default_names_preserve_non_utf8_and_gui_stages_remain_private() {
        use std::os::unix::{ffi::OsStringExt, fs::PermissionsExt};
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join(std::ffi::OsString::from_vec(b"source-\xff.png".to_vec()));
        std::fs::write(&input, b"source").unwrap();
        let output = default_output_path(&input, OutputFormat::Png);
        assert_eq!(output.file_name().unwrap(), std::ffi::OsString::from_vec(b"source-\xff_ascii.png".to_vec()));
        let intent = intent(&input, &output);
        let stage = intent.stage().unwrap();
        assert_eq!(std::fs::metadata(&stage).unwrap().permissions().mode() & 0o777, 0o600);
        std::fs::write(&stage, b"private output").unwrap();
        intent.publish(stage, &ascii_painter_video::Cancellation::default()).unwrap();
        assert_eq!(std::fs::metadata(&output).unwrap().permissions().mode() & 0o777, 0o600);
    }
}
