//! The agent skills in `skills/` never teach a command or an option that jlock does not have: every
//! `jlock ...` they show, in code blocks and in inline code, is checked against the command table.
//! Each skill also has the frontmatter its tool needs (`name` matching its folder, a `description`).

#[path = "../src/commands.rs"]
#[allow(dead_code)]
mod commands;

use std::path::{Path, PathBuf};

const TOOLS: [(&str, &str); 3] = [
    ("claude-code", "claude:"),
    ("opencode", "opencode:"),
    ("codex", "codex:"),
];
/// Options every command takes, handled before its own parser.
const GLOBAL: [&str; 3] = ["lang", "help", "json"];

fn skill(tool: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("skills")
        .join(tool)
        .join("jlock")
        .join("SKILL.md")
}

/// Every command line the text shows: lines of code blocks and inline code spans starting with `jlock `.
fn command_lines(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_block = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            in_block = !in_block;
            continue;
        }
        if in_block {
            let l = line.trim();
            if l.starts_with("jlock ") {
                out.push(l.to_string());
            }
            continue;
        }
        for (i, span) in line.split('`').enumerate() {
            if i % 2 == 1 && span.starts_with("jlock ") {
                out.push(span.to_string());
            }
        }
    }
    out
}

/// What is wrong with one command line, if anything.
fn check(line: &str) -> Option<String> {
    let words: Vec<&str> = line.split_whitespace().collect();
    let cmd = words.get(1)?;
    if *cmd == "--version" {
        return None;
    }
    let Some(c) = commands::find(cmd) else {
        return Some(format!("no command '{cmd}'"));
    };
    for w in &words[2..] {
        if *w == "--" {
            break; // the rest belongs to the command of `run`
        }
        let Some(flag) = w.strip_prefix("--") else {
            continue;
        };
        let name = flag.split('=').next().unwrap();
        if c.opt(name).is_none() && !GLOBAL.contains(&name) {
            return Some(format!("{cmd} has no option --{name}"));
        }
    }
    None
}

#[test]
fn every_skill_has_its_frontmatter() {
    for (tool, _) in TOOLS {
        let text = std::fs::read_to_string(skill(tool)).unwrap_or_else(|e| panic!("{tool}: {e}"));
        let mut lines = text.lines();
        assert_eq!(lines.next(), Some("---"), "{tool}: frontmatter first");
        let front: Vec<&str> = lines.by_ref().take_while(|l| *l != "---").collect();
        assert!(
            front.contains(&"name: jlock"),
            "{tool}: name matches the folder"
        );
        let description = front
            .iter()
            .find_map(|l| l.strip_prefix("description: "))
            .unwrap_or_else(|| panic!("{tool}: no description"));
        assert!(
            description.len() > 80,
            "{tool}: description too short to trigger"
        );
        assert!(
            lines.next().is_some(),
            "{tool}: no body after the frontmatter"
        );
    }
}

#[test]
fn every_command_in_the_skills_exists() {
    for (tool, owner) in TOOLS {
        let text = std::fs::read_to_string(skill(tool)).unwrap();
        let lines = command_lines(&text);
        assert!(
            lines.len() >= 10,
            "{tool}: only {} commands found",
            lines.len()
        );
        for line in &lines {
            if let Some(problem) = check(line) {
                panic!("{tool}: `{line}`: {problem}");
            }
            if line.contains("--owner ") && !line.contains("--owner <") {
                assert!(
                    line.contains(owner),
                    "{tool}: `{line}` uses another tool's owner"
                );
            }
        }
    }
}

#[test]
fn the_check_catches_mistakes() {
    assert!(check("jlock hold a --detach --owner x").is_none());
    assert!(check("jlock run name:b --owner x -- cargo build --release").is_none());
    assert!(check("jlock acquire a --detach").is_some());
    assert!(check("jlock lock a").is_some());
}
