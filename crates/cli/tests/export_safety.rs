//! Real CLI subprocesses and real ffmpeg, isolated from preferences and user files.
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use ascii_painter_video::FfmpegPaths;

fn tools() -> Option<FfmpegPaths> {
    match FfmpegPaths::locate(None) {
        Ok(paths) => Some(paths),
        Err(error) => {
            assert!(std::env::var_os("ASCII_PAINTER_REQUIRE_MEDIA_TESTS").is_none(), "required media tools: {error}");
            eprintln!("skipping: {error}");
            None
        }
    }
}

fn clip(paths: &FfmpegPaths, dir: &Path) -> PathBuf {
    let input = dir.join("input.mp4");
    let result = Command::new(&paths.ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=d=0.4:s=32x24:r=5",
            "-c:v",
            "libx264",
        ])
        .arg(&input)
        .output()
        .unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    input
}

fn command(paths: &FfmpegPaths, input: &Path, output: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ascii-painter-cli"));
    cmd.arg("convert")
        .arg(input)
        .arg("--output")
        .arg(output)
        .args(["--cols", "8", "--ffmpeg-dir"])
        .arg(paths.ffmpeg.parent().unwrap());
    cmd
}
fn succeeds(result: Output) {
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
}
fn no_stages(dir: &Path) {
    assert!(
        !std::fs::read_dir(dir).unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".ascii-painter-"))
    );
}

#[test]
fn cli_creates_each_video_format_and_requires_force_for_replacement() {
    let Some(paths) = tools() else { return };
    let dir = tempfile::tempdir().unwrap();
    let input = clip(&paths, dir.path());
    let source = std::fs::read(&input).unwrap();
    for extension in ["mp4", "txt", "ans"] {
        let output = dir.path().join(format!("art.{extension}"));
        succeeds(command(&paths, &input, &output).output().unwrap());
        let expected = std::fs::read(&output).unwrap();
        assert!(!expected.is_empty());
        if extension == "mp4" {
            assert_eq!(ascii_painter_video::probe(&paths, &output).unwrap().frames, Some(2));
        }
        assert!(!command(&paths, &input, &output).output().unwrap().status.success());
        assert_eq!(std::fs::read(&output).unwrap(), expected);
        std::fs::write(&output, b"old destination").unwrap();
        succeeds(command(&paths, &input, &output).arg("--force").output().unwrap());
        assert_eq!(std::fs::read(&output).unwrap(), expected);
        no_stages(dir.path());
    }
    assert_eq!(std::fs::read(&input).unwrap(), source);
}

#[test]
fn cli_force_cannot_delete_a_source_or_hardlink_alias() {
    let Some(paths) = tools() else { return };
    let dir = tempfile::tempdir().unwrap();
    let input = clip(&paths, dir.path());
    let source = std::fs::read(&input).unwrap();
    let alias = dir.path().join("alias.txt");
    std::fs::hard_link(&input, &alias).unwrap();
    for output in [&input, &alias] {
        assert!(!command(&paths, &input, output).arg("--force").output().unwrap().status.success());
        assert_eq!(std::fs::read(&input).unwrap(), source);
        assert_eq!(std::fs::read(&alias).unwrap(), source);
        no_stages(dir.path());
    }
}

#[test]
fn cli_validation_and_no_frame_failures_preserve_existing_output() {
    let Some(paths) = tools() else { return };
    let dir = tempfile::tempdir().unwrap();
    let input = clip(&paths, dir.path());
    let output = dir.path().join("keep.txt");
    let settings = dir.path().join("invalid.toml");
    std::fs::write(&settings, "[charset]\npreset = \"Custom\"\ncustom = \"\"\n").unwrap();
    std::fs::write(&output, b"existing bytes").unwrap();
    assert!(
        !command(&paths, &input, &output)
            .arg("--force")
            .arg("--settings")
            .arg(&settings)
            .output()
            .unwrap()
            .status
            .success()
    );
    assert_eq!(std::fs::read(&output).unwrap(), b"existing bytes");
    assert!(!command(&paths, &input, &output).args(["--force", "--start", "99"]).output().unwrap().status.success());
    assert_eq!(std::fs::read(&output).unwrap(), b"existing bytes");
    no_stages(dir.path());
}

#[cfg(unix)]
#[test]
fn cli_new_file_permissions_obey_umask_in_an_isolated_child_process() {
    use std::os::unix::fs::PermissionsExt;
    let Some(paths) = tools() else { return };
    let dir = tempfile::tempdir().unwrap();
    let input = clip(&paths, dir.path());
    let output = dir.path().join("umask.txt");
    // umask is process-global: change it only in the shell executing this one CLI.
    let result = Command::new("sh")
        .args(["-c", "umask 027; exec \"$@\"", "ascii-painter-umask-test"])
        .arg(env!("CARGO_BIN_EXE_ascii-painter-cli"))
        .arg("convert")
        .arg(&input)
        .arg("--output")
        .arg(&output)
        .args(["--cols", "8", "--ffmpeg-dir"])
        .arg(paths.ffmpeg.parent().unwrap())
        .output()
        .unwrap();
    succeeds(result);
    assert_eq!(std::fs::metadata(&output).unwrap().permissions().mode() & 0o777, 0o640);
    no_stages(dir.path());
}
