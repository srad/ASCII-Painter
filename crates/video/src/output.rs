//! Preflight and publication shared by GUI intents and video jobs.
//!
//! Conversion only writes owned adjacent stages. Publication is a rename, never
//! delete-then-write. Snapshot validation detects ordinary intervening changes;
//! it is not an atomic compare-and-swap against arbitrary external writers.
use std::fs::{self, OpenOptions, Permissions};
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use same_file::Handle;
use tempfile::TempPath;

use crate::{Cancellation, VideoError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputPolicy {
    CreateNew,
    ReplaceExisting,
}

struct Snapshot {
    identity: Handle,
    len: u64,
    modified: SystemTime,
    permissions: Permissions,
}

impl Snapshot {
    fn read(path: &Path) -> io::Result<Option<Self>> {
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let linked = metadata.file_type().is_symlink();
        #[cfg(windows)]
        let linked = {
            use std::os::windows::fs::MetadataExt;
            linked || metadata.file_attributes() & 0x400 != 0 // FILE_ATTRIBUTE_REPARSE_POINT
        };
        // Check before opening: opening a FIFO or device can block or have effects.
        if linked || !metadata.is_file() {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                "The output must be a regular file, not a symbolic link, reparse point or directory.",
            ));
        }
        if metadata.permissions().readonly() {
            return Err(io::Error::new(ErrorKind::PermissionDenied, "The output file is read-only."));
        }
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(0x7); // FILE_SHARE_READ | WRITE | DELETE
        }
        let file = options.open(path)?;
        let metadata = file.metadata()?;
        Ok(Some(Self {
            len: metadata.len(),
            modified: metadata.modified()?,
            permissions: metadata.permissions(),
            identity: Handle::from_file(file)?,
        }))
    }
}

/// A destination snapshot captured before conversion or overwrite confirmation.
pub struct OutputDestination {
    input: PathBuf,
    output: PathBuf,
    existing: Option<Snapshot>,
}

impl OutputDestination {
    pub fn capture(input: &Path, output: &Path, policy: OutputPolicy) -> Result<Self, VideoError> {
        let existing = Snapshot::read(output)?;
        let parent = output.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
        if !fs::metadata(parent)?.is_dir() {
            return Err(io::Error::new(ErrorKind::NotADirectory, "The output parent is not a directory.").into());
        }
        Self::reject_alias(input, existing.as_ref())?;
        if existing.is_some() && policy == OutputPolicy::CreateNew {
            return Err(io::Error::new(
                ErrorKind::AlreadyExists,
                "The output already exists; replacement was not authorized.",
            )
            .into());
        }
        Ok(Self { input: input.into(), output: output.into(), existing })
    }

    fn reject_alias(input: &Path, existing: Option<&Snapshot>) -> Result<(), VideoError> {
        if let Some(existing) = existing
            && Handle::from_path(input)? == existing.identity
        {
            return Err(VideoError::SameFile);
        }
        Ok(())
    }

    pub fn exists(&self) -> bool {
        self.existing.is_some()
    }

    pub fn validate(&self) -> Result<(), VideoError> {
        let current = Snapshot::read(&self.output)?;
        Self::reject_alias(&self.input, current.as_ref())?;
        match (&self.existing, current) {
            (None, None) => Ok(()),
            (Some(before), Some(after))
                if before.identity == after.identity
                    && before.len == after.len
                    && before.modified == after.modified =>
            {
                Ok(())
            }
            _ => Err(io::Error::new(
                ErrorKind::AlreadyExists,
                "The destination changed during export. Export again to confirm the new file.",
            )
            .into()),
        }
    }

    /// CLI/pipeline stage. New outputs use ordinary creation permissions (0666
    /// filtered by umask on Unix); GUI-owned outer stages remain private.
    pub fn stage(&self, extension: &str) -> Result<TempPath, VideoError> {
        self.validate()?;
        let parent = self.output.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
        let mut builder = tempfile::Builder::new();
        builder.prefix(".ascii-painter-video-");
        let suffix = format!(".{extension}");
        builder.suffix(&suffix);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // Replacement permissions are restored exactly at publication, not
            // filtered a second time by the current process's umask.
            if self.existing.is_none() {
                builder.permissions(Permissions::from_mode(0o666));
            }
        }
        Ok(builder.tempfile_in(parent)?.into_temp_path())
    }

    /// The successful rename is the commit point. Callers must not turn success
    /// into cancellation afterwards. This does not promise fsync/power-loss
    /// durability, or preservation of ownership, ACLs or extended attributes.
    pub fn publish(self, stage: TempPath) -> Result<PathBuf, VideoError> {
        self.publish_cancellable(stage, &Cancellation::default())
    }

    pub fn publish_cancellable(self, stage: TempPath, cancel: &Cancellation) -> Result<PathBuf, VideoError> {
        self.validate()?;
        if let Some(existing) = self.existing {
            #[cfg(unix)]
            fs::set_permissions(&stage, existing.permissions.clone())?;
            #[cfg(not(unix))]
            let _ = &existing.permissions;
            drop(existing); // release identity handle before rename on Windows
            if cancel.is_cancelled() {
                return Err(VideoError::Cancelled);
            }
            stage.persist(&self.output).map_err(|error| error.error)?;
        } else {
            // Even ReplaceExisting cannot clobber a destination absent at capture.
            if cancel.is_cancelled() {
                return Err(VideoError::Cancelled);
            }
            stage.persist_noclobber(&self.output).map_err(|error| error.error)?;
        }
        Ok(self.output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(dir: &Path) -> PathBuf {
        let input = dir.join("input.mp4");
        fs::write(&input, b"source bytes").unwrap();
        input
    }

    #[test]
    fn explicit_policy_allows_replacement_but_not_unrequested_clobbering() {
        let dir = tempfile::tempdir().unwrap();
        let input = source(dir.path());
        let output = dir.path().join("out.txt");
        fs::write(&output, b"old").unwrap();
        assert!(matches!(OutputDestination::capture(&input, &output, OutputPolicy::CreateNew),
            Err(VideoError::Io(error)) if error.kind() == ErrorKind::AlreadyExists));
        let destination = OutputDestination::capture(&input, &output, OutputPolicy::ReplaceExisting).unwrap();
        let stage = destination.stage("txt").unwrap();
        assert_eq!(stage.extension().unwrap(), "txt");
        fs::write(&stage, b"new").unwrap();
        assert_eq!(fs::read(&output).unwrap(), b"old");
        destination.publish(stage).unwrap();
        assert_eq!(fs::read(&output).unwrap(), b"new");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
    }

    #[test]
    fn absent_destinations_stay_no_clobber_even_with_replace_permission() {
        for policy in [OutputPolicy::CreateNew, OutputPolicy::ReplaceExisting] {
            let dir = tempfile::tempdir().unwrap();
            let input = source(dir.path());
            let output = dir.path().join("out.ans");
            let destination = OutputDestination::capture(&input, &output, policy).unwrap();
            let stage = destination.stage("ans").unwrap();
            fs::write(&stage, b"ours").unwrap();
            fs::write(&output, b"late external file").unwrap();
            assert!(destination.publish(stage).is_err());
            assert_eq!(fs::read(&output).unwrap(), b"late external file");
            assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
        }
    }

    #[test]
    fn changed_replaced_or_removed_destinations_are_not_published_over() {
        for change in ["modified", "replaced", "removed", "input alias"] {
            let dir = tempfile::tempdir().unwrap();
            let input = source(dir.path());
            let output = dir.path().join("out.mp4");
            fs::write(&output, b"before").unwrap();
            let destination = OutputDestination::capture(&input, &output, OutputPolicy::ReplaceExisting).unwrap();
            let stage = destination.stage("mp4").unwrap();
            fs::write(&stage, b"our output").unwrap();
            match change {
                "modified" => fs::write(&output, b"external modification").unwrap(),
                "replaced" => {
                    fs::remove_file(&output).unwrap();
                    fs::write(&output, b"before").unwrap();
                }
                "removed" => fs::remove_file(&output).unwrap(),
                _ => {
                    fs::remove_file(&output).unwrap();
                    fs::hard_link(&input, &output).unwrap();
                }
            }
            let expected = fs::read(&output).ok();
            assert!(destination.publish(stage).is_err(), "{change}");
            assert_eq!(fs::read(&output).ok(), expected, "{change}");
            assert_eq!(fs::read(&input).unwrap(), b"source bytes");
            assert!(
                !fs::read_dir(dir.path()).unwrap().any(|e| e
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".ascii-painter-"))
            );
        }
    }

    #[test]
    fn directories_read_only_files_and_io_errors_are_rejected_without_writing() {
        let dir = tempfile::tempdir().unwrap();
        let input = source(dir.path());
        assert!(OutputDestination::capture(&input, dir.path(), OutputPolicy::ReplaceExisting).is_err());
        let output = dir.path().join("readonly.txt");
        fs::write(&output, b"read only").unwrap();
        let original = fs::metadata(&output).unwrap().permissions();
        let mut readonly = original.clone();
        readonly.set_readonly(true);
        fs::set_permissions(&output, readonly).unwrap();
        let result = OutputDestination::capture(&input, &output, OutputPolicy::ReplaceExisting);
        fs::set_permissions(&output, original).unwrap();
        assert!(matches!(result, Err(VideoError::Io(error)) if error.kind() == ErrorKind::PermissionDenied));
        assert_eq!(fs::read(&output).unwrap(), b"read only");
        // NotADirectory must not be mistaken for an absent destination.
        assert!(OutputDestination::capture(&input, &input.join("out.txt"), OutputPolicy::ReplaceExisting).is_err());
    }

    #[cfg(any(unix, windows))]
    fn symlink(target: &Path, link: &Path) -> bool {
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(target, link).unwrap();
            true
        }
        #[cfg(windows)]
        {
            match std::os::windows::fs::symlink_file(target, link) {
                Ok(()) => true,
                Err(error) if error.raw_os_error() == Some(1314) => {
                    eprintln!("Windows symbolic-link validation unavailable: privilege not held");
                    false
                }
                Err(error) => panic!("create test symbolic link: {error}"),
            }
        }
    }

    #[test]
    fn symbolic_and_dangling_links_are_rejected_and_left_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let input = source(dir.path());
        for present in [true, false] {
            let target = dir.path().join(format!("target-{present}.txt"));
            let link = dir.path().join(format!("link-{present}.txt"));
            if present {
                fs::write(&target, b"target bytes").unwrap();
            }
            if !symlink(&target, &link) {
                return;
            }
            for policy in [OutputPolicy::CreateNew, OutputPolicy::ReplaceExisting] {
                assert!(OutputDestination::capture(&input, &link, policy).is_err());
                assert_eq!(fs::read_link(&link).unwrap(), target);
                assert_eq!(fs::read(&target).ok(), present.then(|| b"target bytes".to_vec()));
            }
            fs::remove_file(&link).unwrap();
        }
    }

    #[cfg(unix)]
    #[test]
    fn special_files_are_rejected_before_opening_and_replacement_keeps_unix_mode() {
        use std::os::unix::{fs::PermissionsExt, net::UnixListener};
        let dir = tempfile::tempdir().unwrap();
        let input = source(dir.path());
        let socket = dir.path().join("socket.txt");
        let _listener = UnixListener::bind(&socket).unwrap();
        assert!(OutputDestination::capture(&input, &socket, OutputPolicy::ReplaceExisting).is_err());
        let output = dir.path().join("out.txt");
        fs::write(&output, b"before").unwrap();
        fs::set_permissions(&output, Permissions::from_mode(0o640)).unwrap();
        let destination = OutputDestination::capture(&input, &output, OutputPolicy::ReplaceExisting).unwrap();
        let stage = destination.stage("txt").unwrap();
        fs::write(&stage, b"after").unwrap();
        destination.publish(stage).unwrap();
        assert_eq!(fs::metadata(&output).unwrap().permissions().mode() & 0o777, 0o640);
    }
}
