//! `jlock graph`: who waits for whom, and the cycles.

use crate::cli::{fs_error, print_json, unknown_option, usage, Arg, CmdResult, Parser, EXIT_OK};
use crate::graph::{chain_text, cycles, edges, Via};
use crate::i18n::{t, txt};
use crate::index;
use crate::store::Store;
use serde_json::json;

pub fn run(args: Vec<String>) -> CmdResult {
    let mut p = Parser::new(args);
    let mut json = false;
    while let Some(arg) = p.next() {
        match arg {
            Arg::Flag(f) if f == "json" => json = true,
            Arg::Flag(f) => return Err(unknown_option("graph", &f)),
            _ => return Err(usage(txt!("error.takes_no_args", cmd = "graph"))),
        }
    }
    let store = Store::open()?;
    let states = index::scan(&store).map_err(|e| fs_error(txt!("error.cannot_read"), &e))?;
    let all = edges(&states);
    let found = cycles(&all);
    if json {
        let chains: Vec<String> = found.iter().map(|c| chain_text(c)).collect();
        print_json(&json!({ "result": "ok", "edges": all, "cycles": chains }));
        return Ok(EXIT_OK);
    }
    if all.is_empty() {
        println!("{}", t!("graph.no_waiters"));
        return Ok(EXIT_OK);
    }
    for e in &all {
        let how = match e.via {
            Via::Held => "graph.edge_held",
            Via::Queued => "graph.edge_queued",
        };
        println!(
            "{}",
            t!(how, from = e.from, to = e.to, key = e.key, holder = e.to)
        );
    }
    if found.is_empty() {
        println!("{}", t!("graph.no_cycles"));
    } else {
        println!("{}", t!("graph.cycles"));
        for c in &found {
            println!("  {}", chain_text(c));
        }
    }
    Ok(EXIT_OK)
}
