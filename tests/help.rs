//! The help is complete (every command, every option the parser takes), fits in 100 columns, comes
//! in the chosen language; tips are on stderr, never in `--json`, and `JLOCK_HINTS=0` drops them.

mod common;

#[path = "../src/commands.rs"]
#[allow(dead_code)]
mod commands;

use common::Env;
use std::time::Duration;

/// Arguments that make each command stop with a usage error right after reading its options, so
/// that trying an option has no effect (and `list --watch` does not run forever).
fn after_options(cmd: &str) -> &'static [&'static str] {
    match cmd {
        "list" | "graph" | "gc" | "about" => &["extra-argument"],
        _ => &[],
    }
}

#[test]
fn every_command_has_a_help_page() {
    let env = Env::new("help-pages");
    for c in commands::COMMANDS {
        let page = env.run(&["help", c.name]);
        assert_eq!(page.code, 0, "help {}: {}", c.name, page.stderr);
        assert!(page.stdout.starts_with(&format!("jlock {} - ", c.name)));
        assert!(page.stdout.contains("Exit codes:"), "{}", page.stdout);
        assert!(!page.stdout.contains("help.cmd."), "raw key in {}", c.name);
        let flag = env.run(&[c.name, "--help"]);
        assert_eq!(flag.code, 0, "{} --help", c.name);
        assert_eq!(flag.stdout, page.stdout, "{} --help", c.name);
    }
    let general = env.run(&["help"]);
    assert_eq!(general.code, 0);
    for c in commands::COMMANDS {
        assert!(
            general.stdout.contains(c.name),
            "{} not in jlock help",
            c.name
        );
    }
    assert_eq!(env.run(&["--help"]).stdout, general.stdout);
}

#[test]
fn every_option_has_a_line_of_help_and_is_taken_by_the_parser() {
    let env = Env::new("help-options");
    for c in commands::COMMANDS {
        let page = env.run(&["help", c.name]).stdout;
        for o in c.opts {
            let label = format!("--{}", o.name);
            assert!(
                page.lines().any(|l| l.trim_start().starts_with(&label)),
                "help {} has no line for {label}",
                c.name
            );
            let mut args = vec![c.name.to_string(), label.clone()];
            if o.meta.is_some() {
                let value = if o.example.is_empty() { "x" } else { o.example };
                args.push(value.trim_matches('"').to_string());
            }
            args.extend(after_options(c.name).iter().map(|s| s.to_string()));
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            let done = env.spawn(&args).finish(Duration::from_secs(20));
            assert!(
                !done.stderr.contains("unknown option"),
                "{args:?} refused by the parser: {}",
                done.stderr
            );
        }
        let bad = env.run(&[c.name, "--no-such-option"]);
        assert_eq!(bad.code, 2, "{} --no-such-option", c.name);
        assert!(bad.stderr.contains("unknown option --no-such-option"));
    }
}

#[test]
fn help_fits_in_100_columns() {
    let env = Env::new("help-width");
    for lang in ["en", "pt-BR"] {
        let mut pages = vec![env.run(&["--lang", lang, "help"]).stdout];
        for c in commands::COMMANDS {
            pages.push(env.run(&["--lang", lang, "help", c.name]).stdout);
        }
        for page in pages {
            for line in page.lines() {
                assert!(line.chars().count() <= 100, "{lang}: {line}");
            }
        }
    }
}

#[test]
fn unknown_command_suggests_the_closest() {
    let env = Env::new("help-unknown");
    let done = env.run(&["help", "aquire"]);
    assert_eq!(done.code, 2);
    assert!(
        done.stderr.contains("did you mean 'acquire'"),
        "{}",
        done.stderr
    );
    assert!(done.stderr.contains("release"), "the list: {}", done.stderr);
    let done = env.run(&["help", "nao-existe"]);
    assert_eq!(done.code, 2);
    assert!(done.stderr.contains("Commands: acquire"), "{}", done.stderr);
    let done = env.run(&["stauts", "x"]);
    assert_eq!(done.code, 2);
    assert!(
        done.stderr.contains("did you mean 'status'"),
        "{}",
        done.stderr
    );
}

#[test]
fn help_in_portuguese() {
    let env = Env::new("help-pt");
    let done = env.run(&["--lang", "pt-BR", "help", "acquire"]);
    assert_eq!(done.code, 0);
    assert!(done.stdout.contains("Opções:"), "{}", done.stdout);
    assert!(done.stdout.contains("Códigos de saída:"), "{}", done.stdout);
    assert!(
        done.stdout.contains("Pega todas as chaves"),
        "{}",
        done.stdout
    );
    for opt in ["--wait", "--no-wait", "--timeout", "--ttl", "--owner"] {
        assert!(done.stdout.contains(opt), "{opt}");
    }
}

#[test]
fn about_and_version() {
    let env = Env::new("help-about");
    let v = env.run(&["--version"]);
    assert_eq!(v.stdout, format!("jlock {}\n", env!("CARGO_PKG_VERSION")));
    let about = env.run(&["about"]);
    assert_eq!(about.code, 0);
    for want in [
        "Rodrigo Jäger",
        "https://github.com/rodrigojager",
        "https://www.linkedin.com/in/rodrigojager",
        "https://github.com/rodrigojager/jlock",
        "MIT",
        "USAGE.md",
        "README.md",
        "ROADMAP.md",
    ] {
        assert!(about.stdout.contains(want), "{want}: {}", about.stdout);
    }
    let json = env.run(&["about", "--json"]).json();
    assert_eq!(json["author"], "Rodrigo Jäger");
    assert_eq!(json["github"], "https://github.com/rodrigojager");
    assert_eq!(json["linkedin"], "https://www.linkedin.com/in/rodrigojager");
    assert_eq!(json["license"], "MIT");
    let footer = env.run(&["help"]).stdout;
    assert!(footer.trim_end().ends_with(
        "jlock by Rodrigo Jäger · github.com/rodrigojager · linkedin.com/in/rodrigojager"
    ));
}

#[test]
fn tips_go_to_stderr_and_can_be_turned_off() {
    let mut env = Env::new("help-tips");
    assert_eq!(env.run(&["acquire", "name:x", "--owner", "a"]).code, 0);
    let busy = env.run(&["acquire", "name:x", "--owner", "b", "--no-wait"]);
    assert_eq!(busy.code, 10);
    assert!(
        busy.stdout.starts_with("busy name:x held by a"),
        "{}",
        busy.stdout
    );
    assert_eq!(busy.stdout.lines().count(), 1, "{}", busy.stdout);
    assert!(
        busy.stderr.contains("tip: jlock status name:x"),
        "{}",
        busy.stderr
    );

    let json = env.run(&["acquire", "name:x", "--owner", "b", "--no-wait", "--json"]);
    assert!(!json.stdout.contains("tip"), "{}", json.stdout);
    assert!(!json.stderr.contains("tip"), "{}", json.stderr);

    let refused = env.run(&["release", "name:x", "--owner", "b"]);
    assert_eq!(refused.code, 13);
    assert!(refused.stderr.contains("--owner a"), "{}", refused.stderr);

    env.set("JLOCK_HINTS", "0");
    let busy = env.run(&["acquire", "name:x", "--owner", "b", "--no-wait"]);
    assert_eq!(busy.code, 10);
    assert!(!busy.stderr.contains("tip"), "{}", busy.stderr);
}

#[test]
fn list_footer_and_status_explain_themselves() {
    let env = Env::new("help-list");
    let folder = env.path("proj/src");
    let file = env.path("proj/src/a.ts");
    assert_eq!(env.run(&["acquire", &folder, "--owner", "a"]).code, 0);
    let list = env.run(&["list"]);
    assert!(
        list.stdout.contains("\n1 lock, 0 waiting\n"),
        "{}",
        list.stdout
    );
    assert!(!list.stdout.contains("(dead)"), "{}", list.stdout);

    let status = env.run(&["status", &file]);
    assert!(status.stdout.contains("conflicts:"), "{}", status.stdout);
    assert!(
        status.stdout.contains("why: a lock on a folder"),
        "{}",
        status.stdout
    );
    assert_eq!(env.run(&["release", &folder, "--owner", "a"]).code, 0);

    assert_eq!(env.run(&["acquire", &file, "--owner", "b"]).code, 0);
    let status = env.run(&["status", &folder]);
    assert!(
        status.stdout.contains("why: a folder can only be taken"),
        "{}",
        status.stdout
    );
}
