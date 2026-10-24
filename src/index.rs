//! Reading the registry: every key folder under `locks/`, with its holder and queue. The folder is
//! the truth; `index.json` is only derived from it for quick outside reading.

use crate::fsx;
use crate::holder::{iso, now_micros, Holder};
use crate::key::{sha1_hex, Key};
use crate::queue::{self, Ticket};
use crate::store::{read_body, KeyDir, Store};
use crate::wait::random_u64;
use serde_json::{json, Value};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::Duration;

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

fn entries(states: &[KeyState]) -> Value {
    let locks: Vec<Value> = states
        .iter()
        .filter(|st| st.holder.is_some() || !st.tickets.is_empty())
        .map(|st| {
            let h = st.holder.as_ref();
            json!({
                "key": st.dir.key.text,
                "owner": h.map(|h| h.owner.clone()),
                "group": h.and_then(|h| h.group.clone()),
                "intent": h.and_then(|h| h.intent.clone()),
                "since": h.map(|h| h.since.clone()),
                "queue": st.tickets.iter()
                    .map(|t| json!({ "owner": t.body.owner, "since": t.body.since }))
                    .collect::<Vec<_>>(),
            })
        })
        .collect();
    Value::Array(locks)
}

pub fn index_path(store: &Store) -> PathBuf {
    store.root.join("index.json")
}

/// The current `index.json`, or `None` if it is missing or does not parse (it is only derived).
pub fn load(store: &Store) -> Option<Value> {
    let text = fs::read_to_string(index_path(store)).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    v.get("locks")?.as_array()?;
    Some(v)
}

/// Rebuilds `index.json` from the folder: written to a `.tmp` and renamed over the old one. Left
/// alone when its content would not change. Being derived, a failed rename is not retried for
/// long: the next change or sweep writes it again.
pub fn rewrite(store: &Store) -> io::Result<()> {
    let locks = entries(&scan(store)?);
    if load(store).is_some_and(|old| old["locks"] == locks) {
        return Ok(());
    }
    let value = json!({ "updated": iso(now_micros()), "locks": locks });
    let tmp = store.root.join(format!(
        "index.json.{}-{:04x}.tmp",
        std::process::id(),
        random_u64() & 0xffff
    ));
    fsx::create_exclusive(&tmp, value.to_string().as_bytes())?;
    let target = index_path(store);
    let mut result = fs::rename(&tmp, &target);
    for _ in 0..3 {
        match &result {
            Err(e) if fsx::is_transient(e) => {
                sleep(Duration::from_millis(10));
                result = fs::rename(&tmp, &target);
            }
            _ => break,
        }
    }
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}
