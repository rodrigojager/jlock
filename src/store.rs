//! The registry on disk (plan section 4): `locks/<sha1>/{key, holder.json, queue/, broken/}`.

use crate::cli::{fs_error, usage, Failure};
use crate::fsx;
use crate::holder::Holder;
use crate::key::Key;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::Duration;

pub struct Store {
    pub root: PathBuf,
    pub locks: PathBuf,
}

impl Store {
    pub fn open() -> Result<Store, Failure> {
        let root = crate::home::root().map_err(usage)?;
        let locks = fsx::retry(|| crate::home::locks_dir(&root)).map_err(|e| {
            fs_error(
                format!("cannot create {}", root.join("locks").display()),
                &e,
            )
        })?;
        Ok(Store { root, locks })
    }

    pub fn key_dir(&self, key: &Key) -> KeyDir {
        KeyDir {
            path: self.locks.join(&key.hash),
            key: key.clone(),
        }
    }
}

pub struct KeyDir {
    pub path: PathBuf,
    pub key: Key,
}

pub fn dir_is_empty(path: &Path) -> bool {
    match fs::read_dir(path) {
        Ok(mut entries) => entries.next().is_none(),
        Err(e) => fsx::is_not_found(&e),
    }
}

impl KeyDir {
    pub fn holder_path(&self) -> PathBuf {
        self.path.join("holder.json")
    }

    pub fn queue_dir(&self) -> PathBuf {
        self.path.join("queue")
    }

    pub fn broken_dir(&self) -> PathBuf {
        self.path.join("broken")
    }

    pub fn key_file(&self) -> PathBuf {
        self.path.join("key")
    }

    pub fn exists(&self) -> bool {
        self.path.is_dir()
    }

    /// Creates the key folder and its `key` file if missing.
    pub fn ensure(&self) -> io::Result<()> {
        fsx::retry(|| fs::create_dir_all(&self.path))?;
        self.ensure_key_file()
    }

    /// Writes the `key` file if it is missing. A concurrent cleanup may have removed it right after
    /// a holder or ticket appeared, so whoever creates one calls this again afterwards.
    pub fn ensure_key_file(&self) -> io::Result<()> {
        if self.key_file().exists() {
            return Ok(());
        }
        match fsx::create_exclusive(&self.key_file(), self.key.text.as_bytes()) {
            Ok(()) => Ok(()),
            Err(e) if fsx::is_exists(&e) || fsx::is_not_found(&e) => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// The current holder, `None` if free. A file still being written is read again for a moment;
    /// if it never parses, a placeholder with owner `?` is returned.
    pub fn read_holder(&self) -> io::Result<Option<Holder>> {
        read_body(&self.holder_path(), &self.key.text)
    }

    /// Removes the key folder if nothing is in it any more: no holder, no ticket, nothing in `broken/`.
    /// Losing a race with a newcomer is fine: `rmdir` fails with "not empty" and the folder stays.
    pub fn cleanup(&self) -> io::Result<bool> {
        if self.holder_path().exists()
            || !dir_is_empty(&self.queue_dir())
            || !dir_is_empty(&self.broken_dir())
        {
            return Ok(false);
        }
        fsx::remove_with_retry(&self.key_file())?;
        fsx::rmdir_if_empty(&self.queue_dir())?;
        fsx::rmdir_if_empty(&self.broken_dir())?;
        fsx::rmdir_if_empty(&self.path)
    }
}

/// Reads a holder or ticket body; `None` if the file does not exist.
pub fn read_body(path: &Path, key: &str) -> io::Result<Option<Holder>> {
    let mut last_err = None;
    for _ in 0..20 {
        match fs::read_to_string(path) {
            Ok(text) => {
                if let Some(h) = Holder::parse(&text) {
                    return Ok(Some(h));
                }
                last_err = None;
            }
            Err(e) if fsx::is_not_found(&e) => return Ok(None),
            Err(e) if fsx::is_transient(&e) => last_err = Some(e),
            Err(e) => return Err(e),
        }
        sleep(Duration::from_millis(5));
    }
    match last_err {
        Some(e) => Err(e),
        None => Ok(Some(Holder::unreadable(key))),
    }
}
