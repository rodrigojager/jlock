//! `jlock unlock (<key> | --owner x | --group g) --force`: breaks someone else's locks, writing
//! down who forced and why. The owner finds out on its next heartbeat (`lost`) and stops. The holds
//! nested in a broken hold end with it.

use crate::cli::{
    fs_error, print_json, unexpected_rest, unknown_option, usage, Arg, CmdResult, Parser, EXIT_OK,
};
use crate::cmd::acquire::normalize_keys;
use crate::expire::break_file;
use crate::i18n::{t, txt};
use crate::index;
use crate::log;
use crate::store::{Hold, KeyDir, Store};
use serde_json::json;
use std::io;

pub fn default_by() -> String {
    let user = std::env::var(if cfg!(windows) { "USERNAME" } else { "USER" }).unwrap_or_default();
    format!("user:{user}")
}

/// Breaks one hold. Returns its owner if there was one to break.
pub fn force_hold(
    kd: &KeyDir,
    hold: &Hold,
    by: &str,
    reason: Option<&str>,
) -> io::Result<Option<String>> {
    let suffix = format!("{}-forced.json", hold.grant);
    let Some(body) = break_file(kd, &hold.path, &suffix, |_, _| false)? else {
        return Ok(None);
    };
    kd.remove_lease(&hold.grant)?;
    let owner = body.as_ref().map_or("?".to_string(), |b| b.owner.clone());
    let group = body.as_ref().and_then(|b| b.group.clone());
    let mut fields = log::about(&kd.key.text, &owner, group.as_deref());
    fields["by"] = json!(by);
    fields["reason"] = json!(reason);
    fields["grant"] = json!(hold.grant);
    fields["body"] = json!(body);
    log::event("forced", fields);
    if let Some(locks) = kd.path.parent() {
        crate::expire::end_nested(locks, vec![(kd.key.text.clone(), hold.grant.clone())])?;
    }
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
                _ => return Err(unknown_option("unlock", &f)),
            },
            Arg::Rest(_) => return Err(unexpected_rest("unlock")),
        }
    }
    if inputs.is_empty() && owner.is_none() && group.is_none() {
        return Err(usage(txt!("error.unlock_needs")));
    }
    if !force {
        return Err(usage(txt!("error.unlock_needs_force")));
    }
    let keys = normalize_keys(&inputs)?;
    let store = Store::open()?;
    let failed = |e: io::Error| fs_error(txt!("error.cannot_force"), &e);
    // Keys named: every hold on them. --owner / --group: those holds, wherever they are.
    let mut targets: Vec<(KeyDir, Hold)> = Vec::new();
    for k in &keys {
        let kd = store.key_dir(k);
        for h in kd.read_holds().map_err(failed)? {
            targets.push((kd.clone(), h));
        }
    }
    if owner.is_some() || group.is_some() {
        for st in index::scan(&store).map_err(failed)? {
            for h in &st.holds {
                let by_owner = owner.as_ref().is_some_and(|o| *o == h.body.owner);
                let by_group = group.is_some() && h.body.group == group;
                let named = targets.iter().any(|(_, t)| t.path == h.path);
                if (by_owner || by_group) && !named {
                    targets.push((st.dir.clone(), h.clone()));
                }
            }
        }
    }
    let mut forced = Vec::new();
    for (kd, hold) in &targets {
        if let Some(was) = force_hold(kd, hold, &by, reason.as_deref()).map_err(failed)? {
            if !json {
                println!("{}", t!("unlock.forced", key = kd.key.text, owner = was));
            }
            forced.push(json!({ "key": kd.key.text, "owner": was, "grant": hold.grant }));
        }
    }
    if json {
        print_json(&json!({ "result": "ok", "forced": forced, "by": by, "reason": reason }));
    } else if forced.is_empty() {
        println!("{}", t!("unlock.nothing"));
    }
    Ok(EXIT_OK)
}
