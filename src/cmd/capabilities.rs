//! `jlock capabilities`: what this binary can do, as a stable answer for integrators. The numbers
//! go up when a contract changes incompatibly; 0 means "not available here".

use crate::cli::{print_json, usage, Arg, CmdResult, Parser, EXIT_OK};
use crate::i18n::txt;
use serde_json::{json, Value};

/// The custody contract version (0 until custody is wired in).
pub const CUSTODY: u32 = 0;

pub fn answer() -> Value {
    let inspect = u32::from(crate::process_identity::SUPPORTED);
    let scope = if cfg!(windows) { "job" } else { "none" };
    json!({
        "version": env!("CARGO_PKG_VERSION"),
        "capabilities": { "custody": CUSTODY, "processInspect": inspect },
        "custodyScope": scope,
    })
}

pub fn run(args: Vec<String>) -> CmdResult {
    let mut p = Parser::new(args);
    let mut json = false;
    while let Some(arg) = p.next() {
        match arg {
            Arg::Flag(f) if f == "json" => json = true,
            Arg::Flag(f) => return Err(crate::cli::unknown_option("capabilities", &f)),
            _ => return Err(usage(txt!("error.takes_no_args", cmd = "capabilities"))),
        }
    }
    let a = answer();
    if json {
        print_json(&a);
        return Ok(EXIT_OK);
    }
    println!("version: {}", a["version"].as_str().unwrap_or(""));
    println!("custody: {}", a["capabilities"]["custody"]);
    println!("processInspect: {}", a["capabilities"]["processInspect"]);
    println!("custodyScope: {}", a["custodyScope"].as_str().unwrap_or(""));
    Ok(EXIT_OK)
}
