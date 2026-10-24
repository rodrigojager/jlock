//! Sweeps (plan 7.5 "Varredura"). Local: the keys a command touches clean themselves as they go
//! (`expire`, `cleanup`). Global: at most once per `JLOCK_SWEEP_MS`, any command sweeps every key
//! folder, rotates the log, rewrites the index and renews `last-sweep`. Every removal is idempotent
//! and every break a rename, so concurrent sweepers do no harm.

use crate::expire;
use crate::fsx;
use crate::heartbeat::stale_after;
use crate::holder::{now_micros, Holder};
use crate::index;
use crate::log;
use crate::store::{dir_is_empty, read_body, KeyDir, Store};
use crate::wait::{env_ms, random_u64};
use serde::Serialize;
use serde_json::json;
use std::fs;
use std::io;
use std::path::Path;

#[derive(Default, Debug, Serialize)]
pub struct Report {
    pub expired_locks: usize,
    pub expired_tickets: usize,
    pub broken_consumed: usize,
    pub folders_removed: usize,
    pub log_bytes: u64,
    pub errors: Vec<String>,
}

impl Report {
    fn removed_anything(&self) -> bool {
        self.expired_locks + self.expired_tickets + self.broken_consumed + self.folders_removed > 0
    }
}

pub fn sweep_ms() -> u64 {
    env_ms("JLOCK_SWEEP_MS", 60_000)
}

/// Notes and deletes whatever is left in `broken/` (a breaker died between rename and delete).
fn consume_broken(kd: &KeyDir, report: &mut Report) -> io::Result<()> {
    let entries = match fsx::retry(&kd.broken_dir(), || fs::read_dir(kd.broken_dir())) {
        Ok(entries) => entries,
        Err(e) if fsx::is_not_found(&e) => return Ok(()),
        Err(e) => return Err(e),
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        let body: Option<Holder> = read_body(&path, &kd.key.text).ok().flatten();
        if fsx::remove_with_retry(&path)? {
            report.broken_consumed += 1;
            let owner = body.as_ref().map_or("?", |b| b.owner.as_str()).to_string();
            let mut fields = log::about(&kd.key.text, &owner, None);
            fields["reason"] = json!("left in broken/");
            fields["body"] = json!(body);
            log::event("gc", fields);
        }
    }
    fsx::rmdir_if_empty(&kd.broken_dir())?;
    Ok(())
}

/// One key: expire the dead, consume `broken/`, restore a missing `key` file, remove the folder
/// if nothing is left in it.
pub fn sweep_key(kd: &KeyDir, report: &mut Report) -> io::Result<()> {
    for e in expire::expire_key(kd, None)? {
        match e.what {
            "holder" => report.expired_locks += 1,
            _ => report.expired_tickets += 1,
        }
    }
    consume_broken(kd, report)?;
    if kd.holder_path().exists() || !dir_is_empty(&kd.queue_dir()) {
        kd.ensure_key_file()?;
    }
    if kd.cleanup()? {
        report.folders_removed += 1;
    }
    Ok(())
}

/// A folder whose key cannot be told (no `key` file, no readable body): whatever in it is older
/// than STALE is debris from a crash and goes.
fn sweep_unknown(dir: &Path, report: &mut Report) -> io::Result<()> {
    let old = |p: &Path| fsx::age(p).is_ok_and(|a| a > stale_after());
    for sub in ["queue", "broken"] {
        if let Ok(entries) = fs::read_dir(dir.join(sub)) {
            for e in entries.filter_map(|e| e.ok()) {
                if old(&e.path()) && fsx::remove_with_retry(&e.path())? {
                    report.broken_consumed += 1;
                }
            }
        }
        fsx::rmdir_if_empty(&dir.join(sub))?;
    }
    for file in ["holder.json", "key"] {
        let path = dir.join(file);
        if path.exists() && old(&path) {
            fsx::remove_with_retry(&path)?;
        }
    }
    if fsx::rmdir_if_empty(dir)? {
        report.folders_removed += 1;
        log::event(
            "gc",
            json!({ "reason": format!("removed unknown folder {}", dir.display()) }),
        );
    }
    Ok(())
}

pub fn log_bytes(store: &Store) -> u64 {
    ["log.jsonl", "log.1.jsonl"]
        .iter()
        .filter_map(|f| fs::metadata(store.root.join(f)).ok())
        .map(|m| m.len())
        .sum()
}

/// The whole registry, now.
pub fn sweep_all(store: &Store) -> Report {
    let mut report = Report::default();
    let keep = |what: String, r: io::Result<()>, report: &mut Report| {
        if let Err(e) = r {
            report.errors.push(format!("{what}: {e}"));
        }
    };
    match fs::read_dir(&store.locks) {
        Ok(entries) => {
            for entry in entries.filter_map(|e| e.ok()) {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let r = match index::key_of_dir(&path) {
                    Some(key) => sweep_key(&KeyDir { path, key }, &mut report),
                    None => sweep_unknown(&path, &mut report),
                };
                keep(
                    entry.file_name().to_string_lossy().into_owned(),
                    r,
                    &mut report,
                );
            }
        }
        Err(e) => report
            .errors
            .push(format!("{}: {e}", store.locks.display())),
    }
    let r = log::rotate_if_needed(&store.root, 0).map(|_| ());
    keep("log rotation".into(), r, &mut report);
    keep("index.json".into(), index::rewrite(store), &mut report);
    keep("last-sweep".into(), mark(store), &mut report);
    report.log_bytes = log_bytes(store);
    for e in &report.errors {
        log::event("gc", json!({ "reason": format!("sweep error: {e}") }));
    }
    if report.removed_anything() {
        log::event("gc", json!({ "reason": "sweep", "removed": &report }));
    }
    report
}

/// Renews `last-sweep` (micros since the epoch), by `.tmp` + rename.
fn mark(store: &Store) -> io::Result<()> {
    let tmp = store.root.join(format!(
        "last-sweep.{}-{:04x}.tmp",
        std::process::id(),
        random_u64() & 0xffff
    ));
    fsx::create_exclusive(&tmp, now_micros().to_string().as_bytes())?;
    let result = fsx::rename_with_retry(&tmp, &store.root.join("last-sweep"));
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// The global sweep if `last-sweep` is missing or older than `JLOCK_SWEEP_MS`. `last-sweep` is
/// renewed first, so that the processes ending right after this one do not all sweep too.
pub fn maybe_sweep(store: &Store) {
    let last = fs::read_to_string(store.root.join("last-sweep"))
        .ok()
        .and_then(|t| t.trim().parse::<u64>().ok());
    let due = last.is_none_or(|t| now_micros().saturating_sub(t) / 1000 >= sweep_ms());
    if !due {
        return;
    }
    if let Err(e) = mark(store) {
        log::event(
            "gc",
            json!({ "reason": format!("cannot write last-sweep: {e}") }),
        );
        return;
    }
    sweep_all(store);
}
