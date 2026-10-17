//! Locating and spawning the ffmpeg/ffprobe binaries.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, ExitStatus, Stdio};
use std::thread::JoinHandle;

use crate::VideoError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FfmpegPaths {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
}

fn candidates(dir: &Path, tool: &str) -> [PathBuf; 2] {
    let exe = std::env::consts::EXE_SUFFIX;
    [dir.join(format!("{tool}{exe}")), dir.join(format!("{tool}-{}{exe}", env!("ASCII_PAINTER_TARGET")))]
}

fn find_in(dir: &Path, tool: &str) -> Option<PathBuf> {
    candidates(dir, tool).into_iter().find(|p| p.is_file())
}

fn find_tool(tool: &str, override_dir: Option<&Path>) -> Option<PathBuf> {
    let sidecar = std::env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_path_buf));
    sidecar.iter().map(PathBuf::as_path).chain(override_dir).find_map(|dir| find_in(dir, tool)).or_else(|| {
        std::env::var_os("PATH").and_then(|path| std::env::split_paths(&path).find_map(|d| find_in(&d, tool)))
    })
}

impl FfmpegPaths {
    /// Bundled sidecar next to the executable first, then `override_dir`, then `PATH`.
    pub fn locate(override_dir: Option<&Path>) -> Result<Self, VideoError> {
        let ffmpeg = find_tool("ffmpeg", override_dir).ok_or(VideoError::NotFound("ffmpeg"))?;
        let ffprobe = find_tool("ffprobe", override_dir).ok_or(VideoError::NotFound("ffprobe"))?;
        Ok(Self { ffmpeg, ffprobe })
    }
}

/// A command with quiet logging and, on Windows, no console window.
pub(crate) fn command(program: &Path) -> Command {
    let mut cmd = Command::new(program);
    cmd.args(["-hide_banner", "-loglevel", "error"]).stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// A running ffmpeg/ffprobe whose stderr is drained on its own thread (a full pipe would block
/// the child). Dropping it kills the process.
pub(crate) struct Process {
    tool: &'static str,
    child: Child,
    stderr: Option<JoinHandle<String>>,
}

impl Process {
    pub fn spawn(tool: &'static str, mut cmd: Command) -> Result<Self, VideoError> {
        cmd.stderr(Stdio::piped());
        let mut child = cmd.spawn().map_err(|source| VideoError::Spawn { tool, source })?;
        let mut pipe = child.stderr.take().expect("stderr piped");
        let stderr = std::thread::spawn(move || {
            let mut text = String::new();
            let _ = pipe.read_to_string(&mut text);
            text
        });
        Ok(Self { tool, child, stderr: Some(stderr) })
    }

    pub fn take_stdout(&mut self) -> ChildStdout {
        self.child.stdout.take().expect("stdout piped")
    }

    pub fn take_stdin(&mut self) -> ChildStdin {
        self.child.stdin.take().expect("stdin piped")
    }

    /// Wait for exit; a failure carries ffmpeg's stderr.
    pub fn finish(mut self) -> Result<(), VideoError> {
        let status: ExitStatus = self.child.wait()?;
        let stderr = self.stderr.take().and_then(|h| h.join().ok()).unwrap_or_default();
        if status.success() {
            Ok(())
        } else {
            Err(VideoError::Ffmpeg { tool: self.tool, status: status.to_string(), stderr: stderr.trim().to_owned() })
        }
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        if self.stderr.is_some() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}
