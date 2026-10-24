//! `jlock list`: everything locked on this machine, read from the folder (never the index).

use crate::cli::{fs_error, print_json, usage, Arg, CmdResult, Parser, EXIT_OK};
use crate::cmd::acquire::normalize_keys;
use crate::cmd::status::{heartbeat_ms, human, liveness, ms_since};
use crate::conflict::is_ancestor;
use crate::heartbeat::stale_after;
use crate::holder::now_micros;
use crate::index::{self, KeyState};
use crate::key::Kind;
use crate::store::Store;
use crate::wait;
use serde_json::{json, Value};
use std::io::Write;
use std::thread::sleep;
use std::time::Duration;

#[derive(Default)]
struct Filters {
    owner: Option<String>,
    group: Option<String>,
    under: Option<String>,
    stale: bool,
}

fn row(st: &KeyState) -> Value {
    let stale_ms = stale_after().as_millis() as u64;
    let holder = st.holder.as_ref().map(|h| {
        let hb = heartbeat_ms(&st.dir.holder_path());
        let (dead, late) = liveness(&st.dir.holder_path(), h);
        json!({
            "owner": h.owner, "group": h.group, "intent": h.intent, "meta": h.meta,
            "pid": h.pid, "host": h.host, "cwd": h.cwd, "since": h.since, "ttl_secs": h.ttl_secs,
            "held_ms": ms_since(&h.since), "heartbeat_ms": hb, "stale": dead, "late": late,
        })
    });
    let queue: Vec<Value> = st
        .tickets
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let hb = heartbeat_ms(&t.path);
            json!({
                "position": i + 1, "owner": t.body.owner, "group": t.body.group,
                "since": t.body.since, "waiting_ms": ms_since(&t.body.since),
                "heartbeat_ms": hb, "stale": hb.is_some_and(|ms| ms > stale_ms),
                "ticket": t.name,
            })
        })
        .collect();
    json!({ "key": st.dir.key.text, "holder": holder, "queue": queue })
}

fn keep(row: &Value, f: &Filters) -> bool {
    let holder = &row["holder"];
    let queue = row["queue"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    let field_matches = |field: &str, want: &str| {
        holder[field].as_str() == Some(want)
            || queue.iter().any(|t| t[field].as_str() == Some(want))
    };
    if f.owner.as_ref().is_some_and(|o| !field_matches("owner", o)) {
        return false;
    }
    if f.group.as_ref().is_some_and(|g| !field_matches("group", g)) {
        return false;
    }
    if let Some(under) = &f.under {
        let key = row["key"].as_str().unwrap_or("");
        if key != under && !is_ancestor(under, key) {
            return false;
        }
    }
    if f.stale {
        let dead =
            holder["stale"] == json!(true) || queue.iter().any(|t| t["stale"] == json!(true));
        if !dead {
            return false;
        }
    }
    true
}

fn rows(store: &Store, f: &Filters) -> std::io::Result<Vec<Value>> {
    Ok(index::scan(store)?
        .iter()
        .filter(|st| st.holder.is_some() || !st.tickets.is_empty())
        .map(row)
        .filter(|r| keep(r, f))
        .collect())
}

fn table(rows: &[Value]) -> String {
    if rows.is_empty() {
        return "nothing locked\n".into();
    }
    let ms = |v: &Value| v.as_u64().map_or("-".to_string(), human);
    let text = |v: &Value| v.as_str().unwrap_or("-").to_string();
    let mut lines = vec![[
        "KEY",
        "OWNER",
        "GROUP",
        "INTENT",
        "HELD",
        "HEARTBEAT",
        "QUEUE",
    ]
    .map(String::from)];
    for r in rows {
        let h = &r["holder"];
        let mut beat = ms(&h["heartbeat_ms"]);
        if h["stale"] == json!(true) {
            beat += " (dead)";
        } else if h["late"] == json!(true) {
            beat += " (late)";
        }
        lines.push([
            text(&r["key"]),
            text(&h["owner"]),
            text(&h["group"]),
            text(&h["intent"]),
            ms(&h["held_ms"]),
            beat,
            r["queue"].as_array().map_or(0, Vec::len).to_string(),
        ]);
    }
    let widths: Vec<usize> = (0..7)
        .map(|c| {
            lines
                .iter()
                .map(|l| l[c].chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect();
    let mut out = String::new();
    for l in &lines {
        let cells: Vec<String> = l
            .iter()
            .zip(&widths)
            .map(|(cell, w)| format!("{cell:<w$}"))
            .collect();
        out += cells.join("  ").trim_end();
        out += "\n";
    }
    out
}

fn enable_ansi() {
    #[cfg(windows)]
    // SAFETY: plain console calls on our own stdout handle; failures are ignored.
    unsafe {
        use windows_sys::Win32::System::Console::{
            GetConsoleMode, GetStdHandle, SetConsoleMode, ENABLE_VIRTUAL_TERMINAL_PROCESSING,
            STD_OUTPUT_HANDLE,
        };
        let out = GetStdHandle(STD_OUTPUT_HANDLE);
        let mut mode = 0;
        if GetConsoleMode(out, &mut mode) != 0 {
            SetConsoleMode(out, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING);
        }
    }
}

pub fn run(args: Vec<String>) -> CmdResult {
    let mut p = Parser::new(args);
    let mut f = Filters::default();
    let (mut json, mut watch) = (false, false);
    let mut under = None;
    while let Some(arg) = p.next() {
        match arg {
            Arg::Flag(flag) => match flag.as_str() {
                "owner" => f.owner = Some(p.value(&flag)?),
                "group" => f.group = Some(p.value(&flag)?),
                "under" => under = Some(p.value(&flag)?),
                "stale" => f.stale = true,
                "json" => json = true,
                "watch" => watch = true,
                _ => return Err(usage(format!("list: unknown option --{flag}"))),
            },
            Arg::Pos(x) => return Err(usage(format!("list takes no keys (got '{x}')"))),
            Arg::Rest(_) => return Err(usage("list: unexpected '--'")),
        }
    }
    if let Some(u) = under {
        let key = normalize_keys(&[u])?.remove(0);
        if key.kind != Kind::Path {
            return Err(usage("--under wants a path"));
        }
        f.under = Some(key.text);
    }
    let store = Store::open()?;
    let failed = |e: std::io::Error| fs_error("cannot read the locks", &e);
    if !watch {
        let rows = rows(&store, &f).map_err(failed)?;
        if json {
            print_json(&json!({ "result": "ok", "locks": rows }));
        } else {
            print!("{}", table(&rows));
        }
        return Ok(EXIT_OK);
    }
    wait::install_interrupt_handler();
    enable_ansi();
    while !wait::interrupted() {
        let rows = rows(&store, &f).map_err(failed)?;
        if json {
            print_json(&json!({ "result": "ok", "locks": rows }));
        } else {
            print!("\x1b[2J\x1b[H{}", table(&rows));
            println!(
                "\n{}  (every 1s, CTRL+C to stop)",
                crate::holder::iso(now_micros())
            );
        }
        let _ = std::io::stdout().flush();
        for _ in 0..50 {
            if wait::interrupted() {
                break;
            }
            sleep(Duration::from_millis(20));
        }
    }
    Ok(EXIT_OK)
}
