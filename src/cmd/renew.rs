//! `jlock renew`: moves the deadline of a live lock to now + ttl and renews its heartbeat. The
//! deadline goes to `holds/<grant>.lease` (see `lease.rs`); the hold file is only touched, so the
//! key never becomes free and a `hold` keeping it goes on as before.

use crate::cli::{
    fs_error, parse_duration, print_json, unexpected_rest, unknown_option, usage, Arg, CmdResult,
    Parser, EXIT_LOST, EXIT_NOT_OWNER, EXIT_OK,
};
use crate::cmd::acquire::normalize_keys;
use crate::expire;
use crate::fsx;
use crate::help::hint;
use crate::holder::{iso, now_micros, Ident};
use crate::i18n::{t, txt};
use crate::index;
use crate::lease::{self, Lease};
use crate::store::{Hold, KeyDir, Store};
use serde_json::json;
use std::io;

pub enum Renewal {
    /// Renewed; the new deadline, `None` for a lock without one (only the heartbeat).
    Done(Option<String>),
    NotOwner(Option<String>),
    /// Ours, but its deadline had already passed: expired now.
    Lost,
}

/// Renews every hold `owner` has on one key. `ttl` (seconds) overrides the ttl of each grant.
pub fn renew_key(kd: &KeyDir, owner: &str, ttl: Option<u64>) -> io::Result<Renewal> {
    let holds = kd.read_holds()?;
    let (mine, others): (Vec<Hold>, Vec<Hold>) =
        holds.into_iter().partition(|h| h.body.owner == owner);
    if mine.is_empty() {
        return Ok(Renewal::NotOwner(
            others.first().map(|h| h.body.owner.clone()),
        ));
    }
    let now = now_micros();
    if mine.iter().any(|h| lease::expired(kd, h, now)) {
        let _ = expire::expire_key(kd, None);
        let _ = kd.cleanup();
        let mut fields = crate::log::about(&kd.key.text, owner, mine[0].body.group.as_deref());
        fields["reason"] = json!("ttl");
        crate::log::event("lost", fields);
        return Ok(Renewal::Lost);
    }
    let mut last_until = None;
    let mut renewed = 0;
    for h in &mine {
        let until = match ttl.or(h.body.ttl_secs) {
            Some(secs) => {
                let until = iso(now + secs * 1_000_000);
                let body = Lease {
                    owner: owner.to_string(),
                    since: h.body.since.clone(),
                    until: until.clone(),
                };
                match lease::write(kd, &h.grant, &body) {
                    Ok(()) => {}
                    // Released and the folder removed meanwhile.
                    Err(e) if fsx::is_not_found(&e) => continue,
                    Err(e) => return Err(e),
                }
                Some(until)
            }
            None => None,
        };
        match fsx::touch(&h.path) {
            Ok(()) => {}
            Err(e) if fsx::is_not_found(&e) => {
                // The grant ended between the read and the write: its lease counts for nobody.
                let _ = kd.remove_lease(&h.grant);
                continue;
            }
            Err(e) => return Err(e),
        }
        renewed += 1;
        last_until = until.or(last_until);
    }
    if renewed == 0 {
        let _ = kd.cleanup();
        return Ok(Renewal::NotOwner(None));
    }
    let mut fields = crate::log::about(&kd.key.text, owner, mine[0].body.group.as_deref());
    fields["until"] = json!(last_until);
    crate::log::event("renew", fields);
    Ok(Renewal::Done(last_until))
}

pub fn run(args: Vec<String>) -> CmdResult {
    let mut p = Parser::new(args);
    let mut inputs = Vec::new();
    let mut owner = None;
    let mut group = None;
    let mut ttl = None;
    let mut json = false;
    while let Some(arg) = p.next() {
        match arg {
            Arg::Pos(k) => inputs.push(k),
            Arg::Flag(f) if f == "owner" => owner = Some(p.value(&f)?),
            Arg::Flag(f) if f == "group" => group = Some(p.value(&f)?),
            Arg::Flag(f) if f == "ttl" => {
                ttl = Some(parse_duration(&p.value(&f)?)?.as_secs().max(1));
            }
            Arg::Flag(f) if f == "json" => json = true,
            Arg::Flag(f) => return Err(unknown_option("renew", &f)),
            Arg::Rest(_) => return Err(unexpected_rest("renew")),
        }
    }
    if inputs.is_empty() && owner.is_none() && group.is_none() {
        return Err(usage(txt!("error.renew_needs")));
    }
    let owner = owner.unwrap_or_else(Ident::default_owner);
    let keys = normalize_keys(&inputs)?;
    let store = Store::open()?;
    let failed = |e: io::Error| fs_error(txt!("error.cannot_renew_all"), &e);

    // Explicit keys are renewed (or refused) one by one; without keys, every lock of the owner (in
    // the group, with --group).
    let targets: Vec<KeyDir> = if keys.is_empty() {
        index::scan(&store)
            .map_err(failed)?
            .into_iter()
            .filter(|st| {
                st.holds.iter().any(|h| {
                    h.body.owner == owner
                        && group
                            .as_ref()
                            .is_none_or(|g| h.body.group.as_ref() == Some(g))
                })
            })
            .map(|st| st.dir)
            .collect()
    } else {
        keys.iter().map(|k| store.key_dir(k)).collect()
    };

    let mut renewed = Vec::new();
    let mut refused = Vec::new();
    let mut lost = Vec::new();
    let mut last_until = None;
    // One key that cannot be written does not cost the others their renewal: it is reported at the end.
    let mut error = None;
    for kd in &targets {
        let outcome = match renew_key(kd, &owner, ttl) {
            Ok(outcome) => outcome,
            Err(e) => {
                error.get_or_insert(e);
                continue;
            }
        };
        match outcome {
            Renewal::Done(until) => {
                if !json {
                    match &until {
                        Some(u) => println!("{}", t!("renew.until", key = kd.key.text, until = u)),
                        None => println!("{}", t!("heartbeat.renewed", key = kd.key.text)),
                    }
                }
                renewed.push(kd.key.text.clone());
                last_until = until.or(last_until);
            }
            Renewal::NotOwner(holder) => {
                if keys.is_empty() {
                    continue; // changed hands while we looked: not ours to report
                }
                if !json {
                    match &holder {
                        Some(h) => {
                            println!("{}", t!("renew.not_owner", key = kd.key.text, holder = h));
                            hint(false, t!("tip.not_owner", owner = h));
                        }
                        None => println!("{}", t!("renew.not_locked", key = kd.key.text)),
                    }
                }
                refused.push(json!({ "key": kd.key.text, "holder": holder }));
            }
            Renewal::Lost => {
                if !json {
                    println!("{}", t!("result.lost", key = kd.key.text));
                    hint(false, t!("tip.lost", key = kd.key.text));
                }
                lost.push(kd.key.text.clone());
            }
        }
    }
    if let Some(e) = error {
        return Err(failed(e));
    }
    if !json && targets.is_empty() {
        println!("{}", t!("renew.nothing"));
    }
    let result = if !lost.is_empty() {
        "lost"
    } else if !refused.is_empty() {
        "not_owner"
    } else {
        "ok"
    };
    if json {
        let mut out = json!({ "result": result, "owner": owner, "keys": renewed,
                              "until": last_until });
        if keys.len() == 1 {
            out["key"] = json!(keys[0].text);
        }
        if !refused.is_empty() {
            out["not_owner"] = json!(refused);
        }
        if !lost.is_empty() {
            out["lost"] = json!(lost);
        }
        print_json(&out);
    }
    Ok(match result {
        "lost" => EXIT_LOST,
        "not_owner" => EXIT_NOT_OWNER,
        _ => EXIT_OK,
    })
}
