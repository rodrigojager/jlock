//! `jlock acquire`: a ticket in the key's queue, then `holder.json` created exclusively when the
//! ticket is the oldest one alive and no conflicting key is taken (plan section 7).

use crate::cli::{
    fs_error, parse_duration, print_json, usage, Arg, CmdResult, Failure, Parser, EXIT_BUSY,
    EXIT_DEADLOCK, EXIT_OK, EXIT_TIMEOUT,
};
use crate::conflict;
use crate::expire;
use crate::fsx;
use crate::graph;
use crate::holder::{iso, now_micros, Holder, Ident};
use crate::index;
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
    /// Created by this call.
    Granted,
    /// Already held by the same owner.
    Already,
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
                Some(h) if h.owner == body.owner => return Ok(Take::Already),
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
    /// Not taken with `--no-wait`. `via` names the conflicting key when the block comes from the
    /// hierarchy (or `*`) rather than from the key itself.
    Busy {
        holder: Option<Holder>,
        position: usize,
        via: Option<String>,
    },
    Timeout {
        holder: Option<Holder>,
        position: usize,
        via: Option<String>,
    },
    Interrupted,
    /// Waiting would close a cycle in "who waits for whom".
    Deadlock {
        chain: String,
    },
}

/// Would `owner` waiting for `key` (with a ticket younger than all others) close a cycle?
fn deadlock_before_queueing(store: &Store, key: &Key, owner: &str) -> io::Result<Option<String>> {
    let states = index::scan(store)?;
    let all = graph::edges(&states);
    let proposed = graph::edges_for(owner, key, None, &states);
    let proposed: Vec<&graph::Edge> = proposed.iter().collect();
    Ok(graph::cycle_through(&all, &proposed).map(|c| graph::chain_text(&c)))
}

/// Is my ticket part of a cycle in which it is the newest ticket (the one that gives up)?
fn deadlock_while_waiting(store: &Store, ticket: &Ticket) -> io::Result<Option<String>> {
    let states = index::scan(store)?;
    let all = graph::edges(&states);
    let mine: Vec<&graph::Edge> = all.iter().filter(|e| e.ticket == ticket.name).collect();
    let Some(cycle) = graph::cycle_through(&all, &mine) else {
        return Ok(None);
    };
    let newest = cycle.iter().map(|e| e.ticket.as_str()).max();
    Ok((newest == Some(ticket.name.as_str())).then(|| graph::chain_text(&cycle)))
}

/// What the waiter sees on one poll.
struct View {
    holder: Option<Holder>,
    /// Tickets of other owners ahead of mine.
    ahead: usize,
    /// My place in the queue, 1-based.
    position: usize,
    /// A conflicting key held, or with an older ticket, by another owner (and its holder).
    blocker: Option<(String, Option<Holder>)>,
}

/// Is `t` ahead of my ticket `mine` (every ticket is ahead when I have none)?
fn is_ahead(t: &Ticket, mine: Option<&str>) -> bool {
    mine.is_none_or(|m| t.name.as_str() < m)
}

fn look(store: &Store, kd: &KeyDir, owner: &str, mine: Option<&str>) -> io::Result<View> {
    let tickets = queue::list(kd)?;
    let before: Vec<&Ticket> = tickets.iter().filter(|t| is_ahead(t, mine)).collect();
    let mut blocker = None;
    for cd in conflict::conflicting_dirs(store, &kd.key)? {
        expire::expire_key(&cd, None)?;
        if let Some(h) = cd.read_holder()?.filter(|h| h.owner != owner) {
            blocker = Some((cd.key.text.clone(), Some(h)));
            break;
        }
        let older = queue::list(&cd)?
            .iter()
            .any(|t| t.body.owner != owner && is_ahead(t, mine));
        if older {
            blocker = Some((cd.key.text.clone(), None));
            break;
        }
    }
    Ok(View {
        holder: kd.read_holder()?,
        ahead: before.iter().filter(|t| t.body.owner != owner).count(),
        position: before.len() + 1,
        blocker,
    })
}

/// A conflicting key held by another owner right now.
fn conflicting_holder(
    store: &Store,
    key: &Key,
    owner: &str,
) -> io::Result<Option<(String, Holder)>> {
    for cd in conflict::conflicting_dirs(store, key)? {
        if let Some(h) = cd.read_holder()?.filter(|h| h.owner != owner) {
            return Ok(Some((cd.key.text.clone(), h)));
        }
    }
    Ok(None)
}

/// After creating `holder.json`, makes sure no conflicting key was taken at the same instant by
/// someone who could not see us yet. If one was, ours is withdrawn and the caller goes on
/// waiting; the other side, seeing ours, does the same, or had been granted before us.
fn confirm(store: &Store, kd: &KeyDir, owner: &str) -> io::Result<Option<(String, Holder)>> {
    let clash = conflicting_holder(store, &kd.key, owner)?;
    if let Some((via, h)) = &clash {
        wait::debug(format!(
            "withdrawing {}: {via} taken by {}",
            kd.key.text, h.owner
        ));
        fsx::remove_with_retry(&kd.holder_path())?;
    }
    Ok(clash)
}

/// The holder body as written at the grant: `since` is the moment the lock was taken.
fn granted_now(body: &Holder) -> Holder {
    let mut b = body.clone();
    b.since = iso(now_micros());
    b
}

/// Takes one key for `ident`: at once or never with `wait == false`; otherwise in arrival order,
/// up to `deadline`. Conflicting keys (a folder and what is inside it, `*`) are waited for too.
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
    let owner = ident.owner.as_str();

    if let Some(h) = kd.read_holder()? {
        if h.owner == owner {
            return Ok(Outcome::Granted { waited_ms: 0 });
        }
    }

    if !wait_turn {
        expire::expire_key(&kd, None)?;
        let view = look(store, &kd, owner, None)?;
        let position = view.position;
        let busy = |holder, via| Outcome::Busy {
            holder,
            position,
            via,
        };
        if let Some((via, holder)) = view.blocker {
            let _ = kd.cleanup();
            return Ok(busy(holder, Some(via)));
        }
        if view.holder.is_some() || view.ahead > 0 {
            return Ok(busy(view.holder, None));
        }
        return match try_take(&kd, &body)? {
            Take::Already => Ok(Outcome::Granted { waited_ms: 0 }),
            Take::Granted => match confirm(store, &kd, owner)? {
                None => Ok(Outcome::Granted { waited_ms: 0 }),
                Some((via, h)) => {
                    let _ = kd.cleanup();
                    Ok(busy(Some(h), Some(via)))
                }
            },
            Take::Held(h) => Ok(busy(Some(*h), None)),
        };
    }

    if let Some(chain) = deadlock_before_queueing(store, key, owner)? {
        return Ok(Outcome::Deadlock { chain });
    }
    let mut ticket = queue::create(&kd, &body)?;
    wait::debug(format!("queued {} as {}", key.text, ticket.name));
    let result = wait_turn_loop(store, &kd, &body, &mut ticket, start, deadline);
    if result.is_err() {
        let _ = queue::remove(&ticket);
    }
    result
}

fn wait_turn_loop(
    store: &Store,
    kd: &KeyDir,
    body: &Holder,
    ticket: &mut Ticket,
    start: Instant,
    deadline: Option<Instant>,
) -> io::Result<Outcome> {
    let poll = wait::poll_ms();
    let owner = body.owner.as_str();
    let mut polls: u64 = 0;
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
        expire::expire_key(kd, Some(&ticket.name))?;
        let view = look(store, kd, owner, Some(&ticket.name))?;
        match &view.holder {
            Some(h) if h.owner == owner => return granted(ticket),
            None if view.ahead == 0 && view.blocker.is_none() => {
                match try_take(kd, &granted_now(body))? {
                    Take::Already => return granted(ticket),
                    Take::Granted => {
                        if confirm(store, kd, owner)?.is_none() {
                            return granted(ticket);
                        }
                    }
                    Take::Held(h) => wait::debug(format!(
                        "EEXIST on {}: {} got in first",
                        kd.key.text, h.owner
                    )),
                }
            }
            _ => {}
        }
        if deadline.is_some_and(|d| Instant::now() >= d) {
            queue::remove(ticket)?;
            let (holder, via) = match (view.holder, view.blocker) {
                (Some(h), _) => (Some(h), None),
                (None, Some((via, h))) => (h, Some(via)),
                (None, None) => (None, None),
            };
            return Ok(Outcome::Timeout {
                holder,
                position: view.position,
                via,
            });
        }
        polls += 1;
        if polls.is_multiple_of(8) {
            if let Some(chain) = deadlock_while_waiting(store, ticket)? {
                queue::remove(ticket)?;
                return Ok(Outcome::Deadlock { chain });
            }
        }
        wait::pause(poll, deadline);
    }
}

#[allow(clippy::large_enum_variant)] // built once per command
pub enum SetOutcome {
    Granted {
        keys: Vec<Key>,
        waited_ms: u64,
    },
    /// `key` was not obtained; the keys taken before it were released again.
    Failed {
        key: Key,
        outcome: Outcome,
        released: Vec<Key>,
    },
}

/// Canonical order: sorted by normalized text, duplicates dropped.
pub fn canonical(keys: &[Key]) -> Vec<Key> {
    let mut keys = keys.to_vec();
    keys.sort_by(|a, b| a.text.cmp(&b.text));
    keys.dedup_by(|a, b| a.text == b.text);
    keys
}

/// Takes all keys in canonical order, or none: on the first failure the ones already taken (and
/// not held by this owner before) are released.
pub fn acquire_all(
    store: &Store,
    keys: &[Key],
    ident: &Ident,
    wait_turn: bool,
    deadline: Option<Instant>,
) -> io::Result<SetOutcome> {
    let start = Instant::now();
    let mut taken: Vec<Key> = Vec::new();
    let undo = |taken: &[Key]| {
        for k in taken {
            let _ = crate::cmd::release::release_key(&store.key_dir(k), &ident.owner);
        }
    };
    for key in canonical(keys) {
        let had_it = store
            .key_dir(&key)
            .read_holder()
            .ok()
            .flatten()
            .is_some_and(|h| h.owner == ident.owner);
        match acquire_one(store, &key, ident, wait_turn, deadline) {
            Ok(Outcome::Granted { .. }) => {
                if !had_it {
                    taken.push(key);
                }
            }
            Ok(outcome) => {
                undo(&taken);
                return Ok(SetOutcome::Failed {
                    key,
                    outcome,
                    released: taken,
                });
            }
            Err(e) => {
                undo(&taken);
                return Err(e);
            }
        }
    }
    Ok(SetOutcome::Granted {
        keys: canonical(keys),
        waited_ms: start.elapsed().as_millis() as u64,
    })
}

fn texts(keys: &[Key]) -> Vec<String> {
    keys.iter().map(|k| k.text.clone()).collect()
}

/// Prints the outcome (one line per key, or one JSON object) and returns the exit code.
pub fn report(opts: &AcquireOpts, outcome: &SetOutcome) -> i32 {
    let owner = &opts.ident.owner;
    match outcome {
        SetOutcome::Granted { keys, waited_ms } => {
            if opts.json {
                let mut out = json!({
                    "result": "granted", "keys": texts(keys), "owner": owner, "waited_ms": waited_ms
                });
                if keys.len() == 1 {
                    out["key"] = json!(keys[0].text);
                }
                print_json(&out);
            } else {
                for k in keys {
                    println!("granted {}", k.text);
                }
            }
            EXIT_OK
        }
        SetOutcome::Failed {
            key,
            outcome,
            released,
        } => {
            let (result, word, code) = match outcome {
                Outcome::Busy { .. } => ("queued", "busy", EXIT_BUSY),
                Outcome::Timeout { .. } => ("timeout", "timeout", EXIT_TIMEOUT),
                Outcome::Interrupted => ("error", "interrupted", EXIT_INTERRUPTED),
                Outcome::Deadlock { .. } => ("deadlock", "deadlock", EXIT_DEADLOCK),
                Outcome::Granted { .. } => unreachable!("a granted key is not a failure"),
            };
            let mut out = json!({
                "result": result, "key": key.text, "keys": texts(&opts_keys(opts)),
                "owner": owner, "failed": key.text, "released": texts(released),
            });
            match outcome {
                Outcome::Busy {
                    holder,
                    position,
                    via,
                }
                | Outcome::Timeout {
                    holder,
                    position,
                    via,
                } => {
                    out["holder"] = json!(holder);
                    out["position"] = json!(position);
                    if let Some(via) = via {
                        out["via"] = json!(via);
                    }
                    if !opts.json {
                        let by = holder
                            .as_ref()
                            .map_or("nobody (queue ahead)", |h| h.owner.as_str());
                        let through = via.as_ref().map_or(String::new(), |v| format!(" via {v}"));
                        println!(
                            "{word} {} held by {by}{through} (position {position})",
                            key.text
                        );
                    }
                }
                Outcome::Deadlock { chain } => {
                    out["chain"] = json!(chain);
                    if !opts.json {
                        println!("{word} {}: {chain}", key.text);
                    }
                }
                _ => {
                    out["error"] = json!("interrupted");
                    if !opts.json {
                        println!("{word} {}", key.text);
                    }
                }
            }
            if opts.json {
                print_json(&out);
            } else {
                for k in released {
                    println!("released {}", k.text);
                }
            }
            code
        }
    }
}

fn opts_keys(opts: &AcquireOpts) -> Vec<Key> {
    canonical(&normalize_keys(&opts.keys).unwrap_or_default())
}

pub fn run(args: Vec<String>) -> CmdResult {
    let (opts, rest) = parse(args, "acquire")?;
    if !rest.is_empty() {
        return Err(usage("acquire: unexpected '--' (did you mean jlock run?)"));
    }
    wait::install_interrupt_handler();
    let keys = normalize_keys(&opts.keys)?;
    let store = Store::open()?;
    let deadline = opts.timeout.map(|t| Instant::now() + t);
    let outcome = acquire_all(&store, &keys, &opts.ident, opts.wait, deadline)
        .map_err(|e| fs_error("cannot lock", &e))?;
    Ok(report(&opts, &outcome))
}
