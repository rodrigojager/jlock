//! `jlock gc`: the global sweep, now, with a report of what it removed.

use crate::cli::{print_json, usage, Arg, CmdResult, Parser, EXIT_OK};
use crate::store::Store;
use crate::sweep::sweep_all;
use serde_json::json;

pub fn run(args: Vec<String>) -> CmdResult {
    let mut p = Parser::new(args);
    let mut json = false;
    while let Some(arg) = p.next() {
        match arg {
            Arg::Flag(f) if f == "json" => json = true,
            Arg::Flag(f) => return Err(usage(format!("gc: unknown option --{f}"))),
            _ => return Err(usage("gc takes no arguments")),
        }
    }
    let store = Store::open()?;
    let r = sweep_all(&store);
    if json {
        let mut out = json!(r);
        out["result"] = json!("ok");
        print_json(&out);
    } else {
        println!("expired locks:   {}", r.expired_locks);
        println!("expired tickets: {}", r.expired_tickets);
        println!("broken consumed: {}", r.broken_consumed);
        println!("folders removed: {}", r.folders_removed);
        println!("log bytes:       {}", r.log_bytes);
        for e in &r.errors {
            println!("error: {e}");
        }
    }
    Ok(EXIT_OK)
}
