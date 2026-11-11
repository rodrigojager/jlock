//! `jlock acquire`: a ticket in the key's queue, then a hold file (`holds/<grant>.json`) created
//! under the key's gate when nothing blocks the request (plan section 7, revised for 0.3). What
//! blocks is decided by `conflict::judge`: holds that exclude the request (key, mode and owner
//! family), and older conflicting tickets unless the request is covered by its own family.

use crate::cli::{
    fs_error, parse_duration, print_json, unknown_option, usage, Arg, CmdResult, Failure, Parser,
    EXIT_BUSY, EXIT_DEADLOCK, EXIT_OK, EXIT_TIMEOUT,
};
use crate::conflict::{self, Claim, Verdict};
use crate::expire;
use crate::fsx;
use crate::graph;
use crate::help::hint;
use crate::holder::{iso, now_micros, Holder, Ident, Mode, Parent};
use crate::i18n::{t, txt};
use crate::index;
use crate::key::{self, Key, Kind};
use crate::log;
use crate::queue::{self, Ticket};
use crate::store::{KeyDir, Store};
use crate::wait;
use serde_json::{json, Value};
use std::io;
use std::time::{Duration, Instant};

pub const EXIT_INTERRUPTED: i32 = 130;

pub struct AcquireOpts {
    pub keys: Vec<String>,
    /// `--mode`: the mode of every key without its own `@shared`/`@exclusive`.
    pub mode: Option<Mode>,
    pub ident: Ident,
    pub wait: bool,
    pub timeout: Option<Duration>,
    pub command_timeout: Option<Duration>,
    pub json: bool,
    /// `hold --detach`: answer and return, leaving a second process to keep the lock.
    pub detach: bool,
    /// `hold --release-on-eof`: the end of standard input counts as the line `release`.
    pub release_on_eof: bool,
}

/// One key asked for, in one mode.
#[derive(Clone, Debug)]
pub struct Want {
    pub key: Key,
    pub mode: Mode,
}

/// Parses the options shared by `acquire`, `hold` and `run`; returns what follows `--`.
pub fn parse(args: Vec<String>, cmd: &str) -> Result<(AcquireOpts, Vec<String>), Failure> {
    let mut p = Parser::new(args);
    let mut opts = AcquireOpts {
        keys: Vec::new(),
        mode: None,
        ident: Ident {
            owner: Ident::default_owner(),
            ..Ident::default()
        },
        wait: true,
        timeout: None,
        command_timeout: None,
        json: false,
        detach: false,
        release_on_eof: false,
    };
    let mut rest = Vec::new();
    while let Some(arg) = p.next() {
        match arg {
            Arg::Pos(k) => opts.keys.push(k),
            Arg::Rest(r) => rest = r,
            Arg::Flag(f) => match f.as_str() {
                "owner" => {
                    let owner = nonempty(p.value(&f)?, &f)?;
                    key::check_owner(&owner).map_err(|k| usage(txt!(&k, owner = owner.clone())))?;
                    opts.ident.owner = owner;
                }
                "group" => opts.ident.group = Some(nonempty(p.value(&f)?, &f)?),
                "intent" => opts.ident.intent = Some(p.value(&f)?),
                "meta" => {
                    let kv = p.value(&f)?;
                    let (k, v) = kv
                        .split_once('=')
                        .ok_or_else(|| usage(txt!("error.meta_kv", value = kv)))?;
                    opts.ident.meta.insert(k.to_string(), v.to_string());
                }
                "ttl" => {
                    let d = parse_duration(&p.value(&f)?)?;
                    opts.ident.ttl_secs = Some(d.as_secs().max(1));
                }
                "mode" => {
                    let v = p.value(&f)?;
                    opts.mode = Some(
                        Mode::parse(&v).ok_or_else(|| usage(txt!("error.bad_mode", value = v)))?,
                    );
                }
                "wait" => opts.wait = true,
                "no-wait" => opts.wait = false,
                "timeout" => opts.timeout = Some(parse_duration(&p.value(&f)?)?),
                "command-timeout" if cmd == "run" => {
                    opts.command_timeout = Some(parse_duration(&p.value(&f)?)?)
                }
                "json" => opts.json = true,
                "detach" => opts.detach = true,
                "release-on-eof" => opts.release_on_eof = true,
                _ => return Err(unknown_option(cmd, &f)),
            },
        }
    }
    if opts.keys.is_empty() {
        return Err(usage(txt!("error.needs_key", cmd = cmd)));
    }
    Ok((opts, rest))
}

fn nonempty(v: String, flag: &str) -> Result<String, Failure> {
    if v.trim().is_empty() {
        Err(usage(txt!("error.empty_value", flag = flag)))
    } else {
        Ok(v)
    }
}

/// Normalizes keys given on the command line. A mode suffix (`@shared`, `@exclusive`) is dropped:
/// commands that do not take keys (release, status...) accept it and ignore it.
pub fn normalize_keys(inputs: &[String]) -> Result<Vec<Key>, Failure> {
    let cwd = std::env::current_dir().map_err(|e| fs_error(txt!("error.cwd"), &e))?;
    let mut keys = Vec::new();
    for input in inputs {
        let (bare, _) = key::split_mode_suffix(input);
        // key.rs answers with a catalog key: it is pure string work and knows no languages.
        keys.push(key::normalize(bare, &cwd).map_err(|k| usage(txt!(&k)))?);
    }
    Ok(keys)
}

/// Checks the `resource:` keys against the catalogue; returns each key in canonical form with the
/// resource's default mode (`None` for other keys).
pub fn check_resources(keys: Vec<Key>) -> Result<Vec<(Key, Option<Mode>)>, Failure> {
    if !keys.iter().any(|k| k.kind == Kind::Resource) {
        return Ok(keys.into_iter().map(|k| (k, None)).collect());
    }
    let catalogue = crate::resources::load();
    catalogue.warn();
    keys.into_iter()
        .map(|k| {
            if k.kind == Kind::Resource {
                catalogue
                    .check(&k)
                    .map(|(k, m)| (k, Some(m)))
                    .map_err(usage)
            } else {
                Ok((k, None))
            }
        })
        .collect()
}

/// The keys of a request with their modes: the key's own suffix, else `--mode`, else the
/// resource's default, else exclusive. A key named twice is taken once, in the stronger mode.
pub fn wants(opts: &AcquireOpts) -> Result<Vec<Want>, Failure> {
    let keys = check_resources(normalize_keys(&opts.keys)?)?;
    let mut out: Vec<Want> = Vec::new();
    for (input, (key, default)) in opts.keys.iter().zip(keys) {
        let suffix = key::split_mode_suffix(input).1.and_then(Mode::parse);
        let mode = suffix.or(opts.mode).or(default).unwrap_or_default();
        match out.iter_mut().find(|w| w.key.text == key.text) {
            Some(w) => w.mode = w.mode.max(mode),
            None => out.push(Want { key, mode }),
        }
    }
    out.sort_by(|a, b| a.key.text.cmp(&b.key.text));
    Ok(out)
}

pub enum Outcome {
    Granted {
        waited_ms: u64,
        /// The grant created; `None` when the owner already held the key in that mode or stronger.
        grant: Option<String>,
        /// The ancestor's hold this one is nested in.
        parent: Option<Parent>,
    },
    /// Not taken with `--no-wait`. `via` names the conflicting key when the block comes from the
    /// hierarchy (or `*`) rather than from the key itself.
    Busy(Blocked),
    Timeout(Blocked),
    Interrupted,
    /// Waiting would close a cycle in "who waits for whom".
    Deadlock {
        chain: String,
    },
}

/// Who is in the way of a request that was not granted.
pub struct Blocked {
    /// The hold in the way (on the key itself if there is one), `None` when only the queue is.
    pub holder: Option<Holder>,
    /// Every hold in the way.
    pub holders: Vec<Holder>,
    pub position: usize,
    pub via: Option<String>,
}

fn blocked(v: &Verdict, key: &Key, position: usize) -> Blocked {
    let first = v
        .blockers
        .iter()
        .find(|c| c.key.text == key.text)
        .or_else(|| v.blockers.first());
    let via = match first {
        Some(c) => (c.key.text != key.text).then(|| c.key.text.clone()),
        None => v
            .ahead
            .first()
            .filter(|c| c.key.text != key.text)
            .map(|c| c.key.text.clone()),
    };
    Blocked {
        holder: first.map(|c| c.body.clone()),
        holders: v.blockers.iter().map(|c| c.body.clone()).collect(),
        position,
        via,
    }
}

/// Would `owner` waiting for `want` (with a ticket younger than all others) close a cycle?
fn deadlock_before_queueing(store: &Store, want: &Want, owner: &str) -> io::Result<Option<String>> {
    let states = index::scan(store)?;
    let all = graph::edges(&states);
    let proposed = graph::edges_for(owner, &want.key, want.mode, None, &states);
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
    verdict: Verdict,
    /// My place in the queue of the key, 1-based.
    position: usize,
}

/// Reads the key and every conflicting key (expiring the dead on the way) and judges the request.
fn look(
    store: &Store,
    kd: &KeyDir,
    want: &Want,
    owner: &str,
    mine: Option<&str>,
) -> io::Result<View> {
    let own = index::state(kd.clone())?;
    let position = own
        .tickets
        .iter()
        .filter(|t| mine.is_none_or(|m| t.name.as_str() < m))
        .count()
        + 1;
    let mut holds: Vec<Claim> = own.hold_claims().collect();
    let mut tickets: Vec<Claim> = own.ticket_claims().collect();
    for cd in conflict::conflicting_dirs(store, &kd.key)? {
        expire::expire_key(&cd, None)?;
        let st = index::state(cd)?;
        holds.extend(st.hold_claims());
        tickets.extend(st.ticket_claims());
    }
    Ok(View {
        verdict: conflict::judge(&kd.key, want.mode, owner, mine, &holds, &tickets),
        position,
    })
}

/// Creates the hold file under the key's gate, unless a hold of the key excludes it. Returns the
/// grant, or `None` when the key was taken in between.
fn take(kd: &KeyDir, body: &Holder) -> io::Result<Option<String>> {
    let bytes = body.to_bytes();
    let _gate = kd.gate()?;
    let holds = kd.read_holds()?;
    if holds
        .iter()
        .any(|h| conflict::excludes(&kd.key, body.mode, &body.owner, &kd.key, &h.body))
    {
        return Ok(None);
    }
    loop {
        let grant = queue::new_name();
        match fsx::create_exclusive(&kd.hold_path(&grant), &bytes) {
            Ok(()) => {
                kd.ensure_key_file()?;
                return Ok(Some(grant));
            }
            Err(e) if fsx::is_exists(&e) => continue,
            Err(e) if fsx::is_not_found(&e) => kd.ensure()?,
            Err(e) => return Err(e),
        }
    }
}

/// After creating the hold, makes sure no conflicting key was taken at the same instant by someone
/// who could not see us yet, and that the hold we are nested in is still there. If either fails,
/// ours is withdrawn and the caller goes on waiting; the other side, seeing ours, does the same, or
/// had been granted before us. Returns the clash (`Some(None)`: the parent hold ended).
fn confirm(
    store: &Store,
    kd: &KeyDir,
    grant: &str,
    body: &Holder,
) -> io::Result<Option<Option<Claim>>> {
    let mut clash: Option<Option<Claim>> = None;
    if body.parent.is_some() && expire::parent_gone(kd, body) {
        clash = Some(None);
    }
    if clash.is_none() {
        let mut dirs = vec![kd.clone()];
        dirs.extend(conflict::conflicting_dirs(store, &kd.key)?);
        'outer: for d in dirs {
            for h in d.read_holds()? {
                if h.grant != grant
                    && conflict::excludes(&kd.key, body.mode, &body.owner, &d.key, &h.body)
                {
                    clash = Some(Some(Claim {
                        key: d.key.clone(),
                        id: h.grant,
                        body: h.body,
                    }));
                    break 'outer;
                }
            }
        }
    }
    if clash.is_some() {
        wait::debug(format!("withdrawing {} ({grant})", kd.key.text));
        fsx::remove_with_retry(&kd.hold_path(grant))?;
    }
    Ok(clash)
}

/// The hold body as written at the grant: `since` is the moment the lock was taken, `parent` the
/// ancestor's hold it is nested in.
fn granted_now(body: &Holder, parent: Option<Parent>) -> Holder {
    let mut b = body.clone();
    b.since = iso(now_micros());
    b.parent = parent;
    b
}

/// The owner already holds the key in this mode or a stronger one.
fn already(kd: &KeyDir, want: &Want, owner: &str) -> io::Result<bool> {
    Ok(kd.holds_of(owner)?.iter().any(|h| h.body.mode >= want.mode))
}

/// One attempt without waiting: granted, or busy with who is in the way.
fn try_now(store: &Store, kd: &KeyDir, want: &Want, body: &Holder) -> io::Result<Outcome> {
    let owner = body.owner.as_str();
    expire::expire_key(kd, None)?;
    for _ in 0..3 {
        let view = look(store, kd, want, owner, None)?;
        if !view.verdict.free() {
            let _ = kd.cleanup();
            return Ok(Outcome::Busy(blocked(
                &view.verdict,
                &kd.key,
                view.position,
            )));
        }
        let parent = view.verdict.parent.clone();
        let mine = granted_now(body, parent.clone());
        let Some(grant) = take(kd, &mine)? else {
            continue; // taken in between: look again to say by whom
        };
        match confirm(store, kd, &grant, &mine)? {
            None => {
                return Ok(Outcome::Granted {
                    waited_ms: 0,
                    grant: Some(grant),
                    parent,
                })
            }
            Some(Some(clash)) => {
                let _ = kd.cleanup();
                let v = Verdict {
                    blockers: vec![clash],
                    ..Verdict::default()
                };
                return Ok(Outcome::Busy(blocked(&v, &kd.key, view.position)));
            }
            Some(None) => continue, // the parent hold ended: judge again
        }
    }
    let view = look(store, kd, want, owner, None)?;
    let _ = kd.cleanup();
    Ok(Outcome::Busy(blocked(
        &view.verdict,
        &kd.key,
        view.position,
    )))
}

/// Takes one key for `ident`: at once or never with `wait == false`; otherwise in arrival order,
/// up to `deadline`. Conflicting keys (a folder and what is inside it, `*`) are waited for too.
pub fn acquire_one(
    store: &Store,
    want: &Want,
    ident: &Ident,
    wait_turn: bool,
    deadline: Option<Instant>,
) -> io::Result<Outcome> {
    let outcome = acquire_inner(store, want, ident, wait_turn, deadline)?;
    let mut fields = log::about(&want.key.text, &ident.owner, ident.group.as_deref());
    fields["mode"] = json!(want.mode);
    let event = match &outcome {
        Outcome::Granted {
            waited_ms,
            grant: Some(grant),
            parent,
        } => {
            fields["waited_ms"] = json!(waited_ms);
            fields["grant"] = json!(grant);
            fields["parent"] = json!(parent);
            "acquire"
        }
        Outcome::Timeout(b) => {
            fields["reason"] = json!(b.holder.as_ref().map(|h| format!("held by {}", h.owner)));
            "timeout"
        }
        Outcome::Deadlock { chain } => {
            fields["reason"] = json!(chain);
            "deadlock"
        }
        Outcome::Granted { grant: None, .. } | Outcome::Busy(_) | Outcome::Interrupted => {
            return Ok(outcome)
        }
    };
    log::event(event, fields);
    Ok(outcome)
}

fn acquire_inner(
    store: &Store,
    want: &Want,
    ident: &Ident,
    wait_turn: bool,
    deadline: Option<Instant>,
) -> io::Result<Outcome> {
    let start = Instant::now();
    let kd = store.key_dir(&want.key);
    let body = Holder::new(&want.key.text, ident, want.mode);
    let owner = ident.owner.as_str();

    if already(&kd, want, owner)? {
        return Ok(Outcome::Granted {
            waited_ms: 0,
            grant: None,
            parent: None,
        });
    }

    // Free, nobody in line, no conflicting key taken: take it now, without a ticket. Otherwise
    // `--no-wait` answers busy, and a waiter gets in line.
    let now = try_now(store, &kd, want, &body)?;
    if !wait_turn || matches!(now, Outcome::Granted { .. }) {
        return Ok(now);
    }

    if let Some(chain) = deadlock_before_queueing(store, want, owner)? {
        return Ok(Outcome::Deadlock { chain });
    }
    let mut ticket = queue::create(&kd, &body)?;
    wait::debug(format!("queued {} as {}", want.key.text, ticket.name));
    let mut fields = log::about(&want.key.text, owner, ident.group.as_deref());
    fields["mode"] = json!(want.mode);
    log::event("queued", fields);
    let result = wait_turn_loop(store, &kd, want, &body, &mut ticket, start, deadline);
    if result.is_err() {
        let _ = queue::remove(&ticket);
    }
    result
}

fn wait_turn_loop(
    store: &Store,
    kd: &KeyDir,
    want: &Want,
    body: &Holder,
    ticket: &mut Ticket,
    start: Instant,
    deadline: Option<Instant>,
) -> io::Result<Outcome> {
    let poll = wait::poll_ms();
    let owner = body.owner.as_str();
    let mut polls: u64 = 0;
    let granted = |ticket: &Ticket, grant: Option<String>, parent: Option<Parent>| {
        queue::remove(ticket)?;
        Ok(Outcome::Granted {
            waited_ms: start.elapsed().as_millis() as u64,
            grant,
            parent,
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
        if already(kd, want, owner)? {
            return granted(ticket, None, None);
        }
        let view = look(store, kd, want, owner, Some(&ticket.name))?;
        if view.verdict.free() {
            let parent = view.verdict.parent.clone();
            let mine = granted_now(body, parent.clone());
            match take(kd, &mine)? {
                Some(grant) => {
                    if confirm(store, kd, &grant, &mine)?.is_none() {
                        return granted(ticket, Some(grant), parent);
                    }
                }
                None => {
                    let mut fields = log::about(&kd.key.text, owner, body.group.as_deref());
                    fields["reason"] = json!("taken by someone else at the same instant");
                    log::event("queued", fields);
                }
            }
        }
        if deadline.is_some_and(|d| Instant::now() >= d) {
            queue::remove(ticket)?;
            return Ok(Outcome::Timeout(blocked(
                &view.verdict,
                &kd.key,
                view.position,
            )));
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

/// One key of a granted set.
pub struct Granted {
    pub want: Want,
    pub grant: Option<String>,
    pub parent: Option<Parent>,
}

#[allow(clippy::large_enum_variant)] // built once per command
pub enum SetOutcome {
    Granted {
        keys: Vec<Granted>,
        waited_ms: u64,
    },
    /// `key` was not obtained; the keys taken before it were released again.
    Failed {
        key: Key,
        outcome: Outcome,
        released: Vec<Key>,
    },
}

impl SetOutcome {
    pub fn keys(&self) -> Vec<Key> {
        match self {
            SetOutcome::Granted { keys, .. } => keys.iter().map(|g| g.want.key.clone()).collect(),
            SetOutcome::Failed { .. } => Vec::new(),
        }
    }
}

/// Takes all keys in canonical order (sorted by normalized text), or none: on the first failure
/// the grants already made by this call are released.
pub fn acquire_all(
    store: &Store,
    wants: &[Want],
    ident: &Ident,
    wait_turn: bool,
    deadline: Option<Instant>,
) -> io::Result<SetOutcome> {
    let start = Instant::now();
    let mut taken: Vec<(Key, String)> = Vec::new();
    let undo = |taken: &[(Key, String)]| {
        for (k, grant) in taken {
            let _ = crate::cmd::release::release_grant(&store.key_dir(k), grant, &ident.owner);
        }
    };
    let mut granted = Vec::new();
    for want in wants {
        match acquire_one(store, want, ident, wait_turn, deadline) {
            Ok(Outcome::Granted { grant, parent, .. }) => {
                if let Some(g) = &grant {
                    taken.push((want.key.clone(), g.clone()));
                }
                granted.push(Granted {
                    want: want.clone(),
                    grant,
                    parent,
                });
            }
            Ok(outcome) => {
                undo(&taken);
                return Ok(SetOutcome::Failed {
                    key: want.key.clone(),
                    outcome,
                    released: taken.into_iter().map(|(k, _)| k).collect(),
                });
            }
            Err(e) => {
                undo(&taken);
                return Err(e);
            }
        }
    }
    Ok(SetOutcome::Granted {
        keys: granted,
        waited_ms: start.elapsed().as_millis() as u64,
    })
}

fn texts(keys: &[Key]) -> Vec<String> {
    keys.iter().map(|k| k.text.clone()).collect()
}

/// `{key, mode, grant, nested, parent}` for each granted key.
fn locks_json(keys: &[Granted]) -> Value {
    Value::Array(
        keys.iter()
            .map(|g| {
                json!({
                    "key": g.want.key.text, "mode": g.want.mode, "grant": g.grant,
                    "nested": g.parent.is_some(), "parent": g.parent,
                })
            })
            .collect(),
    )
}

/// Prints the outcome (one line per key, or one JSON object) and returns the exit code.
pub fn report(opts: &AcquireOpts, wants: &[Want], outcome: &SetOutcome) -> i32 {
    let owner = &opts.ident.owner;
    match outcome {
        SetOutcome::Granted { keys, waited_ms } => {
            if opts.json {
                let list: Vec<String> = keys.iter().map(|g| g.want.key.text.clone()).collect();
                let mut out = json!({
                    "result": "granted", "keys": list, "owner": owner, "waited_ms": waited_ms,
                    "locks": locks_json(keys),
                });
                if keys.len() == 1 {
                    out["key"] = json!(keys[0].want.key.text);
                    out["mode"] = json!(keys[0].want.mode);
                }
                print_json(&out);
            } else {
                for g in keys {
                    let mut line = t!("acquire.granted", key = g.want.key.text);
                    if g.want.mode == Mode::Shared {
                        line += &t!("acquire.shared");
                    }
                    if let Some(p) = &g.parent {
                        line += &t!("acquire.nested", owner = p.owner);
                    }
                    println!("{line}");
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
                Outcome::Busy(_) => ("queued", "acquire.busy", EXIT_BUSY),
                Outcome::Timeout(_) => ("timeout", "acquire.timeout", EXIT_TIMEOUT),
                Outcome::Interrupted => ("error", "acquire.interrupted", EXIT_INTERRUPTED),
                Outcome::Deadlock { .. } => ("deadlock", "acquire.deadlock", EXIT_DEADLOCK),
                Outcome::Granted { .. } => unreachable!("a granted key is not a failure"),
            };
            let mode = wants
                .iter()
                .find(|w| w.key.text == key.text)
                .map_or(Mode::Exclusive, |w| w.mode);
            let keys: Vec<Key> = wants.iter().map(|w| w.key.clone()).collect();
            let mut out = json!({
                "result": result, "key": key.text, "keys": texts(&keys), "mode": mode,
                "owner": owner, "failed": key.text, "released": texts(released),
            });
            match outcome {
                Outcome::Busy(b) | Outcome::Timeout(b) => {
                    out["holder"] = json!(b.holder);
                    out["holders"] = json!(b.holders);
                    out["position"] = json!(b.position);
                    if let Some(via) = &b.via {
                        out["via"] = json!(via);
                    }
                    if !opts.json {
                        let by = b.holder.as_ref().map_or_else(
                            || t!("acquire.nobody"),
                            |h| {
                                let mut who = h.owner.clone();
                                if h.mode == Mode::Shared {
                                    who += &t!("acquire.shared");
                                }
                                if b.holders.len() > 1 {
                                    who += &crate::i18n::trn(
                                        "acquire.and_others",
                                        (b.holders.len() - 1) as u64,
                                        &[],
                                    );
                                }
                                who
                            },
                        );
                        let through = b
                            .via
                            .as_ref()
                            .map_or(String::new(), |v| t!("acquire.via", key = v));
                        println!(
                            "{}",
                            t!(
                                word,
                                key = key.text,
                                owner = by,
                                via = through,
                                position = b.position
                            )
                        );
                        let tip = if code == EXIT_BUSY {
                            "tip.busy"
                        } else {
                            "tip.timeout"
                        };
                        hint(false, t!(tip, key = key.text));
                    }
                }
                Outcome::Deadlock { chain } => {
                    out["chain"] = json!(chain);
                    if !opts.json {
                        println!("{}", t!(word, key = key.text, chain = chain));
                        hint(false, t!("tip.deadlock"));
                    }
                }
                _ => {
                    out["error"] = json!("interrupted");
                    if !opts.json {
                        println!("{}", t!(word, key = key.text));
                    }
                }
            }
            if opts.json {
                print_json(&out);
            } else {
                for k in released {
                    println!("{}", t!("result.released", key = k.text));
                }
            }
            code
        }
    }
}

pub fn run(args: Vec<String>) -> CmdResult {
    let (opts, rest) = parse(args, "acquire")?;
    if !rest.is_empty() {
        return Err(usage(txt!("error.rest_not_run", cmd = "acquire")));
    }
    wait::install_interrupt_handler();
    let wants = wants(&opts)?;
    let store = Store::open()?;
    let deadline = opts.timeout.map(|t| Instant::now() + t);
    let outcome = acquire_all(&store, &wants, &opts.ident, opts.wait, deadline)
        .map_err(|e| fs_error(txt!("error.cannot_lock"), &e))?;
    Ok(report(&opts, &wants, &outcome))
}
