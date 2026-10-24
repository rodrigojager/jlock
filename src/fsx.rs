//! File system primitives of plan section 7.5: exclusive create, rename/remove with retries on
//! `EBUSY`/`EPERM`/`EACCES` (50, 100, 200, 400, 800, 1600 ms), never killing anyone.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::thread::sleep;
use std::time::{Duration, SystemTime};

pub const RETRY_DELAYS_MS: [u64; 6] = [50, 100, 200, 400, 800, 1600];

/// Errors worth waiting out: a file held open by an editor or antivirus, or a delete still pending.
pub fn is_transient(e: &io::Error) -> bool {
    match e.raw_os_error() {
        #[cfg(windows)]
        Some(code) => matches!(code, 5 | 32 | 33),
        #[cfg(unix)]
        Some(code) => matches!(code, libc::EBUSY | libc::EPERM | libc::EACCES),
        None => false,
    }
}

pub fn is_not_found(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::NotFound
}

pub fn is_exists(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::AlreadyExists
}

pub fn is_not_empty(e: &io::Error) -> bool {
    if e.kind() == io::ErrorKind::DirectoryNotEmpty {
        return true;
    }
    match e.raw_os_error() {
        #[cfg(windows)]
        Some(code) => code == 145,
        #[cfg(unix)]
        Some(code) => code == libc::ENOTEMPTY || code == libc::EEXIST,
        None => false,
    }
}

/// Runs `op`, waiting and retrying while it fails with a transient error.
pub fn retry<T>(mut op: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    for delay in RETRY_DELAYS_MS {
        match op() {
            Err(e) if is_transient(&e) => sleep(Duration::from_millis(delay)),
            other => return other,
        }
    }
    op()
}

/// Creates `path` only if it does not exist (`create_new`), with `bytes` as content.
/// `AlreadyExists` and `NotFound` come back at once; the caller decides what they mean.
pub fn create_exclusive(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = retry(|| OpenOptions::new().write(true).create_new(true).open(path))?;
    if let Err(e) = file.write_all(bytes) {
        drop(file);
        let _ = fs::remove_file(path);
        return Err(e);
    }
    Ok(())
}

/// Removes a file. `Ok(false)` if it was already gone.
pub fn remove_with_retry(path: &Path) -> io::Result<bool> {
    match retry(|| fs::remove_file(path)) {
        Ok(()) => Ok(true),
        Err(e) if is_not_found(&e) => Ok(false),
        Err(e) => Err(e),
    }
}

/// Renames `from` over `to`. `NotFound` comes back at once (someone else got there first).
pub fn rename_with_retry(from: &Path, to: &Path) -> io::Result<()> {
    retry(|| fs::rename(from, to))
}

/// Removes an empty directory. `Ok(false)` if it is gone, not empty, or busy right now:
/// a directory that stays behind is removed by the next sweep.
pub fn rmdir_if_empty(path: &Path) -> io::Result<bool> {
    match fs::remove_dir(path) {
        Ok(()) => Ok(true),
        Err(e) if is_not_found(&e) || is_not_empty(&e) || is_transient(&e) => Ok(false),
        Err(e) => Err(e),
    }
}

/// Sets the modification time to now (the heartbeat).
pub fn touch(path: &Path) -> io::Result<()> {
    retry(|| {
        let file = OpenOptions::new().write(true).open(path)?;
        file.set_modified(SystemTime::now())
    })
}

/// How long ago `path` was last modified (zero if the clock is behind the file).
pub fn age(path: &Path) -> io::Result<Duration> {
    let modified = fs::metadata(path)?.modified()?;
    Ok(SystemTime::now()
        .duration_since(modified)
        .unwrap_or(Duration::ZERO))
}

pub fn read_string(path: &Path) -> io::Result<String> {
    retry(|| fs::read_to_string(path))
}
