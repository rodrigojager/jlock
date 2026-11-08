//! `jlock status <key>`: the holds (owner, mode, nesting) and queue of one key, and the keys that
//! block it through the hierarchy (a folder above it, files inside it) or `*`.

use crate::cli::{
    fs_error, print_json, unexpected_rest, unknown_option, usage, Arg, CmdResult, Parser, EXIT_OK,
};
use crate::cmd::acquire::{check_resources, normalize_keys};
use crate::conflict::{conflicting_dirs, is_ancestor};
use crate::expire;
use crate::fsx;
use crate::heartbeat::stale_after;
use crate::holder::{iso, now_micros};
use crate::i18n::{t, txt, Text};
use crate::index;
use crate::key::{Key, Kind};
use crate::oslock;
use crate::store::{Hold, KeyDir, Store};
use serde_json::{json, Value};
use std::io;

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

/// `(dead, late)` for a hold. Dead: heartbeat older than STALE with no OS lock on the file, past
/// its deadline (ttl or `renew`), or nested in a hold that has ended; the expiration will break it.
/// Late: heartbeat older than STALE but a `hold`/`run` still has the OS lock, so the owner is alive
/// and the lock stays (see `expire.rs`).
pub fn liveness(kd: &KeyDir, h: &Hold) -> (bool, bool) {
    let stale_ms = stale_after().as_millis() as u64;
    let hb_stale = heartbeat_ms(&h.path).is_some_and(|ms| ms > stale_ms);
    let late = hb_stale && oslock::is_held(&h.path);
    let past = crate::lease::expired(kd, h, now_micros());
    let orphan = crate::expire::parent_gone(kd, &h.body);
    ((hb_stale && !late) || past || orphan, late)
}

/// When the hold expires (ISO time), `None` without a deadline.
pub fn expires(kd: &KeyDir, h: &Hold) -> Option<String> {
    crate::lease::deadline(kd, h).map(iso)
}

/// A hold as `status --json` and `list --json` show it: the body (`owner`, `mode`, `parent`,
/// `group`, ...) plus `grant`, `nested`, `expires`, `held_ms`, `heartbeat_ms`, `stale`, `late`.
pub fn holder_json(kd: &KeyDir, h: &Hold) -> Value {
    let (dead, late) = liveness(kd, h);
    let mut v = json!(h.body);
    v["grant"] = json!(h.grant);
    v["nested"] = json!(h.body.parent.is_some());
    v["parent"] = json!(h.body.parent);
    v["expires"] = json!(expires(kd, h));
    v["held_ms"] = json!(ms_since(&h.body.since));
    v["heartbeat_ms"] = json!(heartbeat_ms(&h.path));
    v["stale"] = json!(dead);
    v["late"] = json!(late);
    v
}

fn describe(kd: &KeyDir, hold: &Hold) -> String {
    let h = &hold.body;
    let mut line = t!(
        "status.by",
        owner = h.owner,
        mode = h.mode,
        since = h.since,
        ago = human(ms_since(&h.since))
    );
    if let Some(p) = &h.parent {
        line += &t!("status.nested", owner = p.owner, key = p.key);
    }
    if let Some(hb) = heartbeat_ms(&hold.path) {
        line += &t!("status.heartbeat", ago = human(hb));
        match liveness(kd, hold) {
            (true, _) => line += &t!("status.dead"),
            (false, true) => line += &t!("status.late"),
            _ => {}
        }
    }
    if let Some(d) = crate::lease::deadline(kd, hold) {
        let left = d.saturating_sub(now_micros()) / 1000;
        line += &t!("status.expires", left = human(left));
    }
    if let Some(g) = &h.group {
        line += &t!("status.group", group = g);
    }
    if let Some(i) = &h.intent {
        line += &t!("status.intent", intent = i);
    }
    line
}

/// `owner (mode)` of each hold, for the conflict lines.
fn owners(holds: &[Hold]) -> String {
    holds
        .iter()
        .map(|h| format!("{} ({})", h.body.owner, h.body.mode))
        .collect::<Vec<_>>()
        .join(", ")
}

fn status(store: &Store, key: Key) -> io::Result<(String, Value)> {
    let kd = store.key_dir(&key);
    if kd.exists() {
        expire::expire_key(&kd, None)?;
        let _ = kd.cleanup();
    }
    let st = index::state(store.key_dir(&key))?;
    let mut text = String::new();
    if st.holds.is_empty() {
        text += &format!("{}\n", t!("status.free", key = key.text));
    } else {
        text += &format!("{}\n", t!("status.held", key = key.text));
        for h in &st.holds {
            text += &format!("  {}\n", describe(&kd, h));
        }
    }
    let mut queue = Vec::new();
    if !st.tickets.is_empty() {
        text += &format!("{}\n", t!("status.queue"));
    }
    for (i, t) in st.tickets.iter().enumerate() {
        let waiting = ms_since(&t.body.since);
        text += &format!(
            "  {}\n",
            t!(
                "status.queue_item",
                position = i + 1,
                owner = t.body.owner,
                mode = t.body.mode,
                time = human(waiting)
            )
        );
        queue.push(
            json!({ "position": i + 1, "owner": t.body.owner, "mode": t.body.mode,
                           "since": t.body.since, "waiting_ms": waiting, "ticket": t.name }),
        );
    }
    let mut conflicts = Vec::new();
    let mut reasons: Vec<&str> = Vec::new();
    for cd in conflicting_dirs(store, &key)? {
        expire::expire_key(&cd, None)?;
        let cs = index::state(cd)?;
        if !cs.busy() {
            continue;
        }
        if conflicts.is_empty() {
            text += &format!("{}\n", t!("status.conflicts"));
        }
        text += "  ";
        if cs.holds.is_empty() {
            text += &t!("status.conflict_free", key = cs.dir.key.text);
        } else {
            text += &t!(
                "status.conflict_held",
                key = cs.dir.key.text,
                owner = owners(&cs.holds)
            );
        }
        if !cs.tickets.is_empty() {
            text += &crate::i18n::trn("status.conflict_waiting", cs.tickets.len() as u64, &[]);
        }
        text += "\n";
        let why = if cs.dir.key.kind == Kind::All || key.kind == Kind::All {
            "status.why_all"
        } else if is_ancestor(&cs.dir.key.text, &key.text) {
            "status.why_above"
        } else {
            "status.why_inside"
        };
        if !reasons.contains(&why) {
            reasons.push(why);
        }
        let holders: Vec<Value> = cs.holds.iter().map(|h| holder_json(&cs.dir, h)).collect();
        conflicts.push(json!({
            "key": cs.dir.key.text,
            "holder": holders.first(),
            "holders": holders,
            "queue": cs.tickets.len(),
        }));
    }
    // Why these keys block this one: a folder covers what is inside it, and `*` covers everything.
    for why in reasons {
        text += &crate::help::wrap(&t!(why), "  ", "    ", crate::help::COLUMNS);
    }
    let holders: Vec<Value> = st.holds.iter().map(|h| holder_json(&kd, h)).collect();
    let value = json!({
        "result": "ok",
        "key": key.text,
        "state": if holders.is_empty() { "free" } else { "held" },
        "holder": holders.first(),
        "holders": holders,
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
            Arg::Pos(k) => return Err(usage(txt!("error.status_extra", key = k))),
            Arg::Flag(f) => return Err(unknown_option("status", &f)),
            Arg::Rest(_) => return Err(unexpected_rest("status")),
        }
    }
    let input = input.ok_or_else(|| usage(txt!("error.status_needs")))?;
    // A resource is checked like when it is taken: a typo is refused, not reported as free.
    let key = check_resources(normalize_keys(std::slice::from_ref(&input))?)?
        .remove(0)
        .0;
    let store = Store::open()?;
    let (text, value) = status(&store, key).map_err(|e| fs_error(Text::plain(&input), &e))?;
    if json {
        print_json(&value);
    } else {
        print!("{text}");
    }
    Ok(EXIT_OK)
}
