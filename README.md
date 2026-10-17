<div align="center">

# ASCII Painter

**Turn images and videos into high-quality ASCII art.**<br>
Live preview, before/after and split view, presets, PNG / MP4 / TXT / ANSI export

[![Rust](https://img.shields.io/badge/Rust-1.88%2B-orange?logo=rust&logoColor=white)](https://www.rust-lang.org)
[![egui](https://img.shields.io/badge/UI-egui%20%2F%20eframe-5b8def)](https://github.com/emilk/egui)
[![Platforms](https://img.shields.io/badge/platform-Windows%20%7C%20Linux-lightgrey)](#build-from-source)
[![ffmpeg](https://img.shields.io/badge/ffmpeg-bundled-007808?logo=ffmpeg&logoColor=white)](#bundled-ffmpeg)
[![License](https://img.shields.io/badge/license-MIT%20%2F%20Apache--2.0-blue)](#licences)

<!-- Replace with a real screenshot, e.g. docs/screenshots/split-view.png -->
<img src="https://placehold.co/1200x675/16181d/9aa4b2?text=Screenshot%3A+split+view" alt="ASCII Painter - split view" width="860">

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
| **Interface** | Purpose-built dark theme with clear panel hierarchy, compact controls and an accented export action |
| **Export** | PNG, MP4 (H.264 / AAC), plain `.txt`, ANSI `.ans`, with progress, ETA and cancel |
| **Presets** | 22 built-in (Classic, Detailed, Line Art, Game Boy LCD, ANSI 16 Terminal, Synthwave Poster, Green Screen, ...) plus your own, saved as TOML |

The preview uses the exact same renderer as the export, so what you see is what you get.

## Using it

Unpack the release archive and run `ascii-painter` (`ascii-painter.exe`). ffmpeg is bundled, so there is nothing else to install.

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

## Build from source

Requires Rust 1.88+.

```sh
cargo run -p ascii-painter-gui            # GUI (uses ffmpeg from PATH during development)
cargo run -p ascii-painter-cli -- --help
cargo test --workspace                    # video tests are skipped if ffmpeg is not on PATH
```

### Release builds

```sh
.\build.ps1      # Windows -> dist\ascii-painter-windows-x64.zip
./build.sh       # Linux   -> dist/ascii-painter-linux-<x64|arm64>.tar.gz
```

Each script builds for the machine it runs on and produces a portable folder (and archive) with the GUI,
the CLI and a pinned, checksum-verified ffmpeg / ffprobe next to them. On Linux the GUI needs the usual
system libraries (see the header of `build.sh`).

> **Windows note:** security software that intercepts new executables (e.g. Norton Data Protector) can make
> cargo fail with *"failed to link or copy ... build-script-build.exe ... Access denied"*. Exclude the
> repository's `target` folder from it. Paths it already blocked can stay blocked after that, which is why
> `build.ps1` builds into `target\package`; set `CARGO_TARGET_DIR` to build somewhere else.

## Project layout

| Crate | Purpose |
|---|---|
| `crates/core` | Conversion library: tone, dithering, glyph matching, colour, renderers (no UI, no ffmpeg) |
| `crates/video` | ffmpeg discovery, probe, decode / encode pipes and the export pipeline |
| `crates/gui` | egui / eframe application (`ascii-painter`) |
| `crates/cli` | Command-line tool (`ascii-painter-cli`) |

## Limitations

- HDR video is converted without tone mapping and may look washed out (the app warns).
- `.txt` export drops colour.

## Bundled ffmpeg

Releases ship unmodified static builds of ffmpeg 9.0.2 as a separate program (GPL, includes libx264); ASCII Painter
runs it as a child process. See [`LICENSES/ffmpeg-NOTICE.txt`](LICENSES/ffmpeg-NOTICE.txt) for the exact builds and source links.

## Licences

ASCII Painter: MIT OR Apache-2.0. Bundled font: DejaVu Sans Mono ([`LICENSES/DejaVu-LICENSE.txt`](LICENSES/DejaVu-LICENSE.txt)).
