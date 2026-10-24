//! `jlock gc`: the global sweep, now, with a report of what it removed.

use crate::cli::{print_json, unknown_option, usage, Arg, CmdResult, Parser, EXIT_OK};
use crate::i18n::{t, txt};
use crate::store::Store;
use crate::sweep::sweep_all;
use serde_json::json;

pub fn run(args: Vec<String>) -> CmdResult {
    let mut p = Parser::new(args);
    let mut json = false;
    while let Some(arg) = p.next() {
        match arg {
            Arg::Flag(f) if f == "json" => json = true,
            Arg::Flag(f) => return Err(unknown_option("gc", &f)),
            _ => return Err(usage(txt!("error.takes_no_args", cmd = "gc"))),
        }
    }
    let store = Store::open()?;
    let r = sweep_all(&store);
    if json {
        let mut out = json!(r);
        out["result"] = json!("ok");
        print_json(&out);
    } else {
        let rows = [
            (t!("gc.expired_locks"), r.expired_locks.to_string()),
            (t!("gc.expired_tickets"), r.expired_tickets.to_string()),
            (t!("gc.broken_consumed"), r.broken_consumed.to_string()),
            (t!("gc.folders_removed"), r.folders_removed.to_string()),
            (t!("gc.log_bytes"), r.log_bytes.to_string()),
        ];
        let width = rows
            .iter()
            .map(|(label, _)| crate::i18n::width(label))
            .max()
            .unwrap_or(0);
        for (label, value) in &rows {
            println!("{label}:{} {value}", crate::i18n::pad(label, width));
        }
        for e in &r.errors {
            println!("{}", t!("gc.error", error = e));
        }
    }
    Ok(EXIT_OK)
}
