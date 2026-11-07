# Release builds

The release scripts build portable application archives for manual upload to
GitHub. Run them from the checkout with Python 3.12 or newer. The shared,
standard-library-only implementation is [`tools/release_package.py`](../tools/release_package.py).

## Build environments

The combined command runs on Windows x64 and builds Windows natively, then Linux
through an installed WSL distribution. Both environments need their own Rust
installation and the toolchain pinned in `rust-toolchain.toml` (currently 1.99.1).
The WSL distribution must be able to access the same checkout through its Windows
mount. Network access is needed for uncached Rust dependencies and media tools.

| Environment | Required build tools | Release target |
|---|---|---|
| Windows x64 | Python available as `python` on PATH, rustup, Visual Studio C++ Build Tools with x64 tools and a Windows SDK | `x86_64-pc-windows-msvc` |
| Ubuntu 24.04 x64 / WSL | Python available as `python3`, rustup, C/C++ compiler, pkg-config, binutils, development libraries below | `x86_64-unknown-linux-gnu` |

Install the pinned toolchain and Windows target in PowerShell:

```powershell
rustup toolchain install 1.99.1
rustup target add x86_64-pc-windows-msvc --toolchain 1.99.1
```

Inside the selected WSL distribution and user account:

```sh
sudo apt install -y python3 build-essential pkg-config binutils libxkbcommon-dev libwayland-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev
rustup toolchain install 1.99.1
rustup target add x86_64-unknown-linux-gnu --toolchain 1.99.1
```

The Linux x64 builder must have **glibc 2.39 exactly**; the finished application
targets glibc 2.39 or newer. A newer builder can silently raise runtime
requirements, so preflight rejects it. Smoke rendering also requires a working
graphics driver: Direct3D 12 on Windows, Vulkan on Linux. Mesa-based Linux systems
can install `libvulkan1` and `mesa-vulkan-drivers`.

## Commands and configuration

Build the complete release from Windows:

```powershell
.\build.ps1 -All -WslDistro Ubuntu
```

The default WSL user must own a working Rust installation. On the current
development machine Rust is installed for `root`, selected explicitly with:

```powershell
.\build.ps1 -All -WslDistro Ubuntu -WslUser root
```

| Option | Default | Meaning |
|---|---|---|
| `-All` | Off | Build and verify both x64 archives, then publish a complete version directory and checksum manifest |
| `-DestinationRoot` | `dist` | Output root; must resolve to a subdirectory of the checkout |
| `-WslDistro` | `Ubuntu` | Existing WSL distribution for the combined build |
| `-WslUser` | Distribution default | WSL account for the combined build |
| `ASCII_PAINTER_DIST_DIR` | `dist` | Output root for `bash ./build.sh` |
| `CARGO_TARGET_DIR` | `target/package` | Build-cache root inside the checkout; each platform gets a separate subdirectory |

Standalone packages use the same checks:

```powershell
.\build.ps1 -DestinationRoot 'target/my-releases'
```

```sh
ASCII_PAINTER_DIST_DIR=target/my-releases bash ./build.sh
```

Standalone archives go under `<output>/native/windows-x64/` or
`<output>/native/linux-x64/`. They do not produce the combined release manifest.
Native Linux ARM64 packaging remains available through `bash ./build.sh` on an
ARM64 host, using `aarch64-unknown-linux-gnu` and that host's glibc version in the
filename. ARM64 is outside the Windows/Linux x64 validation evidence.

## Versions and release contents

Set `[workspace.package].version` in `Cargo.toml` and update the four
`ascii-painter-*` entries in `Cargo.lock` together. All four crates inherit this
version. Packaging reads Cargo metadata with `--locked` and requires the GUI and
CLI versions to agree. Full versions, including prerelease/build suffixes, are
preserved in filenames.

For version 0.2.0, the combined command produces exactly these upload assets:

```text
dist/releases/0.2.0/
  ascii-painter-0.2.0-windows-x64.zip
  ascii-painter-0.2.0-linux-x64-glibc2.39.tar.gz
  ascii-painter-0.2.0-SHA256SUMS.txt
```

Each archive contains one directory with the same base name as the archive:

- GUI, CLI, FFmpeg and FFprobe executables; keep all four together.
- A user README copied from [`package-readme.md`](package-readme.md), a screenshot,
  and the full `LICENSES` directory, including the FFmpeg GPL text and notice.
- `BUILD-INFO.json`: version, OS, architecture, Rust target/toolchain, UTC build
  time, Git revision and dirty state when available, static Windows CRT flag,
  inspected dependencies and pinned upstream media hashes.

Fonts and Rust libraries are embedded. Windows packages include the C runtime;
a separate Visual C++ redistributable is unnecessary. Linux packages use system
C/C++ libraries and desktop graphics/portal components; the packaged README lists
runtime requirements. Python, Rust and compilers are build-time requirements.

The media URLs and expected SHA-256 hashes are pinned in `MEDIA` in the helper.
Downloads are cached in `dist/cache` even when another output root is selected.
Cached files are verified before use; invalid or incomplete downloads cannot
replace a valid package. Existing downloads are reused on subsequent builds.

## Verification performed by every build

The helper builds the release workspace with an explicit target and locked
dependencies. Release flags are scoped to child processes: baseline x86-64 CPU
and static CRT for Windows; generic CPU for ARM64. Caller Rust flags are replaced
for these builds and the calling shell's environment is preserved.

Before publication, it:

1. Inspects all four executables. Windows rejects Visual C++ runtime imports and
   unresolved non-system DLLs. Linux checks ELF64 architecture, allowed system
   dependencies, no RPATH/RUNPATH, resolved libraries, and symbol versions. The
   x64 limits are GLIBC 2.39, GLIBCXX 3.4.30 and CXXABI 1.3.13.
2. Creates and extracts the archive, checking paths, duplicate entries and
   contents. Linux tar entries use owner/group 0, directories and executables
   mode 0755, other files mode 0644, including on Windows-mounted filesystems.
3. Tests the extracted CLI version and bundled FFmpeg/FFprobe versions with PATH
   cleared and isolated application configuration.
4. Generates test media, converts an image and video through the CLI, and verifies
   three H.264 frames with retained AAC audio using the bundled FFprobe.
5. Runs a bounded headless GUI render and checks the generated PNGs.

These checks cover packaging, conversion and headless rendering. Native desktop
interaction and ARM64 acceptance remain separate gates, recorded in
[`rune-validation.md`](rune-validation.md). The helper does not promise
byte-for-byte reproducible archives: build metadata contains a timestamp and Git
state describes the checkout at preflight. Keep sources unchanged during a build.

Focused helper tests can be run on Windows or Linux:

```sh
python -m unittest discover -s tools -p test_release_package.py -v
# Linux: use python3 instead of python.
```

## Publication and recovery

The combined build holds an output-root lock and uses a unique
`<output>/.staging/release-*` directory. A version becomes visible in
`<output>/releases/` only after both packages pass and the checksum file is
written. Other versions remain untouched. Same-version replacement requires an
existing directory containing exactly the two expected archives and a matching
manifest; unexpected files or mismatched hashes stop replacement.

The previous complete version is temporarily renamed to
`<output>/releases/.previous-<version>`. A publication failure attempts to restore
it. If recovery or backup cleanup fails, the backup is retained and its path is
reported. A remaining backup blocks the next combined build for that version.

To recover, stop any active build and verify the manifest in each surviving
directory. If the final version directory is missing, restore the verified backup
by renaming `.previous-<version>` to `<version>`. If the final directory is valid,
move the verified backup to a separate location outside `releases` before
rebuilding. Preserve unexpected files for inspection. An interrupted run may also
leave its staging directory; remove that specific abandoned directory only after
confirming no build uses it. The `.release.lock` file can remain after success;
the operating system releases its lock when the process exits.

## Manual GitHub upload

Use the complete release directory printed after `Release verified`. Check its
manifest from that directory on Linux/WSL:

```sh
sha256sum --check ascii-painter-0.2.0-SHA256SUMS.txt
```

The UTF-8 manifest uses LF line endings and relative archive filenames. Upload
both archives and the manifest to the GitHub release for version 0.2.0. This
workflow builds local files only; creating tags/releases and uploading assets are
manual actions. Build outputs and caches are ignored by Git.
