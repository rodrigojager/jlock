//! Expiration (plan 7.1 step 5): tickets and holders whose heartbeat is older than `STALE_MS`, or
//! holders past `since + ttl`, are renamed into `broken/` (only one breaker wins the rename), read,
//! noted, and deleted.

use crate::fsx;
use crate::heartbeat::stale_after;
use crate::holder::{now_micros, Holder};
use crate::queue;
use crate::store::{read_body, KeyDir};
use crate::wait::random_u64;
use std::fs;
use std::io;
use std::path::Path;
use std::time::Duration;

#[derive(Debug)]
pub struct Expired {
    /// `ticket` or `holder`.
    pub what: &'static str,
    /// `stale` (no heartbeat) or `ttl`.
    pub reason: &'static str,
    pub key: String,
    pub body: Option<Holder>,
}

fn judge(body: Option<&Holder>, age_ok: bool) -> Option<&'static str> {
    if !age_ok {
        Some("stale")
    } else if body.is_some_and(|h| h.ttl_expired(now_micros())) {
        Some("ttl")
    } else {
        None
    }
}

/// Moves `src` into `broken/` and deletes it, returning its body. `Ok(None)` when someone else
/// broke it first, or when `alive(body, age)` says it was alive after all (renewed or replaced
/// between the check and the rename): then it is put back with a hard link, which never
/// overwrites.
pub fn break_file(
    kd: &KeyDir,
    src: &Path,
    suffix: &str,
    alive: impl Fn(Option<&Holder>, Duration) -> bool,
) -> io::Result<Option<Option<Holder>>> {
    for _ in 0..5 {
        fsx::retry(|| fs::create_dir_all(kd.broken_dir()))?;
        let target = kd.broken_dir().join(format!(
            "{:016}-{:04x}-{suffix}",
            now_micros(),
            random_u64() & 0xffff
        ));
        match fsx::rename_with_retry(src, &target) {
            Ok(()) => {
                let body = read_body(&target, &kd.key.text).ok().flatten();
                let age = fsx::age(&target).unwrap_or(Duration::MAX);
                let restored = alive(body.as_ref(), age) && fs::hard_link(&target, src).is_ok();
                fsx::remove_with_retry(&target)?;
                fsx::rmdir_if_empty(&kd.broken_dir())?;
                return Ok(if restored { None } else { Some(body) });
            }
            // Either the file is gone (someone else won) or `broken/` vanished under us.
            Err(e) if fsx::is_not_found(&e) => {
                if !src.exists() {
                    return Ok(None);
                }
            }
            Err(e) => return Err(e),
        }
    }
    Ok(None)
}

/// Expires dead tickets of the key (except `mine`).
pub fn expire_tickets(kd: &KeyDir, mine: Option<&str>) -> io::Result<Vec<Expired>> {
    let mut out = Vec::new();
    for name in queue::names(kd)? {
        if Some(name.as_str()) == mine {
            continue;
        }
        let path = kd.queue_dir().join(format!("{name}.json"));
        let age = match fsx::age(&path) {
            Ok(age) => age,
            Err(e) if fsx::is_not_found(&e) => continue,
            Err(e) => return Err(e),
        };
        if age <= stale_after() {
            continue;
        }
        if let Some(body) = break_file(kd, &path, &format!("{name}.json"), |_, age| {
            age <= stale_after()
        })? {
            out.push(Expired {
                what: "ticket",
                reason: "stale",
                key: kd.key.text.clone(),
                body,
            });
        }
    }
    Ok(out)
}

/// Expires the holder if its heartbeat is stale or its ttl has passed.
pub fn expire_holder(kd: &KeyDir) -> io::Result<Option<Expired>> {
    let path = kd.holder_path();
    let age = match fsx::age(&path) {
        Ok(age) => age,
        Err(e) if fsx::is_not_found(&e) => return Ok(None),
        Err(e) => return Err(e),
    };
    let body = kd.read_holder()?;
    let Some(reason) = judge(body.as_ref(), age <= stale_after()) else {
        return Ok(None);
    };
    Ok(break_file(kd, &path, "expired.json", |b, age| {
        judge(b, age <= stale_after()).is_none()
    })?
    .map(|body| Expired {
        what: "holder",
        reason,
        key: kd.key.text.clone(),
        body,
    }))
}

/// Everything dead in one key: tickets (except `mine`) and the holder. Each one is noted.
pub fn expire_key(kd: &KeyDir, mine: Option<&str>) -> io::Result<Vec<Expired>> {
    let mut out = expire_tickets(kd, mine)?;
    out.extend(expire_holder(kd)?);
    for e in &out {
        note(e);
    }
    Ok(out)
}

pub fn note(e: &Expired) {
    let owner = e.body.as_ref().map_or("?", |b| b.owner.as_str());
    crate::wait::debug(format!(
        "expired {} of {} on {} ({})",
        e.what, owner, e.key, e.reason
    ));
    let group = e.body.as_ref().and_then(|b| b.group.as_deref());
    let mut fields = crate::log::about(&e.key, owner, group);
    fields["what"] = serde_json::json!(e.what);
    fields["reason"] = serde_json::json!(e.reason);
    fields["body"] = serde_json::json!(e.body);
    crate::log::event("expired", fields);
}
