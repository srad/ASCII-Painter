//! Size-rotated application file logger and panic hook.
//!
//! Keeps at most `MAX_BACKUP_FILES + 1` files of `MAX_LOG_BYTES` each in the platform
//! configuration directory so crash and error diagnostics remain available without
//! unbounded disk growth.

use std::any::Any;
use std::cell::Cell;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::panic::Location;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use log::{Level, LevelFilter, Log, Metadata, Record};

/// Maximum size in bytes of a single log file before rotation (1 MiB).
pub const MAX_LOG_BYTES: u64 = 1_048_576;

/// Number of rotated backup files retained (`ascii-painter.log.1`, `ascii-painter.log.2`).
pub const MAX_BACKUP_FILES: usize = 2;

/// Maximum UTF-8 byte length of a single formatted message body before truncation (16 KiB).
pub const MAX_ENTRY_BYTES: usize = 16 * 1024;

static LOGGER: OnceLock<RotatingLogger> = OnceLock::new();
static PANIC_HOOK: OnceLock<()> = OnceLock::new();

thread_local! {
    static IN_LOGGER: Cell<bool> = const { Cell::new(false) };
}

struct ReentrancyGuard;

impl ReentrancyGuard {
    fn enter() -> Option<Self> {
        IN_LOGGER.with(|active| {
            if active.get() {
                None
            } else {
                active.set(true);
                Some(Self)
            }
        })
    }
}

impl Drop for ReentrancyGuard {
    fn drop(&mut self) {
        IN_LOGGER.with(|active| active.set(false));
    }
}

/// Platform log path (`<config_dir>/ascii-painter.log`), honoring an isolated config root when provided.
pub fn default_log_path(isolated_config_dir: Option<&Path>) -> Option<PathBuf> {
    isolated_config_dir
        .map(|dir| dir.join("ascii-painter.log"))
        .or_else(|| directories::ProjectDirs::from("", "", "ASCII Painter").map(|p| p.config_dir().join("ascii-painter.log")))
}

/// Initialize the global rotating file logger and panic hook.
/// Safe to call at most once per process; subsequent calls return the active log path.
pub fn init(isolated_config_dir: Option<&Path>) -> Option<PathBuf> {
    let path = default_log_path(isolated_config_dir)?;
    let logger = LOGGER.get_or_init(|| RotatingLogger {
        path: path.clone(),
        writer: Mutex::new(RotatingWriter::new(path, MAX_LOG_BYTES, MAX_BACKUP_FILES)),
    });
    if log::set_logger(logger).is_ok() {
        log::set_max_level(LevelFilter::Info);
    }
    PANIC_HOOK.get_or_init(install_panic_hook);
    Some(logger.path.clone())
}

fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(logger) = LOGGER.get() {
            let backtrace = std::backtrace::Backtrace::force_capture().to_string();
            let message = format_panic_message(info.payload(), info.location(), &backtrace);
            logger.write_raw("PANIC", "panic", &message);
        }
        previous(info);
    }));
}

struct RotatingLogger {
    path: PathBuf,
    writer: Mutex<RotatingWriter>,
}

impl RotatingLogger {
    fn write_raw(&self, level: &str, target: &str, message: &str) {
        let Some(_guard) = ReentrancyGuard::enter() else { return };
        let timestamp = format_utc_timestamp(SystemTime::now());
        let thread = std::thread::current();
        let thread_name = thread.name().unwrap_or("unnamed");
        let entry = format_entry(&timestamp, level, thread_name, target, message);
        let mut writer = self.writer.lock().unwrap_or_else(PoisonError::into_inner);
        writer.write_entry(&entry);
    }
}

impl Log for RotatingLogger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        allows_target(metadata.target(), metadata.level())
    }

    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let Some(_guard) = ReentrancyGuard::enter() else { return };
        let timestamp = format_utc_timestamp(SystemTime::now());
        let thread = std::thread::current();
        let thread_name = thread.name().unwrap_or("unnamed");
        let message = record.args().to_string();
        let entry = format_entry(&timestamp, record.level().as_str(), thread_name, record.target(), &message);
        let mut writer = self.writer.lock().unwrap_or_else(PoisonError::into_inner);
        writer.write_entry(&entry);
    }

    fn flush(&self) {
        let Some(_guard) = ReentrancyGuard::enter() else { return };
        let mut writer = self.writer.lock().unwrap_or_else(PoisonError::into_inner);
        writer.flush();
    }
}

/// Filter rules keeping third-party GPU/windowing chatter out of the log while retaining warnings/errors.
pub fn allows_target(target: &str, level: Level) -> bool {
    if target.starts_with("ascii_painter") || target == "panic" {
        level <= Level::Info
    } else {
        level <= Level::Warn
    }
}

/// Bounded file writer that rotates `<path>` -> `<path>.1` -> ... -> `<path>.N`.
pub struct RotatingWriter {
    path: PathBuf,
    max_bytes: u64,
    backups: usize,
    file: Option<File>,
    size: u64,
}

impl RotatingWriter {
    pub fn new(path: PathBuf, max_bytes: u64, backups: usize) -> Self {
        Self { path, max_bytes: max_bytes.max(512), backups, file: None, size: 0 }
    }

    pub fn write_entry(&mut self, entry: &str) {
        if self.open_if_needed().is_err() {
            return;
        }
        let entry_bytes = entry.len() as u64;
        if self.size >= self.max_bytes || (self.size > 0 && self.size.saturating_add(entry_bytes) > self.max_bytes) {
            self.rotate();
            if self.open_if_needed().is_err() {
                return;
            }
        }
        if let Some(file) = &mut self.file {
            if file.write_all(entry.as_bytes()).is_ok() {
                self.size = self.size.saturating_add(entry_bytes);
                let _ = file.flush();
            }
        }
    }

    pub fn flush(&mut self) {
        if let Some(file) = &mut self.file {
            let _ = file.flush();
        }
    }

    fn open_if_needed(&mut self) -> std::io::Result<()> {
        if self.file.is_some() {
            return Ok(());
        }
        if let Some(parent) = self.path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(&self.path)?;
        self.size = file.metadata().map(|m| m.len()).unwrap_or(0);
        self.file = Some(file);
        Ok(())
    }

    fn rotate(&mut self) {
        if let Some(mut file) = self.file.take() {
            let _ = file.flush();
        }
        if self.backups == 0 {
            if let Ok(file) = OpenOptions::new().create(true).write(true).truncate(true).open(&self.path) {
                self.size = 0;
                self.file = Some(file);
            }
            return;
        }
        for index in (1..=self.backups).rev() {
            let dst = backup_path(&self.path, index);
            let src = if index == 1 { self.path.clone() } else { backup_path(&self.path, index - 1) };
            if src.exists() {
                let _ = fs::remove_file(&dst);
                if fs::rename(&src, &dst).is_err() && index == 1 {
                    // Fallback if an external viewer holds a handle preventing rename on Windows.
                    if fs::copy(&src, &dst).is_ok() {
                        if let Ok(file) = OpenOptions::new().create(true).write(true).truncate(true).open(&self.path) {
                            self.size = 0;
                            self.file = Some(file);
                            return;
                        }
                    }
                }
            }
        }
        self.size = 0;
    }
}

pub fn backup_path(path: &Path, index: usize) -> PathBuf {
    let mut os = path.as_os_str().to_os_string();
    os.push(format!(".{index}"));
    PathBuf::from(os)
}

/// Format a log entry with UTF-8 safe truncation and indented continuation lines.
pub fn format_entry(timestamp: &str, level: &str, thread: &str, target: &str, message: &str) -> String {
    let normalized = message.replace("\r\n", "\n");
    let trimmed = normalized.trim_end();
    let (body, truncated) = if trimmed.len() > MAX_ENTRY_BYTES {
        let end = trimmed.floor_char_boundary(MAX_ENTRY_BYTES);
        (&trimmed[..end], true)
    } else {
        (trimmed, false)
    };
    let indented = body.replace('\n', "\n  ");
    if truncated {
        format!("{timestamp} [{level}] [{thread}] {target}: {indented} … [truncated]\n")
    } else {
        format!("{timestamp} [{level}] [{thread}] {target}: {indented}\n")
    }
}

/// Format a panic payload, source location, and captured backtrace.
pub fn format_panic_message(
    payload: &(dyn Any + Send),
    location: Option<&Location<'_>>,
    backtrace: &str,
) -> String {
    let message = if let Some(s) = payload.downcast_ref::<&str>() {
        *s
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.as_str()
    } else {
        "Box<dyn Any>"
    };
    let header = match location {
        Some(loc) => format!("panic at {}:{}:{}: {message}", loc.file(), loc.line(), loc.column()),
        None => format!("panic: {message}"),
    };
    let bt = backtrace.trim();
    if bt.is_empty() || bt == "disabled backtrace" {
        header
    } else {
        format!("{header}\nBacktrace:\n{bt}")
    }
}

/// Format a `SystemTime` as an ISO-8601 UTC timestamp (`YYYY-MM-DDTHH:MM:SS.mmmZ`).
pub fn format_utc_timestamp(time: SystemTime) -> String {
    let duration = time.duration_since(UNIX_EPOCH).unwrap_or(Duration::ZERO);
    let total_secs = duration.as_secs();
    let millis = duration.subsec_millis();
    let days = (total_secs / 86_400) as i64;
    let secs_of_day = total_secs % 86_400;
    let hour = secs_of_day / 3_600;
    let minute = (secs_of_day % 3_600) / 60;
    let second = secs_of_day % 60;
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z")
}

/// Howard Hinnant's civil date conversion from days since 1970-01-01 UTC.
fn civil_from_days(days_since_epoch: i64) -> (i64, u32, u32) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = i64::from(yoe) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    (year, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_timestamp_formats_epoch_leap_years_and_current_dates() {
        assert_eq!(format_utc_timestamp(UNIX_EPOCH), "1970-01-01T00:00:00.000Z");
        // 2000-02-29T12:34:56.789Z = 951827696.789s
        let leap = UNIX_EPOCH + Duration::from_millis(951_827_696_789);
        assert_eq!(format_utc_timestamp(leap), "2000-02-29T12:34:56.789Z");
        // 2026-11-08T11:30:00.042Z = 1794137400.042s
        let date = UNIX_EPOCH + Duration::from_millis(1_794_137_400_042);
        assert_eq!(format_utc_timestamp(date), "2026-11-08T11:30:00.042Z");
    }

    #[test]
    fn writer_rotates_bounded_backups_and_drops_oldest() {
        let dir = std::env::temp_dir().join(format!("ascii-painter-log-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let log_path = dir.join("ascii-painter.log");
        let mut writer = RotatingWriter::new(log_path.clone(), 600, 2);

        let chunk_a = format!("{:<250}\n", "A-entry");
        let chunk_b = format!("{:<250}\n", "B-entry");
        let chunk_c = format!("{:<250}\n", "C-entry");
        let chunk_d = format!("{:<250}\n", "D-entry");

        // Write 2 chunks (502 bytes <= 600) into active log
        writer.write_entry(&chunk_a);
        writer.write_entry(&chunk_a);
        assert!(log_path.is_file());
        assert!(!backup_path(&log_path, 1).exists());

        // Next chunk (502 + 251 > 600) rotates A -> .1, writes B into active log
        writer.write_entry(&chunk_b);
        writer.write_entry(&chunk_b);
        assert!(backup_path(&log_path, 1).is_file());
        assert!(!backup_path(&log_path, 2).exists());

        // Next chunk rotates .1 (A) -> .2, active (B) -> .1, writes C into active
        writer.write_entry(&chunk_c);
        writer.write_entry(&chunk_c);
        assert!(backup_path(&log_path, 2).is_file());
        assert!(fs::read_to_string(backup_path(&log_path, 2)).unwrap().contains("A-entry"));
        assert!(fs::read_to_string(backup_path(&log_path, 1)).unwrap().contains("B-entry"));

        // Next chunk rotates B -> .2 (dropping A), C -> .1, writes D into active
        writer.write_entry(&chunk_d);
        assert!(fs::read_to_string(&log_path).unwrap().contains("D-entry"));
        assert!(fs::read_to_string(backup_path(&log_path, 1)).unwrap().contains("C-entry"));
        assert!(fs::read_to_string(backup_path(&log_path, 2)).unwrap().contains("B-entry"));
        assert!(!backup_path(&log_path, 3).exists());

        for file in [&log_path, &backup_path(&log_path, 1), &backup_path(&log_path, 2)] {
            assert!(file.metadata().unwrap().len() <= 600);
        }
        drop(writer);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn entry_truncates_safely_on_multibyte_utf8_and_indents_newlines() {
        let multi = "电影🎬".repeat(3_000);
        assert!(multi.len() > MAX_ENTRY_BYTES);
        let formatted = format_entry("2026-11-08T00:00:00.000Z", "ERROR", "main", "ascii_painter_gui", &multi);
        assert!(formatted.ends_with(" … [truncated]\n"));

        let multiline = format_entry("2026-11-08T00:00:00.000Z", "WARN", "worker", "ascii_painter_gui", "line1\r\nline2\nline3\n");
        assert_eq!(multiline, "2026-11-08T00:00:00.000Z [WARN] [worker] ascii_painter_gui: line1\n  line2\n  line3\n");
    }

    #[test]
    fn target_filter_blocks_third_party_info_and_keeps_warnings() {
        assert!(allows_target("ascii_painter_gui::app", Level::Info));
        assert!(allows_target("ascii_painter_video::pipeline", Level::Info));
        assert!(!allows_target("ascii_painter_gui::app", Level::Debug));
        assert!(!allows_target("wgpu_hal::vulkan", Level::Info));
        assert!(allows_target("wgpu_hal::vulkan", Level::Warn));
        assert!(allows_target("rune::window", Level::Error));
    }

    #[test]
    fn panic_formatter_includes_location_payload_and_backtrace() {
        let payload: Box<dyn Any + Send> = Box::new("GPU surface lost");
        let loc = Location::caller();
        let report = format_panic_message(&*payload, Some(loc), "   0: test_frame\n");
        assert!(report.contains("GPU surface lost"));
        assert!(report.contains("logging.rs"));
        assert!(report.contains("Backtrace:\n0: test_frame"));
    }
}
