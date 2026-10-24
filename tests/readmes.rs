//! The READMEs never lie: every block marked `<!-- jlock-example -->` is run again, in a temporary
//! `JLOCK_HOME` and in the README's language, and its output must match what the README shows
//! (numbers, times and spacing aside). `JLOCK_README_WRITE=1 cargo test --test readmes` writes the
//! real output into the files instead. The eleven READMEs also share the language bar, the sections,
//! the example commands and the author's links, and no relative link is broken.

mod common;

use common::Env;
use std::path::{Path, PathBuf};

const LANGUAGES: [&str; 11] = [
    "en", "pt-BR", "es", "fr", "de", "it", "zh-CN", "ja", "ru", "ar", "hi",
];
/// Translations written without a native speaker: they end with a note asking for a review.
const REVIEW_PENDING: [&str; 5] = ["zh-CN", "ja", "ru", "ar", "hi"];
const MARK: &str = "<!-- jlock-example -->";

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn readme(tag: &str) -> PathBuf {
    if tag == "en" {
        root().join("README.md")
    } else {
        root().join(format!("README.{tag}.md"))
    }
}

fn read(tag: &str) -> String {
    std::fs::read_to_string(readme(tag)).unwrap_or_else(|e| panic!("README for {tag}: {e}"))
}

/// The value of `pub const <name>: &str = "...";` in `src/cmd/about.rs`.
fn about_const(name: &str) -> String {
    let src = std::fs::read_to_string(root().join("src/cmd/about.rs")).unwrap();
    let line = src
        .lines()
        .find(|l| l.starts_with(&format!("pub const {name}: &str = ")))
        .unwrap_or_else(|| panic!("{name} not in about.rs"));
    line.split('"').nth(1).unwrap().to_string()
}

/// Splits a command line like a shell would, for the simple cases the READMEs use ("..." quotes).
fn split(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let (mut quoted, mut any) = (false, false);
    for c in line.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                any = true;
            }
            ' ' if !quoted => {
                if any {
                    out.push(std::mem::take(&mut cur));
                    any = false;
                }
            }
            _ => {
                cur.push(c);
                any = true;
            }
        }
    }
    if any {
        out.push(cur);
    }
    out
}

/// Numbers, times and dates become `0`; runs of spaces become one. What is left must match.
fn normalize(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_ascii_digit() {
            while chars
                .peek()
                .is_some_and(|n| n.is_ascii_alphanumeric() || matches!(n, '.' | ':' | '-'))
            {
                chars.next();
            }
            out.push('0');
        } else if c == ' ' {
            while chars.peek() == Some(&' ') {
                chars.next();
            }
            out.push(' ');
        } else {
            out.push(c);
        }
    }
    out.lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

/// One example block: where its lines are, its commands and the output shown under each.
struct Block {
    start: usize,
    end: usize,
    steps: Vec<(String, Vec<String>)>,
}

fn blocks(text: &str) -> Vec<Block> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if lines[i].trim() != MARK {
            i += 1;
            continue;
        }
        let open = (i + 1..lines.len())
            .find(|&j| lines[j].starts_with("```"))
            .expect("a code block after the marker");
        let close = (open + 1..lines.len())
            .find(|&j| lines[j].starts_with("```"))
            .expect("the end of the code block");
        let mut steps: Vec<(String, Vec<String>)> = Vec::new();
        for line in &lines[open + 1..close] {
            match line.strip_prefix("$ ") {
                Some(cmd) => steps.push((cmd.to_string(), Vec::new())),
                None => steps
                    .last_mut()
                    .expect("output before any command")
                    .1
                    .push(line.to_string()),
            }
        }
        out.push(Block {
            start: open + 1,
            end: close,
            steps,
        });
        i = close + 1;
    }
    out
}

/// Runs the commands of one block in a fresh registry; returns each command's output.
fn run_block(tag: &str, n: usize, block: &Block) -> Vec<String> {
    let mut env = Env::new(&format!("readme-{tag}-{n}"));
    env.set("JLOCK_LANG", tag);
    env.set("JLOCK_HINTS", "1");
    block
        .steps
        .iter()
        .map(|(cmd, _)| {
            let args: Vec<String> = split(cmd)
                .into_iter()
                .map(|a| {
                    if a == "jlock" {
                        common::JLOCK.to_string()
                    } else {
                        a
                    }
                })
                .collect();
            assert_eq!(args[0], common::JLOCK, "{tag}: not a jlock command: {cmd}");
            let mut c = std::process::Command::new(&args[0]);
            c.args(&args[1..]);
            env.apply(&mut c);
            let out = c.output().unwrap();
            let text = String::from_utf8_lossy(&out.stdout).into_owned()
                + &String::from_utf8_lossy(&out.stderr);
            text.lines()
                .map(str::trim_end)
                .collect::<Vec<_>>()
                .join("\n")
        })
        .collect()
}

#[test]
fn examples_match_the_real_output() {
    let write = std::env::var("JLOCK_README_WRITE").is_ok_and(|v| v == "1");
    for tag in LANGUAGES {
        let text = read(tag);
        let found = blocks(&text);
        assert!(!found.is_empty(), "{tag}: no {MARK} block");
        let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
        // From the last block up, so that rewriting one does not move the others.
        for (n, block) in found.iter().enumerate().rev() {
            let real = run_block(tag, n, block);
            if write {
                let mut body = Vec::new();
                for ((cmd, _), out) in block.steps.iter().zip(&real) {
                    body.push(format!("$ {cmd}"));
                    body.extend(out.lines().map(str::to_string));
                }
                lines.splice(block.start..block.end, body);
                continue;
            }
            for ((cmd, shown), out) in block.steps.iter().zip(&real) {
                assert_eq!(
                    normalize(&shown.join("\n")),
                    normalize(out),
                    "{tag}: README shows a different output for `{cmd}`"
                );
            }
        }
        if write {
            std::fs::write(readme(tag), lines.join("\n") + "\n").unwrap();
        }
    }
}

#[test]
fn every_readme_has_the_same_shape() {
    let english = read("en");
    let bar = |text: &str| {
        let lines: Vec<&str> = text.lines().collect();
        let at = lines
            .iter()
            .position(|l| l.trim() == "<!-- languages -->")
            .expect("<!-- languages --> marker");
        lines[at + 1].to_string()
    };
    let sections = |text: &str| text.lines().filter(|l| l.starts_with("## ")).count();
    let commands = |text: &str| -> Vec<String> {
        blocks(text)
            .into_iter()
            .flat_map(|b| b.steps.into_iter().map(|(c, _)| c))
            .collect()
    };
    for tag in LANGUAGES {
        let text = read(tag);
        assert_eq!(bar(&text), bar(&english), "{tag}: language bar");
        for other in LANGUAGES {
            let file = readme(other);
            let name = file.file_name().unwrap().to_string_lossy();
            assert!(bar(&text).contains(&format!("]({name})")), "{tag}: {name}");
        }
        assert_eq!(
            sections(&text),
            sections(&english),
            "{tag}: number of ## sections"
        );
        assert_eq!(
            commands(&text),
            commands(&english),
            "{tag}: example commands"
        );
        assert!(text.contains(&about_const("GITHUB")), "{tag}: GitHub link");
        assert!(
            text.contains(&about_const("LINKEDIN")),
            "{tag}: LinkedIn link"
        );
        if REVIEW_PENDING.contains(&tag) {
            let tail: Vec<&str> = text.lines().rev().take(6).collect();
            assert!(
                tail.iter().any(|l| l.contains("CONTRIBUTING.md")),
                "{tag}: the review note at the end"
            );
        }
        if tag == "ar" {
            assert!(text.contains("<div dir=\"rtl\">"), "ar: right to left");
            assert!(text.contains("<div dir=\"ltr\">"), "ar: code left to right");
        }
    }
}

#[test]
fn relative_links_point_at_files_that_exist() {
    for tag in LANGUAGES {
        let text = read(tag);
        let mut rest = text.as_str();
        while let Some(at) = rest.find("](") {
            let after = &rest[at + 2..];
            let end = after.find(')').unwrap_or(after.len());
            let target = &after[..end];
            rest = &after[end..];
            if target.starts_with("http")
                || target.starts_with('#')
                || target.starts_with("mailto:")
            {
                continue;
            }
            let path = target.split('#').next().unwrap();
            assert!(root().join(path).exists(), "{tag}: broken link {target}");
        }
    }
}
