//! The registry on disk (plan section 4, revised for 0.3):
//! `locks/<sha1>/{key, gate, holds/<grant>.json, holds/<grant>.lease, queue/, broken/}`.
//!
//! A key may have several holds at once (shared holds, nested holds of one family), one file each,
//! named by its grant (`<micros-UTC-16-digits>-<5 digits>`, in grant order). Decisions about one key are
//! made under its `gate`, a file created with `create_new` and deleted a moment later: two callers
//! asking for the same key at the same instant are decided one after the other.

use crate::cli::{fs_error, usage, Failure};
use crate::fsx;
use crate::holder::Holder;
use crate::i18n::txt;
use crate::key::Key;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::sleep;
use std::time::Duration;

static OPENED: AtomicBool = AtomicBool::new(false);

/// Whether this process opened the registry (a command that got past argument checking).
pub fn opened() -> bool {
    OPENED.load(Ordering::SeqCst)
}

pub struct Store {
    pub root: PathBuf,
    pub locks: PathBuf,
}

impl Store {
    pub fn open() -> Result<Store, Failure> {
        let root = crate::home::root().map_err(|k| usage(txt!(k)))?;
        let locks = fsx::retry(&root, || crate::home::locks_dir(&root)).map_err(|e| {
            let path = root.join("locks").display().to_string();
            fs_error(txt!("error.cannot_create", path = path), &e)
        })?;
        crate::log::init(&root);
        OPENED.store(true, Ordering::SeqCst);
        Ok(Store { root, locks })
    }

    pub fn key_dir(&self, key: &Key) -> KeyDir {
        KeyDir {
            path: self.locks.join(&key.hash),
            key: key.clone(),
        }
    }
}

#[derive(Clone)]
pub struct KeyDir {
    pub path: PathBuf,
    pub key: Key,
}

/// One hold of a key: `holds/<grant>.json`. Its mtime is the holder's heartbeat.
#[derive(Clone, Debug)]
pub struct Hold {
    pub grant: String,
    pub path: PathBuf,
    pub body: Holder,
}

pub fn dir_is_empty(path: &Path) -> bool {
    match fs::read_dir(path) {
        Ok(mut entries) => entries.next().is_none(),
        Err(e) => fsx::is_not_found(&e),
    }
}

/// The hold files (`<grant>.json`) in a `holds/` folder, by grant name, in grant order.
fn hold_names(dir: &Path) -> io::Result<Vec<String>> {
    let entries = match fsx::retry(dir, || fs::read_dir(dir)) {
        Ok(entries) => entries,
        Err(e) if fsx::is_not_found(&e) => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.strip_suffix(".json").map(str::to_string)
        })
        .collect();
    names.sort();
    Ok(names)
}

/// The key's gate, held while one decision about the key is made; dropping it deletes the file.
pub struct Gate {
    path: PathBuf,
}

impl Drop for Gate {
    fn drop(&mut self) {
        let _ = fsx::remove_with_retry(&self.path);
    }
}

impl KeyDir {
    pub fn holds_dir(&self) -> PathBuf {
        self.path.join("holds")
    }

    pub fn hold_path(&self, grant: &str) -> PathBuf {
        self.holds_dir().join(format!("{grant}.json"))
    }

    pub fn lease_path(&self, grant: &str) -> PathBuf {
        self.holds_dir().join(format!("{grant}.lease"))
    }

    pub fn gate_path(&self) -> PathBuf {
        self.path.join("gate")
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

    /// Creates the key folder, its `holds/` and its `key` file if missing.
    pub fn ensure(&self) -> io::Result<()> {
        fsx::ensure_dir(&self.holds_dir())?;
        self.ensure_key_file()
    }

    /// Writes the `key` file if it is missing. A concurrent cleanup may have removed it right after
    /// a hold or ticket appeared, so whoever creates one calls this again afterwards.
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

    /// Every hold of the key, in grant order. A file still being written is read again for a
    /// moment; if it never parses, a placeholder with owner `?` stands for it.
    pub fn read_holds(&self) -> io::Result<Vec<Hold>> {
        let mut out = Vec::new();
        for grant in hold_names(&self.holds_dir())? {
            let path = self.hold_path(&grant);
            if let Some(body) = read_body(&path, &self.key.text)? {
                out.push(Hold { grant, path, body });
            }
        }
        Ok(out)
    }

    /// The holds of one owner.
    pub fn holds_of(&self, owner: &str) -> io::Result<Vec<Hold>> {
        Ok(self
            .read_holds()?
            .into_iter()
            .filter(|h| h.body.owner == owner)
            .collect())
    }

    pub fn has_holds(&self) -> bool {
        hold_names(&self.holds_dir()).is_ok_and(|n| !n.is_empty())
    }

    /// Takes the key's gate: decisions about this key are made one at a time. The gate lives for
    /// the moment it takes to read the holds and create one; a gate older than `JLOCK_STALE_MS` was
    /// left by a process that died inside it and is broken.
    pub fn gate(&self) -> io::Result<Gate> {
        let body = format!("{}", std::process::id());
        loop {
            self.ensure()?;
            match fsx::create_exclusive(&self.gate_path(), body.as_bytes()) {
                Ok(()) => {
                    return Ok(Gate {
                        path: self.gate_path(),
                    })
                }
                Err(e) if fsx::is_exists(&e) => {
                    let stale = crate::heartbeat::stale_after();
                    if fsx::age(&self.gate_path()).is_ok_and(|a| a > stale) {
                        self.break_gate(stale)?;
                        continue;
                    }
                    sleep(Duration::from_millis(1));
                }
                Err(e) if fsx::is_not_found(&e) => continue, // folder removed by a release
                Err(e) => return Err(e),
            }
        }
    }

    /// Breaks a gate left by a dead process: renamed into `broken/` (one breaker wins), put back if
    /// it turns out to be a fresh one (taken between the check and the rename), else deleted.
    pub fn break_gate(&self, stale: Duration) -> io::Result<bool> {
        fsx::ensure_dir(&self.broken_dir())?;
        let target = self.broken_dir().join(format!(
            "{:016}-{:04x}-gate",
            crate::holder::now_micros(),
            crate::wait::random_u64() & 0xffff
        ));
        match fsx::rename_with_retry(&self.gate_path(), &target) {
            Ok(()) => {
                let fresh = fsx::age(&target).is_ok_and(|a| a <= stale);
                let restored = fresh && fs::hard_link(&target, self.gate_path()).is_ok();
                fsx::remove_with_retry(&target)?;
                fsx::rmdir_if_empty(&self.broken_dir())?;
                if !restored {
                    crate::log::event(
                        "gc",
                        serde_json::json!({ "key": self.key.text, "reason": "stale gate" }),
                    );
                }
                Ok(!restored)
            }
            Err(e) if fsx::is_not_found(&e) => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Removes the key folder if nothing is in it any more: no hold, no ticket, no gate, nothing in
    /// `broken/`. Leases (and `.tmp` files of a renew that died) go with it, only then: while a hold
    /// sits in `broken/` for a moment its lease still counts. Losing a race with a newcomer is fine:
    /// `rmdir` fails with "not empty" and the folder stays.
    pub fn cleanup(&self) -> io::Result<bool> {
        if self.has_holds()
            || self.gate_path().exists()
            || !dir_is_empty(&self.queue_dir())
            || !dir_is_empty(&self.broken_dir())
        {
            return Ok(false);
        }
        self.remove_leases(|_| true)?;
        fsx::rmdir_if_empty(&self.holds_dir())?;
        fsx::remove_with_retry(&self.key_file())?;
        fsx::rmdir_if_empty(&self.queue_dir())?;
        fsx::rmdir_if_empty(&self.broken_dir())?;
        fsx::rmdir_if_empty(&self.path)
    }

    /// Removes the lease of one grant (and its leftover `.tmp` files).
    pub fn remove_lease(&self, grant: &str) -> io::Result<()> {
        self.remove_leases(|name| name.starts_with(grant))
    }

    /// Removes the leases and lease `.tmp` files in `holds/` whose name passes `pick`.
    fn remove_leases(&self, pick: impl Fn(&str) -> bool) -> io::Result<()> {
        let Ok(entries) = fs::read_dir(self.holds_dir()) else {
            return Ok(());
        };
        for entry in entries.filter_map(|e| e.ok()) {
            let name = entry.file_name().to_string_lossy().into_owned();
            if (name.ends_with(".lease") || name.ends_with(".tmp")) && pick(&name) {
                fsx::remove_with_retry(&entry.path())?;
            }
        }
        Ok(())
    }
}

/// Reads a hold or ticket body; `None` if the file does not exist.
pub fn read_body(path: &Path, key: &str) -> io::Result<Option<Holder>> {
    for _ in 0..20 {
        // A file being deleted while someone reads it answers "access denied" for a moment.
        match fsx::retry(path, || fs::read_to_string(path)) {
            Ok(text) => {
                if let Some(h) = Holder::parse(&text) {
                    return Ok(Some(h));
                }
            }
            Err(e) if fsx::is_not_found(&e) => return Ok(None),
            Err(e) => return Err(e),
        }
        sleep(Duration::from_millis(5));
    }
    Ok(Some(Holder::unreadable(key)))
}
