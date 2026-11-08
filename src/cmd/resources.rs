//! `jlock resources`: the catalogue of `resource:` keys, built-in and user-defined, with their
//! descriptions, default modes and arguments.

use crate::cli::{print_json, unknown_option, usage, Arg, CmdResult, Parser, EXIT_OK};
use crate::i18n::{pad, t, txt, width};
use serde_json::json;

pub fn run(args: Vec<String>) -> CmdResult {
    let mut p = Parser::new(args);
    let mut json = false;
    while let Some(arg) = p.next() {
        match arg {
            Arg::Flag(f) if f == "json" => json = true,
            Arg::Flag(f) => return Err(unknown_option("resources", &f)),
            _ => return Err(usage(txt!("error.takes_no_args", cmd = "resources"))),
        }
    }
    let catalogue = crate::resources::load();
    catalogue.warn();
    let file = catalogue.file.as_ref().map(|f| f.display().to_string());
    if json {
        let list: Vec<_> = catalogue
            .resources
            .iter()
            .map(|r| {
                json!({
                    "name": r.name, "key": format!("resource:{}", r.label()), "arg": r.arg,
                    "mode": r.mode, "source": if r.builtin { "builtin" } else { "user" },
                    "description": r.describe(),
                })
            })
            .collect();
        print_json(&json!({ "result": "ok", "file": file, "resources": list,
                            "warnings": catalogue.warnings }));
        return Ok(EXIT_OK);
    }
    let col = catalogue
        .resources
        .iter()
        .map(|r| width(&r.label()))
        .max()
        .unwrap_or(0);
    let line = |r: &crate::resources::Resource| {
        let label = r.label();
        format!(
            "  {label}{}  {:<9}  {}\n",
            pad(&label, col),
            r.mode.as_str(),
            r.describe()
        )
    };
    let mut out = format!("{}\n", t!("resources.builtin"));
    for r in catalogue.resources.iter().filter(|r| r.builtin) {
        out += &line(r);
    }
    let user: Vec<_> = catalogue.resources.iter().filter(|r| !r.builtin).collect();
    let path = file.unwrap_or_else(|| "-".into());
    out += "\n";
    if user.is_empty() {
        out += &format!("{}\n", t!("resources.no_user", path = path));
    } else {
        out += &format!("{}\n", t!("resources.user", path = path));
        for r in user {
            out += &line(r);
        }
    }
    out += "\n";
    out += &crate::help::wrap(&t!("resources.how"), "", "", crate::help::COLUMNS);
    print!("{out}");
    Ok(EXIT_OK)
}
