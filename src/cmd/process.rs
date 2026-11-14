//! `jlock process inspect --pid <pid> [--identity <token>]`: present, absent or unknown, with the
//! identity token of the incarnation. Read only: it never stops anything.

use crate::cli::{print_json, unknown_option, usage, Arg, CmdResult, Parser, EXIT_OK};
use crate::i18n::txt;

pub fn run(args: Vec<String>) -> CmdResult {
    let mut p = Parser::new(args);
    let mut json = false;
    let mut action: Option<String> = None;
    let mut pid: Option<String> = None;
    let mut identity: Option<String> = None;
    while let Some(arg) = p.next() {
        match arg {
            Arg::Flag(f) if f == "json" => json = true,
            Arg::Flag(f) if f == "pid" => pid = Some(p.value("pid")?),
            Arg::Flag(f) if f == "identity" => {
                let v = p.value("identity")?;
                if v.trim().is_empty() {
                    return Err(usage(txt!("error.empty_value", flag = "identity")));
                }
                identity = Some(v);
            }
            Arg::Flag(f) => return Err(unknown_option("process", &f)),
            Arg::Pos(a) if action.is_none() => action = Some(a),
            Arg::Pos(a) => {
                return Err(usage(txt!("error.process_extra", arg = a)));
            }
            Arg::Rest(_) => return Err(crate::cli::unexpected_rest("process")),
        }
    }
    match action.as_deref() {
        Some("inspect") => {}
        Some(other) => return Err(usage(txt!("error.process_action", action = other))),
        None => return Err(usage(txt!("error.process_needs_action"))),
    }
    let Some(pid) = pid else {
        return Err(usage(txt!("error.process_needs_pid")));
    };
    let pid: u32 = match pid.trim().parse() {
        Ok(n) if n > 0 => n,
        _ => return Err(usage(txt!("error.bad_pid", value = pid))),
    };
    let found = crate::process_identity::inspect(pid, identity.as_deref());
    if json {
        print_json(&found.to_json());
        return Ok(EXIT_OK);
    }
    println!("state: {}", found.state());
    if let Some(id) = found.identity() {
        println!("identity: {id}");
    }
    if let Some(r) = found.reason() {
        println!("reason: {r}");
    }
    Ok(EXIT_OK)
}
