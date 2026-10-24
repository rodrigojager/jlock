//! `jlock about`: who made it and where it lives. The links are defined here once; the help, the
//! `update` command and the READMEs (through a test) use these.

use crate::cli::{print_json, unknown_option, usage, Arg, CmdResult, Parser, EXIT_OK};
use crate::i18n::{t, txt};
use serde_json::json;

pub const AUTHOR: &str = "Rodrigo Jäger";
pub const GITHUB: &str = "https://github.com/rodrigojager";
pub const LINKEDIN: &str = "https://www.linkedin.com/in/rodrigojager";
pub const REPOSITORY: &str = "https://github.com/rodrigojager/jlock";
pub const LICENSE: &str = "MIT";
pub const USAGE_URL: &str = "https://github.com/rodrigojager/jlock/blob/main/docs/USAGE.md";
pub const README_URL: &str = "https://github.com/rodrigojager/jlock/blob/main/README.md";
pub const ROADMAP_URL: &str = "https://github.com/rodrigojager/jlock/blob/main/ROADMAP.md";

/// `github.com/rodrigojager` for `https://github.com/rodrigojager`.
pub fn short(url: &str) -> &str {
    let url = url.strip_prefix("https://").unwrap_or(url);
    url.strip_prefix("www.").unwrap_or(url)
}

/// The last line of `jlock --help`.
pub fn footer() -> String {
    t!(
        "about.footer",
        author = AUTHOR,
        github = short(GITHUB),
        linkedin = short(LINKEDIN)
    )
}

pub fn run(args: Vec<String>) -> CmdResult {
    let mut p = Parser::new(args);
    let mut json = false;
    while let Some(arg) = p.next() {
        match arg {
            Arg::Flag(f) if f == "json" => json = true,
            Arg::Flag(f) => return Err(unknown_option("about", &f)),
            _ => return Err(usage(txt!("error.takes_no_args", cmd = "about"))),
        }
    }
    let version = env!("CARGO_PKG_VERSION");
    if json {
        print_json(&json!({
            "result": "ok", "name": "jlock", "version": version,
            "description": env!("CARGO_PKG_DESCRIPTION"),
            "target": crate::cmd::update::TARGET,
            "author": AUTHOR, "github": GITHUB, "linkedin": LINKEDIN,
            "repository": REPOSITORY, "license": LICENSE,
            "docs": { "usage": USAGE_URL, "readme": README_URL, "roadmap": ROADMAP_URL },
        }));
        return Ok(EXIT_OK);
    }
    println!("jlock {version}");
    print!(
        "{}",
        crate::help::wrap(&t!("about.what"), "", "", crate::help::COLUMNS)
    );
    println!();
    let rows = [
        (t!("about.author"), AUTHOR),
        ("GitHub".to_string(), GITHUB),
        ("LinkedIn".to_string(), LINKEDIN),
        (t!("about.repository"), REPOSITORY),
        (t!("about.license"), LICENSE),
        (t!("about.usage"), USAGE_URL),
        ("README".to_string(), README_URL),
        (t!("about.roadmap"), ROADMAP_URL),
    ];
    let col = rows
        .iter()
        .map(|(l, _)| crate::i18n::width(l))
        .max()
        .unwrap_or(0);
    for (label, value) in &rows {
        println!("  {label}{}  {value}", crate::i18n::pad(label, col));
    }
    Ok(EXIT_OK)
}
