//! `jlock hold`: take the keys, then stay alive renewing the heartbeat until a `release` line on
//! stdin, CTRL+C/SIGTERM, or the end of the process. A lost lock ends it with 14.

use crate::cli::{fs_error, print_json, usage, CmdResult, EXIT_LOST, EXIT_OK};
use crate::cmd::acquire::{acquire_all, normalize_keys, parse, report, SetOutcome};
use crate::cmd::release::release_key;
use crate::expire;
use crate::heartbeat::{beat, heartbeat_interval, Beat};
use crate::key::Key;
use crate::store::Store;
use crate::wait;
use serde_json::json;
use std::io::BufRead;
use std::thread::sleep;
use std::time::{Duration, Instant};

pub enum Trouble {
    /// Someone else holds it now, or nobody (forced, expired, deleted).
    Lost { key: Key, holder: Option<String> },
    /// Our own ttl ran out.
    Ttl { key: Key },
}

/// Keeps a set of granted keys alive: renews each one every heartbeat interval.
pub struct Keeper<'a> {
    pub store: &'a Store,
    pub keys: Vec<Key>,
    pub owner: String,
    pub json: bool,
    next: Instant,
}

impl<'a> Keeper<'a> {
    pub fn new(store: &'a Store, keys: Vec<Key>, owner: &str, json: bool) -> Keeper<'a> {
        Keeper {
            store,
            keys,
            owner: owner.to_string(),
            json,
            next: Instant::now() + heartbeat_interval(),
        }
    }

    /// Renews the heartbeats when due; reports the first lock that is no longer ours.
    pub fn tick(&mut self) -> Option<Trouble> {
        if Instant::now() < self.next {
            return None;
        }
        self.next = Instant::now() + heartbeat_interval();
        for key in &self.keys {
            match beat(&self.store.key_dir(key), &self.owner) {
                Ok(Beat::Renewed) => {}
                Ok(Beat::Lost(holder)) => {
                    return Some(Trouble::Lost {
                        key: key.clone(),
                        holder,
                    })
                }
                Ok(Beat::TtlExpired) => return Some(Trouble::Ttl { key: key.clone() }),
                // A transient failure (file held by someone): try again on the next beat.
                Err(e) => wait::debug(format!("heartbeat on {} failed: {e}", key.text)),
            }
        }
        None
    }

    /// Releases every key that is still ours.
    pub fn release_all(&self) {
        for key in &self.keys {
            let _ = release_key(&self.store.key_dir(key), &self.owner);
        }
    }

    /// Reports a lost lock and lets go of the others. Messages go to stderr when `quiet_stdout`.
    pub fn lose(&self, trouble: &Trouble, quiet_stdout: bool) {
        let (key, holder, reason) = match trouble {
            Trouble::Lost { key, holder } => (key, holder.clone(), "lost"),
            Trouble::Ttl { key } => {
                let kd = self.store.key_dir(key);
                if let Ok(Some(e)) = expire::expire_holder(&kd) {
                    expire::note(&e);
                }
                let _ = kd.cleanup();
                (key, None, "ttl")
            }
        };
        wait::debug(format!("lost {} ({reason})", key.text));
        let line = if self.json {
            json!({ "result": "lost", "key": key.text, "owner": self.owner,
                    "holder": holder, "reason": reason })
            .to_string()
        } else {
            format!("lost {}", key.text)
        };
        if quiet_stdout {
            eprintln!("{line}");
        } else {
            println!("{line}");
        }
        for k in &self.keys {
            if k.text != key.text {
                let _ = release_key(&self.store.key_dir(k), &self.owner);
            }
        }
    }
}

fn watch_stdin() {
    std::thread::spawn(|| {
        for line in std::io::stdin().lock().lines() {
            match line {
                Ok(l) if l.trim() == "release" => {
                    wait::interrupt();
                    return;
                }
                Ok(_) => {}
                Err(_) => return,
            }
        }
    });
}

pub fn run(args: Vec<String>) -> CmdResult {
    let (opts, rest) = parse(args, "hold")?;
    if !rest.is_empty() {
        return Err(usage("hold: unexpected '--' (did you mean jlock run?)"));
    }
    wait::install_interrupt_handler();
    watch_stdin();
    let keys = normalize_keys(&opts.keys)?;
    let store = Store::open()?;
    let deadline = opts.timeout.map(|t| Instant::now() + t);
    let outcome = acquire_all(&store, &keys, &opts.ident, opts.wait, deadline)
        .map_err(|e| fs_error("cannot lock", &e))?;
    let code = report(&opts, &outcome);
    let SetOutcome::Granted { keys, .. } = outcome else {
        return Ok(code);
    };
    let mut keeper = Keeper::new(&store, keys, &opts.ident.owner, opts.json);
    while !wait::interrupted() {
        if let Some(trouble) = keeper.tick() {
            keeper.lose(&trouble, false);
            return Ok(EXIT_LOST);
        }
        sleep(Duration::from_millis(20));
    }
    keeper.release_all();
    if opts.json {
        let texts: Vec<&str> = keeper.keys.iter().map(|k| k.text.as_str()).collect();
        print_json(&json!({ "result": "released", "keys": texts, "owner": keeper.owner }));
    } else {
        for k in &keeper.keys {
            println!("released {}", k.text);
        }
    }
    Ok(EXIT_OK)
}
