//! `jlock release`: the owner deletes `holder.json`; the key folder goes away when nothing is left.

use crate::cli::{fs_error, print_json, usage, Arg, CmdResult, Parser, EXIT_NOT_OWNER, EXIT_OK};
use crate::cmd::acquire::normalize_keys;
use crate::fsx;
use crate::holder::Ident;
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
    let mut owner = Ident::default_owner();
    let mut json = false;
    while let Some(arg) = p.next() {
        match arg {
            Arg::Pos(k) => inputs.push(k),
            Arg::Flag(f) if f == "owner" => owner = p.value(&f)?,
            Arg::Flag(f) if f == "json" => json = true,
            Arg::Flag(f) => return Err(usage(format!("release: unknown option --{f}"))),
            Arg::Rest(_) => return Err(usage("release: unexpected '--'")),
        }
    }
    if inputs.is_empty() {
        return Err(usage("release needs a key"));
    }
    let keys = normalize_keys(&inputs)?;
    let store = Store::open()?;
    let mut released = Vec::new();
    let mut refused = Vec::new();
    for key in &keys {
        let kd = store.key_dir(key);
        match release_key(&kd, &owner)
            .map_err(|e| fs_error(format!("cannot release {}", key.text), &e))?
        {
            Released::Released => {
                if !json {
                    println!("released {}", key.text);
                }
                released.push(key.text.clone());
            }
            Released::NotOwner(holder) => {
                if !json {
                    match &holder {
                        Some(h) => println!("not_owner {} held by {h}", key.text),
                        None => println!("not_owner {} (not locked)", key.text),
                    }
                }
                refused.push(json!({ "key": key.text, "holder": holder }));
            }
        }
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
