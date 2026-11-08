//! Expiration (plan 7.1 step 5): tickets and holds whose heartbeat is older than `STALE_MS`, holds
//! past their deadline (`since + ttl`, or the one `renew` set), and nested holds whose parent hold
//! is gone are renamed into `broken/` (only one breaker wins the rename), read, noted, and deleted.

use crate::fsx;
use crate::heartbeat::stale_after;
use crate::holder::{now_micros, Holder, Parent};
use crate::key::sha1_hex;
use crate::queue;
use crate::store::{read_body, Hold, KeyDir};
use crate::wait::random_u64;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::Duration;

#[derive(Debug)]
pub struct Expired {
    /// `ticket` or `holder`.
    pub what: &'static str,
    /// `stale` (no heartbeat), `ttl`, or `parent` (the hold it was nested in ended).
    pub reason: &'static str,
    pub key: String,
    /// The grant of an expired hold.
    pub grant: Option<String>,
    pub body: Option<Holder>,
}

/// Where the hold a nested hold lives in is, given the key folder of the nested hold.
pub fn parent_path(kd: &KeyDir, p: &Parent) -> PathBuf {
    let locks = kd.path.parent().unwrap_or(&kd.path);
    locks
        .join(sha1_hex(&p.key))
        .join("holds")
        .join(format!("{}.json", p.grant))
}

/// The hold a nested hold lives in has ended. Checked twice, a moment apart: a breaker that finds a
/// hold alive after all puts it back, and that instant must not end the nested holds.
pub fn parent_gone(kd: &KeyDir, body: &Holder) -> bool {
    let Some(p) = &body.parent else {
        return false;
    };
    let path = parent_path(kd, p);
    if path.exists() {
        return false;
    }
    sleep(Duration::from_millis(20));
    !path.exists()
}

fn judge(kd: &KeyDir, grant: &str, body: Option<&Holder>, age_ok: bool) -> Option<&'static str> {
    if !age_ok {
        Some("stale")
    } else if body
        .is_some_and(|h| crate::lease::deadline_of(kd, grant, h).is_some_and(|d| now_micros() > d))
    {
        Some("ttl")
    } else if body.is_some_and(|h| parent_gone(kd, h)) {
        Some("parent")
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
        fsx::ensure_dir(&kd.broken_dir())?;
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
                grant: None,
                body,
            });
        }
    }
    Ok(out)
}

/// Expires one hold if its heartbeat is stale, its ttl has passed, or the hold it was nested in
/// has ended.
pub fn expire_hold(kd: &KeyDir, hold: &Hold) -> io::Result<Option<Expired>> {
    let age = match fsx::age(&hold.path) {
        Ok(age) => age,
        Err(e) if fsx::is_not_found(&e) => return Ok(None),
        Err(e) => return Err(e),
    };
    let Some(reason) = judge(kd, &hold.grant, Some(&hold.body), age <= stale_after()) else {
        return Ok(None);
    };
    // A late heartbeat is not death while a `hold`/`run` still has the OS lock: the kernel would
    // have freed it. The ttl and the end of the parent, on the other hand, apply to the living too.
    if reason == "stale" && crate::oslock::is_held(&hold.path) {
        return Ok(None);
    }
    let grant = hold.grant.clone();
    let broken = break_file(
        kd,
        &hold.path,
        &format!("{grant}-expired.json"),
        |b, age| judge(kd, &grant, b, age <= stale_after()).is_none(),
    )?;
    Ok(broken.map(|body| {
        let _ = kd.remove_lease(&grant);
        Expired {
            what: "holder",
            reason,
            key: kd.key.text.clone(),
            grant: Some(grant),
            body,
        }
    }))
}

/// Expires every dead hold of the key.
pub fn expire_holds(kd: &KeyDir) -> io::Result<Vec<Expired>> {
    let mut out = Vec::new();
    for hold in kd.read_holds()? {
        out.extend(expire_hold(kd, &hold)?);
    }
    Ok(out)
}

/// Everything dead in one key: tickets (except `mine`) and holds. Each one is noted.
pub fn expire_key(kd: &KeyDir, mine: Option<&str>) -> io::Result<Vec<Expired>> {
    let mut out = expire_tickets(kd, mine)?;
    out.extend(expire_holds(kd)?);
    for e in &out {
        note(e);
    }
    Ok(out)
}

/// Ends the nested holds that lived in the holds just ended (`(key, grant)` pairs), and theirs in
/// turn, everywhere under `locks`. Each one is noted as expired with reason `parent`. Without this,
/// the next caller that looks at those keys would expire them anyway; this only does it at once.
pub fn end_nested(locks: &Path, mut ended: Vec<(String, String)>) -> io::Result<()> {
    while !ended.is_empty() {
        let mut next = Vec::new();
        for kd in crate::index::dirs_in(locks)? {
            for hold in kd.read_holds()? {
                let Some(p) = &hold.body.parent else { continue };
                if !ended.iter().any(|(k, g)| *k == p.key && *g == p.grant) {
                    continue;
                }
                if let Some(e) = expire_hold(&kd, &hold)? {
                    note(&e);
                    next.push((kd.key.text.clone(), hold.grant.clone()));
                }
                let _ = kd.cleanup();
            }
        }
        ended = next;
    }
    Ok(())
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
    fields["grant"] = serde_json::json!(e.grant);
    fields["body"] = serde_json::json!(e.body);
    crate::log::event("expired", fields);
}
