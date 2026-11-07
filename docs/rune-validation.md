# Rune migration validation

Validation and documentation refreshed on 2026-11-07 with Rust 1.99.1. The
workspace/package results below precede the later UI polish; its focused checks
are recorded separately. This is implementation evidence, not certification of
every desktop, compositor, GPU or CPU architecture.

## Implemented scope

Rune and Rune Kit are the only UI framework. Rune's core is pinned and vendored;
the native-window extensions are described in
[`vendor/rune/UPSTREAM.md`](../vendor/rune/UPSTREAM.md). The active source,
manifests, lockfile and dependency tree contain no egui/eframe family crates.
Legacy RON is parsed with compatibility-only structs, without the old framework.

The application keeps its executable names, conversion engine, presets and TOML
format, and custom Windows and Linux titlebars. The port includes settings,
numeric/color editing, image/video loading, comparison modes, pan/zoom/split,
video seek/trim/quality, export controls and dialogs. Workers and FFmpeg children
have owned cancellation and shutdown paths. GUI exports use adjacent staging and
captured overwrite intent; CLI video exports also use owned adjacent staging.
Preferences use versioned JSON and non-destructive
legacy import. The default layout is 1400 x 900 with a 900 x 600 minimum.

## Automated checks before UI polish

These are the recorded regression-validation results from 2026-11-07, before
the button and scrollbar changes. They are not new full-suite or release-build
results for the latest UI.

| Check | Result |
| --- | --- |
| Required-media Windows workspace tests | 116 reported passing: core 39, GUI 45, headless 1, video 20, pipeline 8, CLI 3; Windows real-link checks were privilege-gated as noted below |
| Required-media Linux workspace tests | 120 passed: core 39, GUI 47, headless 1, video 21, pipeline 8, CLI 4 |
| `cargo clippy --workspace --all-targets --locked --offline -- -D warnings` | Passed on Windows and Linux |
| `cargo fmt -p ascii-painter-gui -p ascii-painter-video -p ascii-painter-cli -- --check` | Passed |
| `git diff --check` | Passed; Git reports local CRLF conversion warnings |
| Locked Windows x64 and Linux x64 release workspace builds | Passed |
| Windows `build.ps1` and Linux `build.sh` package assembly | Passed, including checksum-verified FFmpeg 9.0.2 sidecars |

The recorded required-media runs selected the packaged standalone FFmpeg/FFprobe 9.0.2 on
PATH and set `ASCII_PAINTER_REQUIRE_MEDIA_TESTS=1`. Missing media tools then fail
the test instead of silently skipping it. No media test was skipped. For example,
after assembling the regression packages:

```powershell
$env:PATH = "$PWD\target\rune-regression-validation\packages\ascii-painter-windows-x64;$env:PATH"
$env:ASCII_PAINTER_REQUIRE_MEDIA_TESTS = '1'
cargo test --workspace --locked --offline -- --test-threads=1
```

```sh
PATH="$PWD/target/rune-regression-validation/packages/ascii-painter-linux-x64:$PATH" \
  ASCII_PAINTER_REQUIRE_MEDIA_TESTS=1 \
  cargo test --workspace --locked --offline -- --test-threads=1
```

Coverage includes all MP4 qualities, frame/audio
retention, rotation, GIF, trimming, preview/export parity, text/ANSI and cancel
cleanup. A GUI-level integration test also publishes a completed video over a
confirmed destination. It guards against cancellation of a shared scope when a
completed encoder job is joined.

GUI tests cover stale results, bounded latest-pending media loading, off-UI
shutdown, immutable overwrite capture, destination identity/alias checks,
preference migration and final flush, numeric draft semantics, color edits,
camera behavior, modal blocking, minimum-size layout, typed synchronous resize
actions and the compositor shader's Rune clip contract.

## Regression fixes and safety boundaries

The follow-up fixes preserve compatible output formats when loading media,
restore directional preset boundaries for unlisted/deleted presets, and derive
the native window title from the accepted source. Custom titlebar rendering is
unchanged. Output paths are normalized to the selected extension before the
overwrite prompt, with typed whitespace trimmed and empty/directory-only input
rejected. Unedited native paths retain Unicode and Unix non-UTF-8 filenames.

GUI job polling now checks thread completion before reading the result channel.
It distinguishes pending work from a disconnected worker without a racy second
completion check, and cannot join a worker still inside its waker. Per-job test
boundaries deterministically reproduce both load and export completion races;
the application tests failed before the fixes and now pass. The headless shell
test likewise reproduced the missing native source title before the fix.

GUI and video-pipeline publication share regular-file, identity, size and mtime
preflight/revalidation. Final-component symbolic links (including dangling
links), Windows reparse points, directories, special files, read-only files and
input/hard-link aliases are refused. Stages are uniquely named beside the
destination, retain the encoder's required suffix, and are the only writable
files during conversion. Writers and child processes are closed/reaped before
publication or stage cleanup. The requested destination is never deleted by
error cleanup.

CLI video uses `OutputPolicy::CreateNew` unless `--force` explicitly permits
replacement. A destination absent at capture remains no-clobber even under
replacement policy. Existing destinations are replaced only after snapshot
revalidation. Cancellation immediately before publication aborts; successful
publication is the commit point, so later cancellation cannot remove the output
or turn success into failure. The existing `ExportRequest`, `VideoError`,
`ExportJob::start` and `start_cancellable` interfaces remain compatible;
policy-aware starters are additive. CLI still-image conversion is unchanged.

Tests cover source/alias preservation, early validation and FFmpeg failures,
drop/unwind cleanup, intervening destination changes, before/after-commit
cancellation, and real CLI subprocesses. Unix replacement permissions are
preserved; new CLI files obey `0666 & umask` (tested in an isolated child), while
GUI-owned stages remain private. Rename publication changes file identity. It
does not promise ownership/ACL/xattr preservation, power-loss durability, or an
atomic compare-and-swap against arbitrary concurrent external writers.

## Rendering and packaged execution before UI polish

48 GPU snapshots cover `empty`, `image`, `video`, `advanced`, `overwrite`,
`progress`, `success` and `failure`, at 900 x 600 and 1400 x 900, each at scales
1, 1.5 and 2. They are generated from deterministic fixtures, not photographs of
a user's session. Representative layouts and the final packaged Windows/Linux
image renders were visually inspected. The README uses the same production
converter and renderer in [`screenshots/split-view.png`](screenshots/split-view.png).

For example:

```sh
mkdir -p target/rune-validation/manual
ascii-painter --scene image --snap target/rune-validation/manual/image.png \
  --size 1400x900 --scale 1.5 --scroll-bars always --at 1.0
```

The settled timestamp includes the viewer's layout callback. On Windows,
PowerShell must wait explicitly for the GUI-subsystem release executable when
checking its exit code (for example, `Start-Process -Wait -PassThru`).

Both recorded x64 packages passed headless GPU rendering and CLI still/video
conversion. Video inputs had three frames plus audio; the packaged output was
probed as three H.264 frames at 112 x 84 with AAC audio. These conversions used
adjacent packaged FFmpeg/FFprobe discovery, not a custom FFmpeg path.

The initial migration Linux package also opened native windows on isolated Xvfb/X11 and
Weston headless/Wayland displays, presenting 120 frames on each with zero failed
surface acquisitions and clean application shutdown. On the 1280-wide X11
display the requested 1400-point width was clamped to 1248 and saved in isolated
preferences. Wayland kept 1400 x 900 on its 1600 x 1000 display. Weston emitted
test-environment cursor/shutdown warnings; no application failure was reported.
No compositor or application processes were left running. The generated
temporary compositor runtime data was removed.

After the regression fixes, fresh Windows/Linux x64 packages repeated image and
video headless GPU captures at 1400 x 900, scale 1.5, with source-title assertions.
Both passed still/video CLI conversion using adjacent bundled sidecars. Both
packaged same-input/output `--force` attempts failed and left the input hash
unchanged. Exported video retained three H.264 frames at 112 x 84 plus AAC audio.
These follow-up checks did not repeat native desktop interaction acceptance.

Linux builds/tests ran in the existing Ubuntu WSL distribution with Rust and
native build/virtual-display prerequisites installed there. The Windows desktop
was not launched or automated: fresh owner permission is still required.

## UI polish and documentation refresh

The later Windows debug UI uses a 144 x 40 Open action and a 180 x 40
Export / Cancel action, including disabled and cancelling states. Typography,
the custom titlebars and other controls are unchanged. The settings scrollbar
uses an opaque muted violet thumb (`#8260AE`) and lighter hover/drag colour
(`#9C7ACE`), retaining Rune's scrolling and reveal/fade behavior.

Five focused `ui_polish` tests pass with:

```sh
cargo test -p ascii-painter-gui ui_polish --locked --offline -- --test-threads=1
```

They cover action dimensions and label containment across empty, image, video,
exporting and cancelling scenes at 900 x 600 and 1400 x 900, each at scales
1, 1.5 and 2; enlarged click targets and keyboard activation; disabled actions;
and scrollbar colours, contrast, dragging, retained offset and reveal/fade.
The unchanged 28-point Browse control and 64-point toolbar are also asserted.
Relevant formatting, the locked debug GUI build and whitespace checks pass.

Earlier polish captures are preserved in ignored `target/ui-polish-18h/`;
the final-size minimum-window image and progress captures are in
`target/ui-buttons-ocq/`. The refreshed README screenshot is generated from
the current debug GUI at 1400 x 900, scale 1, with scrollbars always visible;
its capture and element tree are in `target/docs-cleanup-ejm/`. It was visually
inspected before copying to `screenshots/split-view.png`.

These focused Windows checks do not revalidate the earlier release packages,
repeat the Linux/media suites, or resolve native desktop / ARM64 acceptance.

## Local evidence and archives

Historical captures, native trace headers, isolated preferences, media fixtures
and packages are in ignored `target/rune-validation/`. Existing `dist` releases
were left untouched during those validation runs. The later documentation
cleanup removed only the obsolete egui Windows package folder and ZIP from
`dist`; download caches and historical validation packages were preserved.
To build the current code into a separate destination without overwriting that
evidence, use:

```powershell
.\build.ps1 -DestinationRoot 'target/rune-packages-current'
```

```sh
ASCII_PAINTER_DIST_DIR=target/rune-packages-current ./build.sh
```

| Historical archive in `target/rune-validation/packages/` | SHA-256 |
| --- | --- |
| `ascii-painter-windows-x64.zip` | `d8298a89a5771fd3dc723a597c38bbdec2c32375118ce5ae2984b9f91fb91e75` |
| `ascii-painter-linux-x64.tar.gz` | `6feb8201411bd70b9ade0aa995ec9011d8daaedbac9b4468f1494bf5c0e1905c` |

Both archives contain the GUI, CLI, FFmpeg, FFprobe, README, screenshot and
DejaVu/Rune/Inter/IBM Plex/FFmpeg license notices. Builds retain some pre-existing
upstream Rune dead-code warnings; the application's strict workspace Clippy
check passes.

The regression fixes have separate captures, trees, media fixtures and packages
under ignored `target/rune-regression-validation/`. The original migration
archive hashes above were rechecked during regression validation and left
unchanged. These archives also precede the latest UI polish; they are retained
as evidence, not current release packages.

| Historical archive in `target/rune-regression-validation/packages/` | SHA-256 |
| --- | --- |
| `ascii-painter-windows-x64.zip` | `376b141d526ec36812a613fcd9a3d67ba1c9d75fb0d13e5ea6fcb09f478c61ad` |
| `ascii-painter-linux-x64.tar.gz` | `0504fdcb4f4a01ef3937942f75ff9c9b2d9060230fd9984cc6d7d5c702bc9595` |

## Remaining acceptance boundaries

Beads is the source of truth for outstanding work. `ASCIIfy-iyd.5` covers real
Windows and Linux desktop interactions: custom-frame gestures, Windows Snap,
all resize grips, multi-monitor/DPI restore, native file portals/dialogs,
keyboard focus and assistive-client behavior. Tests of typed resize actions and
virtual-display presentation do not establish those interactive results.

`ASCIIfy-iyd.6` covers Linux ARM64 build, package and runtime validation. The
native-architecture packaging path is retained, but ARM64 was not built or
executed during this validation. Migration acceptance remains open until these
platform gates are resolved. Real Windows symbolic-link tests could not execute
because the account lacks symlink privilege (Windows error 1314); no security
settings were changed. The corresponding real Linux symlink/dangling-link,
hard-link, special-file, permission and non-UTF-8 checks did execute and pass.

The pre-existing CLI video deletion bug `ASCIIfy-3rz` is fixed by this follow-up,
including red-to-green disposable-file tests for source deletion, hard-link
truncation and existing-destination deletion.

An independent launcher-cancellation limitation is tracked as `ASCIIfy-bea`.
A Windows test using the Chocolatey FFmpeg launcher stalled after launcher
parents exited while actual encoder/decoder descendants held pipes open. Only
that verified owned test process tree was stopped. The same cancellation test
passed in 1.68 seconds with standalone packaged FFmpeg, and both final
required-media suites passed with packaged binaries. Launcher-descendant
ownership remains follow-up work; use the bundled/direct standalone tools for
the verified cancellation path.
