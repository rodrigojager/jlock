//! `jlock status <key>`: holder and queue of one key, and the keys that block it through the
//! hierarchy (a folder above it, files inside it) or `*`.

use crate::cli::{fs_error, print_json, usage, Arg, CmdResult, Parser, EXIT_OK};
use crate::cmd::acquire::normalize_keys;
use crate::conflict::conflicting_dirs;
use crate::expire;
use crate::fsx;
use crate::heartbeat::stale_after;
use crate::holder::{now_micros, Holder};
use crate::index;
use crate::key::Key;
use crate::oslock;
use crate::store::{KeyDir, Store};
use serde_json::{json, Value};
use std::io;
use std::path::Path;

/// `850ms`, `12s`, `3m05s`, `2h10m`, `3d4h`.
pub fn human(ms: u64) -> String {
    let s = ms / 1000;
    match s {
        _ if ms < 1000 => format!("{ms}ms"),
        0..=59 => format!("{s}s"),
        60..=3599 => format!("{}m{:02}s", s / 60, s % 60),
        3600..=86_399 => format!("{}h{:02}m", s / 3600, s / 60 % 60),
        _ => format!("{}d{}h", s / 86_400, s / 3600 % 24),
    }
}

/// Milliseconds since an ISO time written by jlock (0 if unknown).
pub fn ms_since(iso: &str) -> u64 {
    crate::holder::parse_iso(iso).map_or(0, |t| now_micros().saturating_sub(t) / 1000)
}

pub fn heartbeat_ms(path: &std::path::Path) -> Option<u64> {
    fsx::age(path).ok().map(|d| d.as_millis() as u64)
}

/// `(dead, late)` for a holder. Dead: heartbeat older than STALE with no OS lock on the file, or
/// past the ttl; the expiration will break it. Late: heartbeat older than STALE but a `hold`/`run`
/// still has the OS lock, so the owner is alive and the lock stays (see `expire.rs`).
pub fn liveness(holder_path: &Path, h: &Holder) -> (bool, bool) {
    let stale_ms = stale_after().as_millis() as u64;
    let hb_stale = heartbeat_ms(holder_path).is_some_and(|ms| ms > stale_ms);
    let late = hb_stale && oslock::is_held(holder_path);
    ((hb_stale && !late) || h.ttl_expired(now_micros()), late)
}

pub fn holder_json(kd: &KeyDir, h: &Holder) -> Value {
    let (dead, late) = liveness(&kd.holder_path(), h);
    let mut v = json!(h);
    v["held_ms"] = json!(ms_since(&h.since));
    v["heartbeat_ms"] = json!(heartbeat_ms(&kd.holder_path()));
    v["stale"] = json!(dead);
    v["late"] = json!(late);
    v
}

fn describe(kd: &KeyDir, h: &Holder) -> String {
    let mut line = format!(
        "by {} since {} ({} ago)",
        h.owner,
        h.since,
        human(ms_since(&h.since))
    );
    if let Some(hb) = heartbeat_ms(&kd.holder_path()) {
        line += &format!(", heartbeat {} ago", human(hb));
        match liveness(&kd.holder_path(), h) {
            (true, _) => line += " (dead)",
            (false, true) => line += " (late, process alive)",
            _ => {}
        }
    }
    if let Some(g) = &h.group {
        line += &format!(", group {g}");
    }
    if let Some(i) = &h.intent {
        line += &format!(", intent \"{i}\"");
    }
    line
}

fn status(store: &Store, key: Key) -> io::Result<(String, Value)> {
    let kd = store.key_dir(&key);
    if kd.exists() {
        expire::expire_key(&kd, None)?;
        let _ = kd.cleanup();
    }
    let st = index::state(store.key_dir(&key))?;
    let mut text = String::new();
    match &st.holder {
        Some(h) => text += &format!("held {}\n  {}\n", key.text, describe(&kd, h)),
        None => text += &format!("free {}\n", key.text),
    }
    let mut queue = Vec::new();
    if !st.tickets.is_empty() {
        text += "queue:\n";
    }
    for (i, t) in st.tickets.iter().enumerate() {
        let waiting = ms_since(&t.body.since);
        text += &format!(
            "  {}. {} (waiting {})\n",
            i + 1,
            t.body.owner,
            human(waiting)
        );
        queue.push(
            json!({ "position": i + 1, "owner": t.body.owner, "since": t.body.since,
                           "waiting_ms": waiting, "ticket": t.name }),
        );
    }
    let mut conflicts = Vec::new();
    for cd in conflicting_dirs(store, &key)? {
        expire::expire_key(&cd, None)?;
        let cs = index::state(cd)?;
        if cs.holder.is_none() && cs.tickets.is_empty() {
            continue;
        }
        if conflicts.is_empty() {
            text += "conflicts:\n";
        }
        match &cs.holder {
            Some(h) => text += &format!("  {} held by {}", cs.dir.key.text, h.owner),
            None => text += &format!("  {} (not held)", cs.dir.key.text),
        }
        if !cs.tickets.is_empty() {
            text += &format!(", {} waiting", cs.tickets.len());
        }
        text += "\n";
        conflicts.push(json!({
            "key": cs.dir.key.text,
            "holder": cs.holder.as_ref().map(|h| holder_json(&cs.dir, h)),
            "queue": cs.tickets.len(),
        }));
    }
    let value = json!({
        "result": "ok",
        "key": key.text,
        "state": if st.holder.is_some() { "held" } else { "free" },
        "holder": st.holder.as_ref().map(|h| holder_json(&kd, h)),
        "queue": queue,
        "conflicts": conflicts,
    });
    Ok((text, value))
}

pub fn run(args: Vec<String>) -> CmdResult {
    let mut p = Parser::new(args);
    let mut json = false;
    let mut input = None;
    while let Some(arg) = p.next() {
        match arg {
            Arg::Flag(f) if f == "json" => json = true,
            Arg::Pos(k) if input.is_none() => input = Some(k),
            Arg::Pos(k) => return Err(usage(format!("status takes one key (extra '{k}')"))),
            Arg::Flag(f) => return Err(usage(format!("status: unknown option --{f}"))),
            Arg::Rest(_) => return Err(usage("status: unexpected '--'")),
        }
    }
    let input = input.ok_or_else(|| usage("status needs a key"))?;
    let key = normalize_keys(std::slice::from_ref(&input))?.remove(0);
    let store = Store::open()?;
    let (text, value) = status(&store, key).map_err(|e| fs_error(&input, &e))?;
    if json {
        print_json(&value);
    } else {
        print!("{text}");
    }
    Ok(EXIT_OK)
}
