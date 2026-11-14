//! `jlock capabilities`: what this binary can do, as a stable answer for integrators. The numbers
//! go up when a contract changes incompatibly; 0 means "not available here".

use crate::cli::{print_json, usage, Arg, CmdResult, Parser, EXIT_OK};
use crate::i18n::txt;
use serde_json::{json, Value};

/// The custody contract (`custody`, version 1) and process inspection (`processInspect`, version 1)
/// exist on Windows only; elsewhere both are 0 and the custody commands refuse.
pub fn answer() -> Value {
    answer_for(crate::custody::SUPPORTED && crate::process_identity::SUPPORTED)
}

/// The answer on a platform with (`true`, Windows) or without custody and process inspection.
/// Kept apart from `cfg!` so both answers are tested on every platform.
pub fn answer_for(windows: bool) -> Value {
    let (custody, inspect, scope) = if windows {
        (1, 1, "job")
    } else {
        (0, 0, "none")
    };
    json!({
        "version": env!("CARGO_PKG_VERSION"),
        "capabilities": { "custody": custody, "processInspect": inspect },
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_answer_announces_custody_in_a_job() {
        let a = answer_for(true);
        assert_eq!(a["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(a["capabilities"]["custody"], 1);
        assert_eq!(a["capabilities"]["processInspect"], 1);
        assert_eq!(a["custodyScope"], "job");
    }

    #[test]
    fn answer_elsewhere_announces_no_custody() {
        let a = answer_for(false);
        assert_eq!(a["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(a["capabilities"]["custody"], 0);
        assert_eq!(a["capabilities"]["processInspect"], 0);
        assert_eq!(a["custodyScope"], "none");
    }

    #[test]
    fn this_build_answers_for_its_platform() {
        assert_eq!(answer(), answer_for(cfg!(windows)));
    }
}
