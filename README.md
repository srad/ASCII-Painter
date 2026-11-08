<div align="center">

# ASCII Painter

**Turn images and videos into high-quality ASCII art.**<br>
Live preview, before/after and split view, presets, PNG / MP4 / TXT / ANSI export

[![Rust](https://img.shields.io/badge/Rust-1.99.1-orange?logo=rust&logoColor=white)](https://www.rust-lang.org)
[![Rune](https://img.shields.io/badge/UI-Rune%20%2B%20Rune%20Kit-ac8cff)](https://github.com/anomalyco/rune)
[![Platforms](https://img.shields.io/badge/platform-Windows%20%7C%20Linux-lightgrey)](#build-from-source)
[![ffmpeg](https://img.shields.io/badge/ffmpeg-bundled-007808?logo=ffmpeg&logoColor=white)](#bundled-ffmpeg)
[![License](https://img.shields.io/badge/license-MIT%20%2F%20Apache--2.0-blue)](#licences)

<img src="docs/screenshots/split-view.png" alt="ASCII Painter Rune interface, rendered with the deterministic sunset fixture" width="860">

</div>

---

## Features

| | |
|---|---|
| **Mapping modes** | Brightness ramp, **shape matching** (picks the glyph whose strokes match the image structure), half blocks `▀▄`, quadrant blocks `▚`, braille `⣿` |
| **Tone** | Brightness, contrast, gamma, saturation, levels (black / white point, shadows, highlights) or **auto levels**, edge-preserving denoise, sharpen, edge boost, invert, edges-only line art |
| **Dithering** | Floyd-Steinberg, Atkinson, Bayer 4x4 / 8x8, blue noise (Bayer and blue noise are stable across video frames) |
| **Colour** | Mono, colour text, colour text + background; true colour, ANSI 256 / 16 or a custom palette; **chroma key** (green screen) and transparent-background PNG / WebP |
| **Video** | Temporal stability (glyph hysteresis, colour smoothing), trimming, audio kept in MP4, rotated / anamorphic sources handled |
| **Compare** | Original, ASCII, side-by-side and a draggable **split view**, with shared zoom and pan |
| **Interface** | Rune / Rune Kit, dark violet theme, large Open / Export controls, accent scrollbar, resizable settings panel and custom Windows / Linux titlebars |
| **Export** | PNG, MP4 (H.264 / AAC), plain `.txt`, ANSI `.ans`, with progress, ETA and cancel |
| **Presets** | 22 built-in (Classic, Detailed, Line Art, Game Boy LCD, ANSI 16 Terminal, Synthwave Poster, Green Screen, ...) plus your own, saved as TOML |

The preview uses the exact same renderer as the export, so what you see is what you get.

## Using it

Unpack the release archive and run `ascii-painter` (`ascii-painter.exe`). No separate ffmpeg installation is needed.

1. **Open** or drop an image / video (or pass it as an argument).
2. Pick a **preset** and fine-tune the settings; the preview updates live.
3. Compare in **Split** view: drag the divider, scroll to zoom, drag to pan, double-click to fit.
4. Hit **Export**.

### Command line

`ascii-painter-cli` sits next to the app:

```sh
ascii-painter-cli convert photo.jpg -o photo.png --preset "Colour Shapes" --cols 200
ascii-painter-cli convert clip.mp4 -o clip_ascii.mp4 --preset "Video Stable" --start 5 --end 15
ascii-painter-cli play clip.mp4          # plays in the terminal; q / Esc to stop
ascii-painter-cli presets
```

The output format follows the extension: `.png/.jpg/.webp/.bmp`, `.mp4`, `.txt`, `.ans`.
Video `.txt` frames are separated by form feeds; video `.ans` files play with `ascii-painter-cli play`.
With *transparent background* on, `.png` and `.webp` keep alpha; `.jpg`, `.bmp` and `.mp4` are composited onto the background colour.
`--settings file.toml` loads a settings file.

User presets are TOML files in the platform config folder (`.../ASCII Painter/presets`).

Application preferences use versioned `preferences.json` in the same config folder. Runtime diagnostics
and crash backtraces are written to `ascii-painter.log` in that folder (rotated at 1 MiB across up to
three files: `ascii-painter.log`, `.1` and `.2`). On the first
Rune launch, previous application settings and normal/maximized/fullscreen window geometry are
imported from `app.ron` when present; that file and preset files are left untouched. Corrupt or newer
JSON preferences produce a visible warning and are not overwritten. Exporting over an existing file
requires confirmation; conversion writes an adjacent staging file and replaces the destination only
after success. A destination changed during the export must be confirmed again.

## Build from source

The checked-in toolchain pins Rust 1.99.1. Rune is pinned to one upstream revision; the small,
vendored native-window patch is described in [`vendor/rune/UPSTREAM.md`](vendor/rune/UPSTREAM.md).
Linux requires a Vulkan-capable GPU/driver, X11 or Wayland, and the desktop file chooser portal
(`xdg-desktop-portal` plus a backend such as GTK, KDE or GNOME). Build prerequisites are listed below.
Tested configurations, dated evidence and remaining desktop / ARM64 acceptance gates are recorded in
[`docs/rune-validation.md`](docs/rune-validation.md).

```sh
cargo run -p ascii-painter-gui            # GUI (uses ffmpeg from PATH during development)
cargo run -p ascii-painter-cli -- --help
cargo test --workspace                    # video tests are skipped if ffmpeg is not on PATH
```

### Release builds

From Windows, build and verify both x64 packages in one foreground command:

```powershell
.\build.ps1 -All -WslDistro Ubuntu -WslUser root
```

The WSL distribution and user are configurable. `root` selects the existing Rust installation on
this development machine; omitting `-WslUser` uses the distribution's default user. WSL must expose
this checkout and provide an Ubuntu 24.04 x64/glibc 2.39 environment.

The version comes from the workspace's `Cargo.toml`. For version 0.2.1, upload these three files
from `dist/releases/0.2.1/` to GitHub:

```text
ascii-painter-0.2.1-windows-x64.zip
ascii-painter-0.2.1-linux-x64-glibc2.39.tar.gz
ascii-painter-0.2.1-SHA256SUMS.txt
```

Use `-DestinationRoot 'target/my-releases'` to change the output root within this workspace.
Other versions are preserved. Same-version replacement validates both packages first, keeps a
recovery backup during publication, and rolls back a failed replacement. Only one combined release
can use an output root at a time. Standalone builds remain available and write separate archives
under `dist/native/windows-x64/` or `dist/native/linux-<x64|arm64>/`:

```sh
.\build.ps1
bash ./build.sh
```

Both build environments need Python 3.12+, the pinned Rust toolchain and target. Windows needs
Visual Studio C++ Build Tools with x64 tools and a Windows SDK. Ubuntu build prerequisites are:

```sh
sudo apt install python3 build-essential pkg-config binutils libxkbcommon-dev libwayland-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev
rustup target add x86_64-unknown-linux-gnu --toolchain 1.99.1
```

Packages include the GUI, CLI, pinned FFmpeg/FFprobe, fonts embedded in the binaries, licenses,
usage instructions and `BUILD-INFO.json` with source/version/target and inspected dependencies.
Windows uses a static C runtime. Linux retains system C/GCC libraries, Vulkan drivers, X11/Wayland
and desktop portal requirements, detailed in the packaged README. Build tools are not needed to run
the packages. ARM64's existing native packaging path is retained; it is not covered by x64 validation.

Every package passes architecture/dependency inspection, archive checks, extracted CLI image/video
conversion with external FFmpeg excluded from discovery, and a headless GUI render before publication.
Checksums use relative filenames; verify them from the release directory with `sha256sum -c *SHA256SUMS.txt`.
These automated checks do not replace native desktop interaction acceptance.

See [`docs/release-builds.md`](docs/release-builds.md) for the complete release procedure,
configuration, validation details and recovery steps. GitHub upload is manual.

> **Windows note:** security software that intercepts new executables (e.g. Norton Data Protector) can make
> cargo fail with *"failed to link or copy ... build-script-build.exe ... Access denied"*. Exclude the
> repository's `target` folder from it. Paths it already blocked can stay blocked after that, which is why
> release builds use separate platform directories in `target/package`; set `CARGO_TARGET_DIR` to
> another directory inside the workspace when a fresh build location is needed.

## Project layout

| Crate | Purpose |
|---|---|
| `crates/core` | Conversion library and rotating file logger: tone, dithering, glyph matching, colour, renderers (no UI, no ffmpeg) |
| `crates/video` | ffmpeg discovery, probe, decode / encode pipes and the export pipeline |
| `crates/gui` | Rune / Rune Kit application (`ascii-painter`), owned media jobs and image compositor |
| `crates/cli` | Command-line tool (`ascii-painter-cli`) |
| `vendor/rune` | Pinned Rune core with custom Windows / Linux window support |

## Limitations

- HDR video is converted without tone mapping and may look washed out (the app warns).
- `.txt` export drops colour.

## Bundled ffmpeg

Releases ship unmodified FFmpeg and FFprobe 9.0.2 as separate programs (GPL, includes libx264);
ASCII Painter runs them as child processes. Linux builds retain system C/GCC runtime dependencies.
See [`LICENSES/ffmpeg-NOTICE.txt`](LICENSES/ffmpeg-NOTICE.txt) for the exact builds and source links.

## Licences

ASCII Painter: MIT OR Apache-2.0. Bundled font: DejaVu Sans Mono ([`LICENSES/DejaVu-LICENSE.txt`](LICENSES/DejaVu-LICENSE.txt)).

Rune / Rune Kit: MIT. Inter and IBM Plex Mono: SIL Open Font License. See
[`LICENSES/Rune-NOTICE.txt`](LICENSES/Rune-NOTICE.txt) and the accompanying license texts.
