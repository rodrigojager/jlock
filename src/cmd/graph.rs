//! `jlock graph`: who waits for whom, and the cycles.

use crate::cli::{fs_error, print_json, usage, Arg, CmdResult, Parser, EXIT_OK};
use crate::graph::{chain_text, cycles, edges, Via};
use crate::index;
use crate::store::Store;
use serde_json::json;

pub fn run(args: Vec<String>) -> CmdResult {
    let mut p = Parser::new(args);
    let mut json = false;
    while let Some(arg) = p.next() {
        match arg {
            Arg::Flag(f) if f == "json" => json = true,
            Arg::Flag(f) => return Err(usage(format!("graph: unknown option --{f}"))),
            _ => return Err(usage("graph takes no arguments")),
        }
    }
    let store = Store::open()?;
    let states = index::scan(&store).map_err(|e| fs_error("cannot read the locks", &e))?;
    let all = edges(&states);
    let found = cycles(&all);
    if json {
        let chains: Vec<String> = found.iter().map(|c| chain_text(c)).collect();
        print_json(&json!({ "result": "ok", "edges": all, "cycles": chains }));
        return Ok(EXIT_OK);
    }
    if all.is_empty() {
        println!("no waiters");
        return Ok(EXIT_OK);
    }
    for e in &all {
        let how = match e.via {
            Via::Held => "held by",
            Via::Queued => "queued behind",
        };
        println!("{} -> {}  ({} {how} {})", e.from, e.to, e.key, e.to);
    }
    if found.is_empty() {
        println!("no cycles");
    } else {
        println!("cycles:");
        for c in &found {
            println!("  {}", chain_text(c));
        }
    }
    Ok(EXIT_OK)
}
