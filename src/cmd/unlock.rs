//! `jlock unlock (<key> | --owner x | --group g) --force`: breaks someone else's locks, writing
//! down who forced and why. The owner finds out on its next heartbeat (`lost`) and stops.

use crate::cli::{fs_error, print_json, usage, Arg, CmdResult, Parser, EXIT_OK};
use crate::cmd::acquire::normalize_keys;
use crate::expire::break_file;
use crate::index;
use crate::log;
use crate::store::{KeyDir, Store};
use serde_json::json;
use std::io;

pub fn default_by() -> String {
    let user = std::env::var(if cfg!(windows) { "USERNAME" } else { "USER" }).unwrap_or_default();
    format!("user:{user}")
}

/// Breaks the holder of one key. Returns its owner if there was one to break.
pub fn force_key(kd: &KeyDir, by: &str, reason: Option<&str>) -> io::Result<Option<String>> {
    if !kd.holder_path().exists() {
        return Ok(None);
    }
    let Some(body) = break_file(kd, &kd.holder_path(), "forced.json", |_, _| false)? else {
        return Ok(None);
    };
    let owner = body.as_ref().map_or("?".to_string(), |b| b.owner.clone());
    let group = body.as_ref().and_then(|b| b.group.clone());
    let mut fields = log::about(&kd.key.text, &owner, group.as_deref());
    fields["by"] = json!(by);
    fields["reason"] = json!(reason);
    fields["body"] = json!(body);
    log::event("forced", fields);
    let _ = kd.cleanup();
    Ok(Some(owner))
}

pub fn run(args: Vec<String>) -> CmdResult {
    let mut p = Parser::new(args);
    let mut inputs = Vec::new();
    let (mut owner, mut group, mut reason) = (None, None, None);
    let mut by = default_by();
    let (mut force, mut json) = (false, false);
    while let Some(arg) = p.next() {
        match arg {
            Arg::Pos(k) => inputs.push(k),
            Arg::Flag(f) => match f.as_str() {
                "owner" => owner = Some(p.value(&f)?),
                "group" => group = Some(p.value(&f)?),
                "reason" => reason = Some(p.value(&f)?),
                "by" => by = p.value(&f)?,
                "force" => force = true,
                "json" => json = true,
                _ => return Err(usage(format!("unlock: unknown option --{f}"))),
            },
            Arg::Rest(_) => return Err(usage("unlock: unexpected '--'")),
        }
    }
    if inputs.is_empty() && owner.is_none() && group.is_none() {
        return Err(usage("unlock needs a key, --owner or --group"));
    }
    if !force {
        return Err(usage(
            "unlock breaks other people's locks: add --force if that is what you mean \
             (to give back your own lock, use jlock release)",
        ));
    }
    let keys = normalize_keys(&inputs)?;
    let store = Store::open()?;
    let failed = |e: io::Error| fs_error("cannot force", &e);
    let mut targets: Vec<KeyDir> = keys.iter().map(|k| store.key_dir(k)).collect();
    if owner.is_some() || group.is_some() {
        for st in index::scan(&store).map_err(failed)? {
            let Some(h) = &st.holder else { continue };
            let by_owner = owner.as_ref().is_some_and(|o| *o == h.owner);
            let by_group = group.is_some() && h.group == group;
            if (by_owner || by_group) && !targets.iter().any(|t| t.key.text == st.dir.key.text) {
                targets.push(st.dir);
            }
        }
    }
    let mut forced = Vec::new();
    for kd in &targets {
        if let Some(was) = force_key(kd, &by, reason.as_deref()).map_err(failed)? {
            if !json {
                println!("forced {} (was {was})", kd.key.text);
            }
            forced.push(json!({ "key": kd.key.text, "owner": was }));
        }
    }
    if json {
        print_json(&json!({ "result": "ok", "forced": forced, "by": by, "reason": reason }));
    } else if forced.is_empty() {
        println!("nothing to force");
    }
    Ok(EXIT_OK)
}
