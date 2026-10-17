//! `ascii-painter-cli`: convert images/videos to ASCII files, or play them in the terminal.

mod play;

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use ascii_painter_core::load::{InputKind, classify, load_still};
use ascii_painter_core::presets::{self, Preset};
use ascii_painter_core::render::raster;
use ascii_painter_core::{ConvertInput, Converter, Settings, Size2};
use ascii_painter_video::{ExportEvent, ExportJob, ExportRequest, FfmpegPaths, TimeRange, VideoFormat, probe};
use clap::{Args, Parser, Subcommand};

#[derive(Parser)]
#[command(version, about = "Convert images and videos to ASCII art")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Convert a file. The output format follows the extension: .png/.jpg/.webp/.bmp (image),
    /// .mp4 (video), .txt (plain text), .ans (ANSI colour).
    Convert {
        input: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
        #[command(flatten)]
        style: Style,
        /// Start of the video section, in seconds.
        #[arg(long)]
        start: Option<f64>,
        /// End of the video section, in seconds.
        #[arg(long)]
        end: Option<f64>,
        /// Overwrite an existing output file.
        #[arg(long)]
        force: bool,
    },
    /// Play an .ans export, a video or an image in this terminal (q / Esc to stop).
    Play {
        input: PathBuf,
        #[command(flatten)]
        style: Style,
    },
    /// List built-in and saved presets.
    Presets,
}

#[derive(Args)]
struct Style {
    /// Preset name (see `presets`).
    #[arg(short, long)]
    preset: Option<String>,
    /// Settings TOML file (as saved by the GUI); overrides --preset.
    #[arg(long)]
    settings: Option<PathBuf>,
    /// Characters per line.
    #[arg(short, long)]
    cols: Option<u32>,
    /// Folder holding ffmpeg/ffprobe, if not bundled or on PATH.
    #[arg(long)]
    ffmpeg_dir: Option<PathBuf>,
}

impl Style {
    fn settings(&self) -> Result<Settings> {
        let mut settings = if let Some(file) = &self.settings {
            let text = std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
            match toml::from_str::<Preset>(&text) {
                Ok(preset) => preset.settings,
                Err(_) => toml::from_str::<Settings>(&text).with_context(|| format!("parsing {}", file.display()))?,
            }
        } else if let Some(name) = &self.preset {
            all_presets()
                .into_iter()
                .find(|p| p.name.eq_ignore_ascii_case(name))
                .with_context(|| format!("unknown preset {name:?}; run `ascii-painter-cli presets`"))?
                .settings
        } else {
            Settings::default()
        };
        if let Some(cols) = self.cols {
            settings.grid.columns = cols;
        }
        Ok(settings.sanitized())
    }

    fn ffmpeg(&self) -> Result<FfmpegPaths> {
        Ok(FfmpegPaths::locate(self.ffmpeg_dir.as_deref())?)
    }
}

fn all_presets() -> Vec<Preset> {
    let mut list = presets::builtin();
    if let Some(dir) = presets::user_dir() {
        list.extend(presets::load_user(&dir).0);
    }
    list
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum OutputKind {
    Image,
    Video(VideoFormat),
}

fn output_kind(path: &Path) -> Result<OutputKind> {
    if let Some(format) = VideoFormat::from_path(path) {
        return Ok(OutputKind::Video(format));
    }
    if image::ImageFormat::from_path(path).is_ok() {
        return Ok(OutputKind::Image);
    }
    bail!("unsupported output extension for {}", path.display())
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Convert { input, output, style, start, end, force } => {
            if output.exists() && !force {
                bail!("{} exists; pass --force to overwrite", output.display());
            }
            let settings = style.settings()?;
            let kind = output_kind(&output)?;
            match classify(&input) {
                InputKind::Still => convert_still(&input, &output, kind, settings),
                InputKind::Animated | InputKind::Other => {
                    let OutputKind::Video(format) = kind else {
                        bail!("video input needs a .mp4, .txt or .ans output");
                    };
                    convert_video(
                        &style.ffmpeg()?,
                        ExportPlan { input, output, format, settings, range: TimeRange { start, end } },
                    )
                }
            }
        }
        Command::Play { input, style } => play::run(&input, &style.settings()?, || style.ffmpeg()),
        Command::Presets => {
            for preset in all_presets() {
                println!("{}", preset.name);
            }
            Ok(())
        }
    }
}

fn convert_still(input: &Path, output: &Path, kind: OutputKind, settings: Settings) -> Result<()> {
    let background = settings.color.bg;
    let image = load_still(input, background).with_context(|| format!("loading {}", input.display()))?;
    let converter = Converter::new(settings)?;
    if !converter.rejected_chars().is_empty() {
        eprintln!("ignored unusable charset characters: {:?}", converter.rejected_chars());
    }
    let grid = converter.grid_for(Size2 { w: image.width(), h: image.height() });
    let frame = converter.convert(ConvertInput { image: &image, grid, previous: None });
    match kind {
        OutputKind::Image => {
            let rendered = converter.render(&frame);
            let keeps_alpha =
                matches!(image::ImageFormat::from_path(output), Ok(image::ImageFormat::Png | image::ImageFormat::WebP));
            if keeps_alpha {
                rendered.save(output)?;
            } else {
                // JPEG has no alpha channel and BMP viewers ignore it: composite onto the background.
                let flat = raster::flatten(rendered, background);
                image::DynamicImage::ImageRgba8(flat).into_rgb8().save(output)?;
            }
        }
        OutputKind::Video(VideoFormat::Text) => std::fs::write(output, converter.to_text(&frame))?,
        OutputKind::Video(VideoFormat::Ansi) => std::fs::write(output, converter.to_ansi(&frame))?,
        OutputKind::Video(VideoFormat::Mp4) => bail!("an image input cannot produce .mp4"),
    }
    eprintln!("wrote {} ({}×{} characters)", output.display(), grid.cols, grid.rows);
    Ok(())
}

struct ExportPlan {
    input: PathBuf,
    output: PathBuf,
    format: VideoFormat,
    settings: Settings,
    range: TimeRange,
}

fn convert_video(paths: &FfmpegPaths, plan: ExportPlan) -> Result<()> {
    let info = probe(paths, &plan.input).with_context(|| format!("probing {}", plan.input.display()))?;
    if info.hdr {
        eprintln!("note: HDR input; colours are not tone-mapped and may look washed out");
    }
    let job = ExportJob::start(
        paths.clone(),
        ExportRequest {
            input: plan.input,
            output: plan.output,
            format: plan.format,
            settings: plan.settings,
            range: plan.range,
            info,
        },
    );
    let mut stderr = std::io::stderr();
    while let Some(event) = job.recv() {
        match event {
            ExportEvent::Progress(p) => {
                let total = p.total.map_or_else(|| "?".to_owned(), |t| t.to_string());
                let eta = p.eta.map_or_else(String::new, |e| format!(", {}s left", e.as_secs()));
                let _ = write!(stderr, "\rframe {}/{total} ({:.1} fps{eta})   ", p.done, p.rate);
            }
            ExportEvent::Finished(result) => {
                eprintln!();
                let path = result?;
                eprintln!("wrote {}", path.display());
                return Ok(());
            }
        }
    }
    bail!("export ended without a result")
}
