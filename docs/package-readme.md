# ASCII Painter

Convert images and videos to ASCII art, with live preview and PNG, MP4, TXT and
ANSI export. Keep all four executables together: FFmpeg and FFprobe are included.
Fonts and the Rust libraries are compiled into the application.

## Windows x64

Extract the complete ZIP and run `ascii-painter.exe`. The application includes its
C runtime; a separate Visual C++ runtime installation is not required. The GUI
requires a working Direct3D 12 graphics driver and Windows system components.

## Linux

Extract the complete tar.gz and run `./ascii-painter`. The x64 release targets
Ubuntu 24.04 or a compatible distribution with glibc 2.39 or newer. It uses the
system C/GCC runtimes (`libc6`, `libgcc-s1`). Executable permissions are in the
archive.

The GUI also requires a Vulkan driver/loader, X11 or Wayland, keyboard libraries,
and a running desktop portal with a file chooser backend. On Ubuntu, common
runtime packages are `libvulkan1`, `libxkbcommon0`, `libxkbcommon-x11-0`,
`libwayland-client0`, `libwayland-cursor0`, `libx11-6`, `libxcb1`, and
`xdg-desktop-portal` plus the GTK, KDE or GNOME portal backend. Mesa systems also
need `mesa-vulkan-drivers`; other GPUs need their vendor's Vulkan driver.

## Using the application

1. Open or drop an image or video.
2. Choose a preset and adjust the conversion settings.
3. Compare the original and converted preview, then export.

For command-line usage, run `./ascii-painter-cli --help` (PowerShell:
`.\ascii-painter-cli.exe --help`). Linux example:

```sh
./ascii-painter-cli convert input.jpg -o output.png --cols 100
```

`BUILD-INFO.json` records the application version, architecture, toolchain,
source revision when available, and inspected binary dependencies. Runtime
diagnostics and crash backtraces are written to `ascii-painter.log` in the
application config folder (rotated at 1 MiB, keeping up to three files). Build
tools, Python and Rust are not needed to run this package.

## Licenses

ASCII Painter is declared MIT OR Apache-2.0. Bundled font and Rune notices are in
`LICENSES/`. FFmpeg and FFprobe run as separate programs; see
[the FFmpeg notice](LICENSES/ffmpeg-NOTICE.txt) for pinned builds and source links,
and [the accompanying GPL license](LICENSES/ffmpeg-GPL-3.0.txt).

![ASCII Painter](docs/screenshots/split-view.png)
