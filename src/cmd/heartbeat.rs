//! `jlock heartbeat <key> --owner id`: renews a lock for callers that do not use `hold`.

use crate::cli::{
    fs_error, print_json, unexpected_rest, unknown_option, usage, Arg, CmdResult, Parser,
    EXIT_NOT_OWNER, EXIT_OK,
};
use crate::cmd::acquire::normalize_keys;
use crate::expire;
use crate::heartbeat::{beat, Beat};
use crate::help::hint;
use crate::holder::Ident;
use crate::i18n::{t, txt};
use crate::store::Store;
use serde_json::json;

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
            Arg::Flag(f) => return Err(unknown_option("heartbeat", &f)),
            Arg::Rest(_) => return Err(unexpected_rest("heartbeat")),
        }
    }
    if inputs.len() != 1 {
        return Err(usage(txt!("error.one_key", cmd = "heartbeat")));
    }
    let key = normalize_keys(&inputs)?.remove(0);
    let store = Store::open()?;
    let kd = store.key_dir(&key);
    let result =
        beat(&kd, &owner).map_err(|e| fs_error(txt!("error.cannot_renew", key = key.text), &e))?;
    let (holder, reason) = match result {
        Beat::Renewed => {
            if json {
                print_json(&json!({ "result": "ok", "key": key.text, "owner": owner }));
            } else {
                println!("{}", t!("heartbeat.renewed", key = key.text));
            }
            return Ok(EXIT_OK);
        }
        Beat::Lost(holder) => (holder, "lost"),
        Beat::TtlExpired => {
            if let Ok(Some(e)) = expire::expire_holder(&kd) {
                expire::note(&e);
            }
            let _ = kd.cleanup();
            (None, "ttl")
        }
    };
    let mut fields = crate::log::about(&key.text, &owner, None);
    fields["reason"] = json!(reason);
    crate::log::event("lost", fields);
    if json {
        print_json(&json!({ "result": "lost", "key": key.text, "owner": owner,
                            "holder": holder, "reason": reason }));
    } else {
        println!("{}", t!("result.lost", key = key.text));
        hint(false, t!("tip.lost", key = key.text));
    }
    Ok(EXIT_NOT_OWNER)
}
