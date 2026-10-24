//! `jlock release`: the owner deletes `holder.json`; the key folder goes away when nothing is left.

use crate::cli::{
    fs_error, print_json, unexpected_rest, unknown_option, usage, Arg, CmdResult, Parser,
    EXIT_NOT_OWNER, EXIT_OK,
};
use crate::cmd::acquire::normalize_keys;
use crate::fsx;
use crate::holder::Ident;
use crate::i18n::{t, txt};
use crate::index;
use crate::store::{KeyDir, Store};
use serde_json::json;
use std::io;

pub enum Released {
    Released,
    NotOwner(Option<String>),
}

/// Releases one key if `owner` holds it, then removes the key folder if it is empty.
pub fn release_key(kd: &KeyDir, owner: &str) -> io::Result<Released> {
    match kd.read_holder()? {
        Some(h) if h.owner == owner => {
            fsx::remove_with_retry(&kd.holder_path())?;
            let mut fields = crate::log::about(&kd.key.text, owner, h.group.as_deref());
            fields["held_ms"] = json!(crate::cmd::status::ms_since(&h.since));
            crate::log::event("release", fields);
            kd.cleanup()?;
            Ok(Released::Released)
        }
        Some(h) => Ok(Released::NotOwner(Some(h.owner))),
        None => Ok(Released::NotOwner(None)),
    }
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
            let Some(h) = &st.holder else { continue };
            let in_group = group.as_ref().is_none_or(|g| h.group.as_ref() == Some(g));
            if in_group && (h.owner == owner || any_owner) {
                let who = h.owner.clone();
                targets.push((st.dir, who));
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
                released.push(kd.key.text.clone());
            }
            Released::NotOwner(holder) => {
                if keys.is_empty() {
                    continue; // changed hands while we looked: not ours to report
                }
                if !json {
                    match &holder {
                        Some(h) => {
                            println!("{}", t!("release.not_owner", key = kd.key.text, holder = h))
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
