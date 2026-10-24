//! `jlock acquire`: a ticket in the key's queue, then `holder.json` created exclusively when the
//! ticket is the oldest one alive (plan section 7).

use crate::cli::{
    fs_error, parse_duration, print_json, usage, Arg, CmdResult, Failure, Parser, EXIT_BUSY,
    EXIT_OK, EXIT_TIMEOUT,
};
use crate::fsx;
use crate::holder::{Holder, Ident};
use crate::key::{self, Key};
use crate::queue::{self, Ticket};
use crate::store::{KeyDir, Store};
use crate::wait;
use serde_json::json;
use std::io;
use std::time::{Duration, Instant};

pub const EXIT_INTERRUPTED: i32 = 130;

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

pub enum Outcome {
    Granted {
        waited_ms: u64,
    },
    Busy {
        holder: Option<Holder>,
        position: usize,
    },
    Timeout {
        holder: Option<Holder>,
        position: usize,
    },
    Interrupted,
}

/// What the waiter sees on one poll.
struct View {
    holder: Option<Holder>,
    /// Tickets of other owners ahead of mine.
    ahead: usize,
    /// My place in the queue, 1-based.
    position: usize,
}

fn look(kd: &KeyDir, owner: &str, mine: Option<&str>) -> io::Result<View> {
    let tickets = queue::list(kd)?;
    let (before, position): (Vec<&Ticket>, usize) = match mine {
        Some(name) => {
            let before: Vec<&Ticket> = tickets.iter().filter(|t| t.name.as_str() < name).collect();
            let n = before.len();
            (before, n + 1)
        }
        None => (tickets.iter().collect(), tickets.len() + 1),
    };
    Ok(View {
        holder: kd.read_holder()?,
        ahead: before.iter().filter(|t| t.body.owner != owner).count(),
        position,
    })
}

/// Takes one key for `ident`: at once or never with `wait == false`; otherwise in arrival order,
/// up to `deadline`.
pub fn acquire_one(
    store: &Store,
    key: &Key,
    ident: &Ident,
    wait_turn: bool,
    deadline: Option<Instant>,
) -> io::Result<Outcome> {
    let start = Instant::now();
    let kd = store.key_dir(key);
    let body = Holder::new(&key.text, ident);

    if !wait_turn {
        let view = look(&kd, &ident.owner, None)?;
        if let Some(h) = &view.holder {
            if h.owner == ident.owner {
                return Ok(Outcome::Granted { waited_ms: 0 });
            }
        }
        if view.holder.is_none() && view.ahead == 0 {
            match try_take(&kd, &body)? {
                Take::Granted => return Ok(Outcome::Granted { waited_ms: 0 }),
                Take::Held(h) => {
                    return Ok(Outcome::Busy {
                        holder: Some(*h),
                        position: view.position,
                    })
                }
            }
        }
        return Ok(Outcome::Busy {
            holder: view.holder,
            position: view.position,
        });
    }

    if let Some(h) = kd.read_holder()? {
        if h.owner == ident.owner {
            return Ok(Outcome::Granted { waited_ms: 0 });
        }
    }
    let mut ticket = queue::create(&kd, &body)?;
    wait::debug(format!("queued {} as {}", key.text, ticket.name));
    let result = wait_turn_loop(&kd, &body, &mut ticket, start, deadline);
    if result.is_err() {
        let _ = queue::remove(&ticket);
    }
    result
}

fn wait_turn_loop(
    kd: &KeyDir,
    body: &Holder,
    ticket: &mut Ticket,
    start: Instant,
    deadline: Option<Instant>,
) -> io::Result<Outcome> {
    let poll = wait::poll_ms();
    let owner = body.owner.as_str();
    let granted = |ticket: &Ticket| -> io::Result<Outcome> {
        queue::remove(ticket)?;
        Ok(Outcome::Granted {
            waited_ms: start.elapsed().as_millis() as u64,
        })
    };
    loop {
        if wait::interrupted() {
            queue::remove(ticket)?;
            return Ok(Outcome::Interrupted);
        }
        match fsx::touch(&ticket.path) {
            Ok(()) => {}
            // My ticket was removed under me: queue again, at the back.
            Err(e) if fsx::is_not_found(&e) => *ticket = queue::create(kd, body)?,
            Err(e) => return Err(e),
        }
        kd.ensure_key_file()?;
        let view = look(kd, owner, Some(&ticket.name))?;
        match &view.holder {
            Some(h) if h.owner == owner => return granted(ticket),
            None if view.ahead == 0 => match try_take(kd, body)? {
                Take::Granted => return granted(ticket),
                Take::Held(h) => wait::debug(format!(
                    "EEXIST on {}: {} got in first",
                    kd.key.text, h.owner
                )),
            },
            _ => {}
        }
        if deadline.is_some_and(|d| Instant::now() >= d) {
            queue::remove(ticket)?;
            return Ok(Outcome::Timeout {
                holder: view.holder,
                position: view.position,
            });
        }
        wait::pause(poll, deadline);
    }
}

/// Prints the outcome (one line, or one JSON object) and returns the exit code.
pub fn report(opts: &AcquireOpts, key: &Key, outcome: &Outcome) -> i32 {
    let owner = &opts.ident.owner;
    let (result, code, extra) = match outcome {
        Outcome::Granted { waited_ms } => {
            if !opts.json {
                println!("granted {}", key.text);
            }
            ("granted", EXIT_OK, json!({ "waited_ms": waited_ms }))
        }
        Outcome::Busy { holder, position } | Outcome::Timeout { holder, position } => {
            let (result, word, code) = match outcome {
                Outcome::Busy { .. } => ("queued", "busy", EXIT_BUSY),
                _ => ("timeout", "timeout", EXIT_TIMEOUT),
            };
            if !opts.json {
                let by = holder
                    .as_ref()
                    .map_or("nobody (queue ahead)", |h| h.owner.as_str());
                println!("{word} {} held by {by} (position {position})", key.text);
            }
            (
                result,
                code,
                json!({ "holder": holder, "position": position }),
            )
        }
        Outcome::Interrupted => {
            if !opts.json {
                println!("interrupted {}", key.text);
            }
            ("error", EXIT_INTERRUPTED, json!({ "error": "interrupted" }))
        }
    };
    if opts.json {
        let mut out =
            json!({ "result": result, "key": key.text, "keys": [key.text], "owner": owner });
        for (k, v) in extra.as_object().unwrap() {
            out[k] = v.clone();
        }
        print_json(&out);
    }
    code
}

pub fn run(args: Vec<String>) -> CmdResult {
    let (opts, rest) = parse(args, "acquire")?;
    if !rest.is_empty() {
        return Err(usage("acquire: unexpected '--' (did you mean jlock run?)"));
    }
    if opts.keys.len() > 1 {
        return Err(usage("acquire: one key per call for now"));
    }
    wait::install_interrupt_handler();
    let keys = normalize_keys(&opts.keys)?;
    let store = Store::open()?;
    let key = &keys[0];
    let deadline = opts.timeout.map(|t| Instant::now() + t);
    let outcome = acquire_one(&store, key, &opts.ident, opts.wait, deadline)
        .map_err(|e| fs_error(format!("cannot lock {}", key.text), &e))?;
    Ok(report(&opts, key, &outcome))
}
