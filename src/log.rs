//! `log.jsonl` (plan section 4.3): one JSON line per event, appended; rotated to `log.1.jsonl`
//! (replacing it) before it would pass `JLOCK_LOG_MAX_BYTES`.

use crate::fsx;
use crate::holder::{iso, now_micros};
use crate::wait::env_ms;
use serde_json::{json, Value};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

static ROOT: OnceLock<PathBuf> = OnceLock::new();
static CHANGED: AtomicBool = AtomicBool::new(false);

/// Where events go; set once the registry root is known.
pub fn init(root: &Path) {
    let _ = ROOT.set(root.to_path_buf());
}

/// Whether this process logged a change (the index then needs rewriting).
pub fn changed() -> bool {
    CHANGED.load(Ordering::SeqCst)
}

pub fn max_bytes() -> u64 {
    env_ms("JLOCK_LOG_MAX_BYTES", 5_242_880)
}

pub fn path(root: &Path) -> PathBuf {
    root.join("log.jsonl")
}

/// Records `event` with the given fields (`null` fields are left out). Never fails the caller:
/// a log that cannot be written is reported under `JLOCK_DEBUG` only.
pub fn event(event: &str, fields: Value) {
    if event != "gc" {
        CHANGED.store(true, Ordering::SeqCst);
    }
    let Some(root) = ROOT.get() else {
        return;
    };
    let mut line = json!({ "at": iso(now_micros()), "event": event });
    if let Value::Object(map) = fields {
        for (k, v) in map {
            if !v.is_null() {
                line[k] = v;
            }
        }
    }
    let mut text = line.to_string();
    text.push('\n');
    if let Err(e) = append(root, text.as_bytes()) {
        crate::wait::debug(format!("cannot write the log: {e}"));
    }
}

fn append(root: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let log = path(root);
    rotate_if_needed(root, bytes.len() as u64)?;
    let mut file = fsx::retry(&log, || {
        OpenOptions::new().append(true).create(true).open(&log)
    })?;
    file.write_all(bytes)
}

/// Moves `log.jsonl` over `log.1.jsonl` if adding `incoming` bytes would pass the limit.
pub fn rotate_if_needed(root: &Path, incoming: u64) -> std::io::Result<bool> {
    let log = path(root);
    let size = match std::fs::metadata(&log) {
        Ok(m) => m.len(),
        Err(e) if fsx::is_not_found(&e) => return Ok(false),
        Err(e) => return Err(e),
    };
    if size == 0 || size + incoming <= max_bytes() {
        return Ok(false);
    }
    match fsx::rename_with_retry(&log, &root.join("log.1.jsonl")) {
        Ok(()) => Ok(true),
        Err(e) if fsx::is_not_found(&e) => Ok(false), // another process rotated it just now
        Err(e) => Err(e),
    }
}

/// The usual fields of an event about `owner` on `key`.
pub fn about(key: &str, owner: &str, group: Option<&str>) -> Value {
    json!({ "key": key, "owner": owner, "group": group })
}

/// The last event logged about `key`, with the owner it names, read from the end of the log only
/// (the last 64 KB, plenty for the moment just after something happened to the key).
pub fn last_about(key: &str) -> Option<(String, String)> {
    use std::io::{Read, Seek, SeekFrom};
    let root = ROOT.get()?;
    let mut file = std::fs::File::open(path(root)).ok()?;
    let len = file.metadata().ok()?.len();
    let from = len.saturating_sub(64 * 1024);
    file.seek(SeekFrom::Start(from)).ok()?;
    let mut tail = Vec::new();
    file.read_to_end(&mut tail).ok()?;
    let text = String::from_utf8_lossy(&tail);
    text.lines().rev().find_map(|line| {
        let v: Value = serde_json::from_str(line).ok()?;
        (v["key"].as_str() == Some(key)).then(|| {
            (
                v["event"].as_str().unwrap_or("").to_string(),
                v["owner"].as_str().unwrap_or("").to_string(),
            )
        })
    })
}
