//! Reading the registry: every key folder under `locks/`, with its holder and queue. The folder is
//! the truth; nothing here trusts a cache.

use crate::fsx;
use crate::holder::Holder;
use crate::key::{sha1_hex, Key};
use crate::queue::{self, Ticket};
use crate::store::{read_body, KeyDir, Store};
use std::fs;
use std::io;
use std::path::Path;

pub struct KeyState {
    pub dir: KeyDir,
    pub holder: Option<Holder>,
    pub tickets: Vec<Ticket>,
}

/// The key a folder stands for: its `key` file, checked against the folder name; if missing (a
/// cleanup raced with a newcomer), the `key` field of its holder or of any ticket.
pub fn key_of_dir(path: &Path) -> Option<Key> {
    let hash = path.file_name()?.to_string_lossy().into_owned();
    let matches = |text: &str| sha1_hex(text) == hash;
    if let Ok(text) = fs::read_to_string(path.join("key")) {
        if matches(&text) {
            return Some(Key::from_text(&text));
        }
    }
    let mut bodies = vec![path.join("holder.json")];
    if let Ok(entries) = fs::read_dir(path.join("queue")) {
        bodies.extend(entries.filter_map(|e| e.ok()).map(|e| e.path()));
    }
    bodies
        .iter()
        .filter_map(|p| read_body(p, "").ok().flatten())
        .find(|h| matches(&h.key))
        .map(|h| Key::from_text(&h.key))
}

/// Every key folder whose key can be told. Folders that cannot are left to the sweep.
pub fn dirs(store: &Store) -> io::Result<Vec<KeyDir>> {
    let entries = match fs::read_dir(&store.locks) {
        Ok(entries) => entries,
        Err(e) if fsx::is_not_found(&e) => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut out = Vec::new();
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if let Some(key) = key_of_dir(&path) {
            out.push(KeyDir { path, key });
        }
    }
    out.sort_by(|a, b| a.key.text.cmp(&b.key.text));
    Ok(out)
}

pub fn state(dir: KeyDir) -> io::Result<KeyState> {
    let holder = dir.read_holder()?;
    let tickets = queue::list(&dir)?;
    Ok(KeyState {
        dir,
        holder,
        tickets,
    })
}

/// Holder and queue of every key, in key order.
pub fn scan(store: &Store) -> io::Result<Vec<KeyState>> {
    dirs(store)?.into_iter().map(state).collect()
}
