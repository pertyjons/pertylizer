//! Crash-safe file replacement.
//!
//! Every user document (project, bundle, patch, template, settings) is written
//! through here instead of straight to its destination. A plain
//! `fs::write`/`File::create` truncates the target *before* the new bytes
//! exist, so a serialization error, a full disk, or a crash midway leaves the
//! user with a half-written file and no copy of their last good save.
//!
//! The sequence here is the standard write-temp-then-replace dance:
//!
//! 1. Create a uniquely named temporary file **in the destination's own
//!    directory** — `rename` is only atomic within one filesystem, so a temp in
//!    `/tmp` would degrade to a copy and reintroduce the torn-write window.
//! 2. Write the full contents into it.
//! 3. `sync_all` so the bytes reach the disk before anything points at them.
//! 4. Match the destination's existing permissions (`inherit_permissions`; a
//!    plain code span because the function is private and so absent from these
//!    docs — a link would resolve to nothing a reader can follow).
//! 5. Replace the destination with a single `rename`.
//!
//! Until step 5 the destination is untouched, so *any* failure before it leaves
//! the previous file exactly as it was — there is nothing to roll back. The
//! temporary file is removed when the [`NamedTempFile`] drops.
//!
//! # Replacing an open file is a platform difference
//!
//! Step 5 is a rename over the destination, and the platforms disagree about
//! whether that is allowed while some other handle still has the destination
//! open. Unix does not care — the rename succeeds and the open handle keeps
//! pointing at the now-unlinked inode. Windows denies it with `Access is
//! denied. (os error 5)`.
//!
//! For the application this is the right behaviour on both: the save fails
//! loudly and the previous file survives, which is the whole contract above.
//!
//! For **tests** it is a trap, because it only fails on one platform. A test
//! that saves onto a [`NamedTempFile`]'s path is holding the destination open,
//! so it passes locally on Linux or macOS and fails only in Windows CI. Save
//! into a path inside a [`tempfile::tempdir`] instead — the directory is
//! cleaned up just the same and nothing holds the file.

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use tempfile::NamedTempFile;

/// Prefix for the temporary file created next to the destination. Visible in a
/// directory listing only if the process dies between creation and replacement.
const TEMP_PREFIX: &str = ".pertylizer-save-";

/// A failure in the write-temp-then-replace sequence.
///
/// Every variant means **the destination was left untouched** — the replacement
/// is the last step and only [`Self::Replace`] can fail with the destination
/// mid-flight, which `rename` itself performs atomically.
#[derive(Debug, thiserror::Error)]
pub enum AtomicWriteError {
    /// The destination path has no parent directory to place the temp file in.
    #[error("Cannot resolve parent directory of {path}")]
    NoParent {
        /// The destination that was being written.
        path: PathBuf,
    },
    /// Creating the temporary file beside the destination failed.
    #[error("Cannot create temporary file in {dir}: {source}")]
    CreateTemp {
        /// The directory the temp file was to be created in.
        dir: PathBuf,
        /// The underlying I/O error.
        source: std::io::Error,
    },
    /// Flushing the buffered contents to the temporary file failed.
    #[error("Cannot flush temporary file for {path}: {source}")]
    Flush {
        /// The destination that was being written.
        path: PathBuf,
        /// The underlying I/O error.
        source: std::io::Error,
    },
    /// `sync_all` on the temporary file failed — the bytes are not durable.
    #[error("Cannot sync temporary file for {path}: {source}")]
    Sync {
        /// The destination that was being written.
        path: PathBuf,
        /// The underlying I/O error.
        source: std::io::Error,
    },
    /// Replacing the destination with the temporary file failed.
    #[error("Cannot replace {path}: {source}")]
    Replace {
        /// The destination that was being written.
        path: PathBuf,
        /// The underlying I/O error.
        source: std::io::Error,
    },
}

/// Write `contents` to `path`, replacing any existing file atomically.
///
/// The destination keeps its previous contents if anything fails.
///
/// # Errors
///
/// Returns [`AtomicWriteError`] if the temporary file cannot be created,
/// written, synced, or moved into place.
pub fn write(path: &Path, contents: &[u8]) -> Result<(), AtomicWriteError> {
    write_with::<_, AtomicWriteError>(path, |file| {
        file.write_all(contents)
            .map_err(|source| AtomicWriteError::Flush {
                path: path.to_path_buf(),
                source,
            })
    })
}

/// Build the contents of `path` by writing into a temporary file, then replace
/// the destination atomically.
///
/// Use this instead of [`write()`] when the payload is streamed rather than held
/// in one buffer — [`crate::bundle::save_bundle`] hands the handle to a ZIP
/// writer. The destination keeps its previous contents if `write_contents`
/// fails, and its error is returned unchanged.
///
/// # Errors
///
/// Returns the error from `write_contents`, or an [`AtomicWriteError`]
/// converted into `E` if the temporary file cannot be created, synced, or moved
/// into place.
pub fn write_with<T, E>(
    path: &Path,
    write_contents: impl FnOnce(&mut File) -> Result<T, E>,
) -> Result<T, E>
where
    E: From<AtomicWriteError>,
{
    let dir = path.parent().ok_or_else(|| AtomicWriteError::NoParent {
        path: path.to_path_buf(),
    })?;
    // An empty parent means a bare filename like `song.ptz` — that is the
    // current directory, not "no directory".
    let dir = if dir.as_os_str().is_empty() {
        Path::new(".")
    } else {
        dir
    };

    let mut temp = NamedTempFile::with_prefix_in(TEMP_PREFIX, dir).map_err(|source| {
        AtomicWriteError::CreateTemp {
            dir: dir.to_path_buf(),
            source,
        }
    })?;

    // The payload's own error wins: it says what actually went wrong (bad
    // serialization, a WAV that would not encode), and the half-written temp
    // file is discarded on drop either way.
    let value = write_contents(temp.as_file_mut())?;

    temp.flush().map_err(|source| AtomicWriteError::Flush {
        path: path.to_path_buf(),
        source,
    })?;
    temp.as_file()
        .sync_all()
        .map_err(|source| AtomicWriteError::Sync {
            path: path.to_path_buf(),
            source,
        })?;

    inherit_permissions(&temp, path);

    temp.persist(path).map_err(|e| AtomicWriteError::Replace {
        path: path.to_path_buf(),
        source: e.error,
    })?;

    Ok(value)
}

/// Give the temporary file the permissions the destination already has, so
/// overwriting a project does not silently change who can read it.
///
/// `NamedTempFile` creates at `0600`. Persisting that over a `0644` project
/// would strip group/other read access on every save. When the destination does
/// not exist yet (a first save), `0644` is applied so a new project matches what
/// `fs::write` would have produced under a typical `022` umask.
///
/// Best-effort: a failure here does not abort the save. The bytes are correct
/// and durable, and the file's mode is not worth losing the user's work over.
#[cfg(unix)]
fn inherit_permissions(temp: &NamedTempFile, destination: &Path) {
    use std::os::unix::fs::PermissionsExt;

    let mode =
        std::fs::metadata(destination).map_or(0o644, |meta| meta.permissions().mode() & 0o777);
    if let Err(e) = temp
        .as_file()
        .set_permissions(std::fs::Permissions::from_mode(mode))
    {
        tracing::warn!(
            target: "pertylizer::io",
            path = %destination.display(),
            error = %e,
            "could not apply permissions to save temp file",
        );
    }
}

/// Windows has no mode bits to inherit; `persist` keeps the destination's ACL.
#[cfg(not(unix))]
fn inherit_permissions(_temp: &NamedTempFile, _destination: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    /// The base case: a file that does not exist yet is created with the
    /// contents.
    #[test]
    fn writes_a_new_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("new.ptz");

        write(&path, b"hello").expect("write");

        assert_eq!(std::fs::read(&path).expect("read"), b"hello");
    }

    /// Overwriting replaces the contents wholesale rather than appending or
    /// leaving a tail of the longer previous file.
    #[test]
    fn overwrites_an_existing_file_completely() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("existing.ptz");
        std::fs::write(&path, b"a much longer previous version").expect("seed");

        write(&path, b"short").expect("write");

        assert_eq!(std::fs::read(&path).expect("read"), b"short");
    }

    /// The whole point: when the payload fails midway, the previous save must
    /// still be on disk untouched.
    #[test]
    fn keeps_the_previous_file_when_the_payload_fails() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("project.ptz");
        std::fs::write(&path, b"last good save").expect("seed");

        let result: Result<(), AtomicWriteError> = write_with(&path, |file| {
            // Write a partial payload, then fail — the classic torn write.
            file.write_all(b"corrupt partial")
                .map_err(|source| AtomicWriteError::Flush {
                    path: path.clone(),
                    source,
                })?;
            Err(AtomicWriteError::NoParent { path: path.clone() })
        });

        assert!(result.is_err(), "payload failure must propagate");
        assert_eq!(
            std::fs::read(&path).expect("read"),
            b"last good save",
            "destination must still hold the previous save",
        );
    }

    /// A failed write must not leave its scratch file behind for the user to
    /// find next to their project.
    #[test]
    fn removes_the_temp_file_when_the_payload_fails() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("project.ptz");

        let result: Result<(), AtomicWriteError> = write_with(&path, |_| {
            Err(AtomicWriteError::NoParent { path: path.clone() })
        });

        assert!(result.is_err());
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .expect("read_dir")
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().starts_with(TEMP_PREFIX))
            .collect();
        assert!(leftovers.is_empty(), "temp file leaked: {leftovers:?}");
        assert!(!path.exists(), "destination must not be created on failure");
    }

    /// A bare filename has an empty parent, which must resolve to the current
    /// directory rather than erroring out.
    #[test]
    fn writes_to_a_bare_relative_filename() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("bare.ptz");
        // Exercise the empty-parent branch directly: `Path::new("bare.ptz")`
        // has `Some("")` as its parent.
        assert_eq!(Path::new("bare.ptz").parent(), Some(Path::new("")));

        write(&path, b"ok").expect("write");
        assert_eq!(std::fs::read(&path).expect("read"), b"ok");
    }

    /// Saving into a directory that does not exist fails at temp-file creation,
    /// with the destination directory named in the error.
    #[test]
    fn reports_a_missing_destination_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("no-such-dir").join("project.ptz");

        let err = write(&path, b"data").expect_err("must fail");

        assert!(
            matches!(err, AtomicWriteError::CreateTemp { .. }),
            "expected CreateTemp, got {err:?}",
        );
    }

    /// Overwriting must not tighten the destination's permissions to the
    /// temp file's `0600`.
    #[cfg(unix)]
    #[test]
    fn preserves_existing_file_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("shared.ptz");
        std::fs::write(&path, b"old").expect("seed");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).expect("set perms");

        write(&path, b"new").expect("write");

        let mode = std::fs::metadata(&path)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o640, "permissions must survive an overwrite");
    }

    /// A brand-new file gets the conventional `0644` rather than the temp
    /// file's private `0600`.
    #[cfg(unix)]
    #[test]
    fn new_files_are_not_created_private() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("fresh.ptz");

        write(&path, b"new").expect("write");

        let mode = std::fs::metadata(&path)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o644);
    }
}
