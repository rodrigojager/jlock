//! `jlock release`: the owner deletes its holds of a key; the holds nested in them end too; the key
//! folder goes away when nothing is left.

use crate::cli::{
    fs_error, print_json, unexpected_rest, unknown_option, usage, Arg, CmdResult, Parser,
    EXIT_NOT_OWNER, EXIT_OK,
};
use crate::cmd::acquire::normalize_keys;
use crate::fsx;
use crate::help::hint;
use crate::holder::Ident;
use crate::i18n::{t, txt};
use crate::index;
use crate::store::Store;
use crate::store::{Hold, KeyDir};
use serde_json::json;
use std::io;

pub enum Released {
    Released,
    NotOwner(Option<String>),
}

/// Deletes the given holds of one key (all of one owner), notes the release, ends the holds nested
/// in them and removes the key folder if it is empty. Holds that vanished meanwhile are skipped.
fn release_holds(kd: &KeyDir, owner: &str, holds: &[Hold]) -> io::Result<bool> {
    let mut gone = Vec::new();
    for h in holds {
        if fsx::remove_with_retry(&h.path)? {
            kd.remove_lease(&h.grant)?;
            gone.push(h);
        }
    }
    let Some(first) = gone.first() else {
        return Ok(false);
    };
    let mut fields = crate::log::about(&kd.key.text, owner, first.body.group.as_deref());
    fields["held_ms"] = json!(crate::cmd::status::ms_since(&first.body.since));
    fields["grants"] = json!(gone.iter().map(|h| h.grant.clone()).collect::<Vec<_>>());
    crate::log::event("release", fields);
    if let Some(locks) = kd.path.parent() {
        let ended = gone
            .iter()
            .map(|h| (kd.key.text.clone(), h.grant.clone()))
            .collect();
        crate::expire::end_nested(locks, ended)?;
    }
    kd.cleanup()?;
    Ok(true)
}

/// Releases every hold `owner` has on one key, then removes the key folder if it is empty.
pub fn release_key(kd: &KeyDir, owner: &str) -> io::Result<Released> {
    let holds = kd.read_holds()?;
    let (mine, others): (Vec<Hold>, Vec<Hold>) =
        holds.into_iter().partition(|h| h.body.owner == owner);
    if !mine.is_empty() && release_holds(kd, owner, &mine)? {
        return Ok(Released::Released);
    }
    Ok(Released::NotOwner(
        others.first().map(|h| h.body.owner.clone()),
    ))
}

/// Releases one grant of `owner` (the undo of a set that could not be completed).
pub fn release_grant(kd: &KeyDir, grant: &str, owner: &str) -> io::Result<bool> {
    let mine: Vec<Hold> = kd
        .holds_of(owner)?
        .into_iter()
        .filter(|h| h.grant == grant)
        .collect();
    release_holds(kd, owner, &mine)
}

pub fn run(args: Vec<String>) -> CmdResult {
    let mut p = Parser::new(args);
    let mut inputs = Vec::new();
    let mut owner = None;
    let mut group = None;
    let (mut any_owner, mut json) = (false, false);
    while let Some(arg) = p.next() {
        match arg {
            Arg::Pos(k) => inputs.push(k),
            Arg::Flag(f) if f == "owner" => owner = Some(p.value(&f)?),
            Arg::Flag(f) if f == "group" => group = Some(p.value(&f)?),
            Arg::Flag(f) if f == "any-owner" => any_owner = true,
            Arg::Flag(f) if f == "json" => json = true,
            Arg::Flag(f) => return Err(unknown_option("release", &f)),
            Arg::Rest(_) => return Err(unexpected_rest("release")),
        }
    }
    if inputs.is_empty() && owner.is_none() && group.is_none() {
        return Err(usage(txt!("error.release_needs")));
    }
    if any_owner && group.is_none() {
        return Err(usage(txt!("error.any_owner_group")));
    }
    let owner = owner.unwrap_or_else(Ident::default_owner);
    let keys = normalize_keys(&inputs)?;
    let store = Store::open()?;
    let failed = |e: io::Error| fs_error(txt!("error.cannot_release"), &e);

    // Explicit keys are released (or refused) one by one; without keys, every lock of the owner,
    // or of the group, is released and the others are left alone.
    let mut targets: Vec<(KeyDir, String)> = Vec::new();
    if keys.is_empty() {
        for st in index::scan(&store).map_err(failed)? {
            let mut whose: Vec<String> = Vec::new();
            for h in &st.holds {
                let in_group = group
                    .as_ref()
                    .is_none_or(|g| h.body.group.as_ref() == Some(g));
                if in_group
                    && (h.body.owner == owner || any_owner)
                    && !whose.contains(&h.body.owner)
                {
                    whose.push(h.body.owner.clone());
                }
            }
            for who in whose {
                targets.push((st.dir.clone(), who));
            }
        }
    } else {
        targets = keys
            .iter()
            .map(|k| (store.key_dir(k), owner.clone()))
            .collect();
    }

    let mut released = Vec::new();
    let mut refused = Vec::new();
    for (kd, who) in &targets {
        match release_key(kd, who).map_err(failed)? {
            Released::Released => {
                if !json {
                    println!("{}", t!("result.released", key = kd.key.text));
                }
                if !released.contains(&kd.key.text) {
                    released.push(kd.key.text.clone());
                }
            }
            Released::NotOwner(holder) => {
                if keys.is_empty() {
                    continue; // changed hands while we looked: not ours to report
                }
                if !json {
                    match &holder {
                        Some(h) => {
                            println!("{}", t!("release.not_owner", key = kd.key.text, holder = h));
                            hint(false, t!("tip.not_owner", owner = h));
                        }
                        None => println!("{}", t!("release.not_locked", key = kd.key.text)),
                    }
                }
                refused.push(json!({ "key": kd.key.text, "holder": holder }));
            }
        }
    }
    if !json && targets.is_empty() {
        println!("{}", t!("release.nothing"));
    }
    let ok = refused.is_empty();
    if json {
        let mut out = json!({
            "result": if ok { "released" } else { "not_owner" },
            "owner": owner, "keys": released,
        });
        if keys.len() == 1 {
            out["key"] = json!(keys[0].text);
        }
        if !ok {
            out["not_owner"] = json!(refused);
        }
        print_json(&out);
    }
    Ok(if ok { EXIT_OK } else { EXIT_NOT_OWNER })
}
