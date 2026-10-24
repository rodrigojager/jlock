//! The operating system's own lock on `holder.json`, held by `hold` and `run` while they own a key:
//! `LockFileEx` (exclusive, fail-immediately) on Windows, `flock(LOCK_EX | LOCK_NB)` on Unix. The
//! kernel drops it the moment the process dies, which tells the living from the dead before the
//! heartbeat goes stale. It never blocks reading, renaming or deleting the file: on Windows the
//! locked byte lies far past the end of the file and the handle shares read, write and delete.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;

/// An OS lock held on a file; dropping it (closing the handle) lets go.
pub struct OsLock {
    _file: File,
}

pub enum Try {
    Locked(OsLock),
    Busy,
    Unsupported,
}

/// Tries to take the OS lock on `path` without waiting.
pub fn try_lock(path: &Path) -> io::Result<Try> {
    // std opens with read, write and delete sharing on Windows, and close-on-exec on Unix (so a
    // command started by `run` never inherits the lock).
    let file = OpenOptions::new().read(true).open(path)?;
    lock(file)
}

#[cfg(windows)]
fn lock(file: File) -> io::Result<Try> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::{ERROR_IO_PENDING, ERROR_LOCK_VIOLATION};
    use windows_sys::Win32::Storage::FileSystem::{
        LockFileEx, LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY,
    };
    use windows_sys::Win32::System::IO::OVERLAPPED;

    // SAFETY: OVERLAPPED is plain data; zero is its documented initial state.
    let mut at: OVERLAPPED = unsafe { std::mem::zeroed() };
    // One byte near the top of the 63-bit range: far beyond any content, so reads never collide.
    at.Anonymous.Anonymous.Offset = 0xFFFF_FFFE;
    at.Anonymous.Anonymous.OffsetHigh = 0x7FFF_FFFF;
    let flags = LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY;
    // SAFETY: the handle is open for the duration of the call and `at` outlives it.
    let ok = unsafe { LockFileEx(file.as_raw_handle() as _, flags, 0, 1, 0, &mut at) };
    if ok != 0 {
        return Ok(Try::Locked(OsLock { _file: file }));
    }
    let err = io::Error::last_os_error();
    match err.raw_os_error().map(|c| c as u32) {
        Some(ERROR_LOCK_VIOLATION) | Some(ERROR_IO_PENDING) => Ok(Try::Busy),
        _ => Ok(Try::Unsupported),
    }
}

#[cfg(unix)]
fn lock(file: File) -> io::Result<Try> {
    use std::os::unix::io::AsRawFd;
    // SAFETY: flock on a valid descriptor owned by `file`.
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc == 0 {
        return Ok(Try::Locked(OsLock { _file: file }));
    }
    let err = io::Error::last_os_error();
    match err.raw_os_error() {
        Some(libc::EWOULDBLOCK) => Ok(Try::Busy),
        _ => Ok(Try::Unsupported),
    }
}

/// Is the OS lock on `path` held by a live process right now? `false` when free, missing or
/// unsupported.
pub fn is_held(path: &Path) -> bool {
    matches!(try_lock(path), Ok(Try::Busy))
}
