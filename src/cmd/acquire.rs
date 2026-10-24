//! `jlock acquire`: take a key by creating `holder.json` exclusively (plan section 7).

use crate::cli::{
    fs_error, parse_duration, print_json, usage, Arg, CmdResult, Failure, Parser, EXIT_BUSY,
    EXIT_OK,
};
use crate::fsx;
use crate::holder::{Holder, Ident};
use crate::key::{self, Key};
use crate::store::{KeyDir, Store};
use serde_json::json;
use std::io;
use std::time::Duration;

pub struct AcquireOpts {
    pub keys: Vec<String>,
    pub ident: Ident,
    pub wait: bool,
    pub timeout: Option<Duration>,
    pub json: bool,
}

/// Parses the options shared by `acquire`, `hold` and `run`; returns what follows `--`.
pub fn parse(args: Vec<String>, cmd: &str) -> Result<(AcquireOpts, Vec<String>), Failure> {
    let mut p = Parser::new(args);
    let mut opts = AcquireOpts {
        keys: Vec::new(),
        ident: Ident {
            owner: Ident::default_owner(),
            ..Ident::default()
        },
        wait: true,
        timeout: None,
        json: false,
    };
    let mut rest = Vec::new();
    while let Some(arg) = p.next() {
        match arg {
            Arg::Pos(k) => opts.keys.push(k),
            Arg::Rest(r) => rest = r,
            Arg::Flag(f) => match f.as_str() {
                "owner" => opts.ident.owner = nonempty(p.value(&f)?, &f)?,
                "group" => opts.ident.group = Some(nonempty(p.value(&f)?, &f)?),
                "intent" => opts.ident.intent = Some(p.value(&f)?),
                "meta" => {
                    let kv = p.value(&f)?;
                    let (k, v) = kv
                        .split_once('=')
                        .ok_or_else(|| usage(format!("--meta wants key=value, got '{kv}'")))?;
                    opts.ident.meta.insert(k.to_string(), v.to_string());
                }
                "ttl" => {
                    let d = parse_duration(&p.value(&f)?)?;
                    opts.ident.ttl_secs = Some(d.as_secs().max(1));
                }
                "wait" => opts.wait = true,
                "no-wait" => opts.wait = false,
                "timeout" => opts.timeout = Some(parse_duration(&p.value(&f)?)?),
                "json" => opts.json = true,
                _ => return Err(usage(format!("{cmd}: unknown option --{f}"))),
            },
        }
    }
    if opts.keys.is_empty() {
        return Err(usage(format!("{cmd} needs at least one key")));
    }
    Ok((opts, rest))
}

fn nonempty(v: String, flag: &str) -> Result<String, Failure> {
    if v.trim().is_empty() {
        Err(usage(format!("--{flag} must not be empty")))
    } else {
        Ok(v)
    }
}

pub fn normalize_keys(inputs: &[String]) -> Result<Vec<Key>, Failure> {
    let cwd =
        std::env::current_dir().map_err(|e| fs_error("cannot read the current directory", &e))?;
    let mut keys = Vec::new();
    for input in inputs {
        keys.push(key::normalize(input, &cwd).map_err(usage)?);
    }
    Ok(keys)
}

/// Result of one attempt at `holder.json`.
pub enum Take {
    Granted,
    Held(Box<Holder>),
}

/// Creates `holder.json` with `create_new`; on `EEXIST` reports who holds it. The same owner
/// asking again is granted without rewriting the file.
pub fn try_take(kd: &KeyDir, body: &Holder) -> io::Result<Take> {
    let bytes = body.to_bytes();
    loop {
        kd.ensure()?;
        match fsx::create_exclusive(&kd.holder_path(), &bytes) {
            Ok(()) => {
                kd.ensure_key_file()?;
                return Ok(Take::Granted);
            }
            Err(e) if fsx::is_exists(&e) => match kd.read_holder()? {
                Some(h) if h.owner == body.owner => return Ok(Take::Granted),
                Some(h) => return Ok(Take::Held(Box::new(h))),
                None => continue, // released in between: try again
            },
            Err(e) if fsx::is_not_found(&e) => continue, // folder removed by a release: recreate
            Err(e) => return Err(e),
        }
    }
}

pub fn run(args: Vec<String>) -> CmdResult {
    let (opts, rest) = parse(args, "acquire")?;
    if !rest.is_empty() {
        return Err(usage("acquire: unexpected '--' (did you mean jlock run?)"));
    }
    if opts.wait {
        return Err(usage(
            "acquire: waiting is not implemented yet (use --no-wait)",
        ));
    }
    let keys = normalize_keys(&opts.keys)?;
    let store = Store::open()?;
    let key = &keys[0];
    let kd = store.key_dir(key);
    let body = Holder::new(&key.text, &opts.ident);
    let take =
        try_take(&kd, &body).map_err(|e| fs_error(format!("cannot lock {}", key.text), &e))?;
    match take {
        Take::Granted => {
            if opts.json {
                print_json(&json!({
                    "result": "granted", "key": key.text, "keys": [key.text],
                    "owner": opts.ident.owner, "waited_ms": 0
                }));
            } else {
                println!("granted {}", key.text);
            }
            Ok(EXIT_OK)
        }
        Take::Held(h) => {
            if opts.json {
                print_json(&json!({
                    "result": "queued", "key": key.text, "owner": opts.ident.owner,
                    "holder": h, "position": 1
                }));
            } else {
                println!("busy {} held by {} (position 1)", key.text, h.owner);
            }
            Ok(EXIT_BUSY)
        }
    }
}
