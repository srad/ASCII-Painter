//! Locating and spawning the ffmpeg/ffprobe binaries.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, ExitStatus, Stdio};
use std::sync::{
    Arc, Mutex, Weak,
    atomic::{AtomicBool, Ordering},
};
use std::thread::JoinHandle;

use crate::VideoError;

/// Request-scoped cancellation. Killing registered children interrupts blocked pipe I/O,
/// not just the work between frames. No process IDs are retained or reused.
#[derive(Clone, Default)]
pub struct Cancellation(Arc<CancelState>);

#[derive(Default)]
struct CancelState {
    cancelled: AtomicBool,
    children: Mutex<Vec<Weak<Mutex<Child>>>>,
}

impl Cancellation {
    pub fn is_cancelled(&self) -> bool {
        self.0.cancelled.load(Ordering::Acquire)
    }

    pub fn cancel(&self) {
        self.0.cancelled.store(true, Ordering::Release);
        for child in self.0.children.lock().unwrap().iter().filter_map(Weak::upgrade) {
            let _ = child.lock().unwrap().kill();
        }
    }

    fn register(&self, child: &Arc<Mutex<Child>>) {
        let mut children = self.0.children.lock().unwrap();
        children.retain(|c| c.strong_count() > 0);
        children.push(Arc::downgrade(child));
        if self.is_cancelled() {
            let _ = child.lock().unwrap().kill();
        }
    }
}

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
    child: Arc<Mutex<Child>>,
    stderr: Option<JoinHandle<String>>,
}

impl Process {
    pub fn spawn_cancellable(tool: &'static str, mut cmd: Command, cancel: &Cancellation) -> Result<Self, VideoError> {
        if cancel.is_cancelled() {
            return Err(VideoError::Cancelled);
        }
        cmd.stderr(Stdio::piped());
        let mut child = cmd.spawn().map_err(|source| VideoError::Spawn { tool, source })?;
        let mut pipe = child.stderr.take().expect("stderr piped");
        let stderr = std::thread::spawn(move || {
            let mut text = String::new();
            let _ = pipe.read_to_string(&mut text);
            text
        });
        let child = Arc::new(Mutex::new(child));
        cancel.register(&child);
        Ok(Self { tool, child, stderr: Some(stderr) })
    }

    pub fn take_stdout(&mut self) -> ChildStdout {
        self.child.lock().unwrap().stdout.take().expect("stdout piped")
    }

    pub fn take_stdin(&mut self) -> ChildStdin {
        self.child.lock().unwrap().stdin.take().expect("stdin piped")
    }

    /// Wait for exit; a failure carries ffmpeg's stderr.
    pub fn finish(mut self) -> Result<(), VideoError> {
        // Never hold the child lock while waiting: cancellation must be able to kill it.
        let status: ExitStatus = loop {
            if let Some(status) = self.child.lock().unwrap().try_wait()? {
                break status;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
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
            let mut child = self.child.lock().unwrap();
            let _ = child.kill();
            let _ = child.wait();
            drop(child);
            if let Some(stderr) = self.stderr.take() {
                let _ = stderr.join();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, Write};
    use std::time::{Duration, Instant};

    #[test]
    fn blocked_child() {
        if std::env::var_os("ASCII_PAINTER_CANCEL_TEST_CHILD").is_none() {
            return;
        }
        println!("APP-CANCEL-CHILD-READY");
        std::io::stdout().flush().unwrap();
        loop {
            std::thread::park_timeout(Duration::from_secs(60));
        }
    }

    #[test]
    fn cancellation_interrupts_blocked_pipe_io_and_reaps_the_child() {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", "ffmpeg::tests::blocked_child", "--nocapture"])
            .env("ASCII_PAINTER_CANCEL_TEST_CHILD", "1")
            .stdout(Stdio::piped());
        let cancel = Cancellation::default();
        let mut process = Process::spawn_cancellable("test-child", command, &cancel).unwrap();
        let stdout = process.take_stdout();
        let (ready, receive) = std::sync::mpsc::channel();
        let reader = std::thread::spawn(move || {
            let mut stdout = std::io::BufReader::new(stdout);
            let mut line = String::new();
            while stdout.read_line(&mut line).unwrap() > 0 {
                if line.contains("APP-CANCEL-CHILD-READY") {
                    let _ = ready.send(());
                    break;
                }
                line.clear();
            }
            let mut rest = String::new();
            let _ = stdout.read_to_string(&mut rest);
        });
        receive.recv_timeout(Duration::from_secs(5)).expect("test child became ready");
        let started = Instant::now();
        cancel.cancel();
        assert!(process.finish().is_err());
        reader.join().unwrap();
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
