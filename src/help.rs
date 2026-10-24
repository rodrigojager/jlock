//! `jlock help [<command>]` and `jlock <command> --help`, rendered from the table in `commands.rs`
//! in the user's language and wrapped at 100 columns; the tips printed after some results; the
//! check that refuses options the table does not know.

use crate::cli::{usage, Failure};
use crate::cmd::about;
use crate::commands::{self, Command, COMMANDS, GROUPS};
use crate::i18n::{char_width, has, pad, t, tr, txt, width};

pub const COLUMNS: usize = 100;

/// Breaks `text` into lines of at most `columns` terminal columns. The first line starts with
/// `first`, the others with `indent`. Words stay whole; a word longer than a line (CJK text has no
/// spaces) is broken between characters. A `\n` in the text starts a new line; a no-break space
/// (U+00A0) keeps two words on the same line and comes out as a plain space.
pub fn wrap(text: &str, first: &str, indent: &str, columns: usize) -> String {
    let mut out = String::new();
    let mut line = first.to_string();
    let mut fresh = true; // only the indent on this line so far
    let flush = |line: &mut String, fresh: &mut bool, out: &mut String| {
        *out += line.trim_end();
        out.push('\n');
        *line = indent.to_string();
        *fresh = true;
    };
    for (i, para) in text.split('\n').enumerate() {
        if i > 0 {
            flush(&mut line, &mut fresh, &mut out);
        }
        for word in para.split(' ').filter(|w| !w.is_empty()) {
            let space = usize::from(!fresh);
            if width(&line) + space + width(word) <= columns {
                if !fresh {
                    line.push(' ');
                }
                line += word;
                fresh = false;
                continue;
            }
            if !fresh {
                flush(&mut line, &mut fresh, &mut out);
            }
            for c in word.chars() {
                if !fresh && width(&line) + char_width(c) > columns {
                    flush(&mut line, &mut fresh, &mut out);
                }
                line.push(c);
                fresh = false;
            }
        }
    }
    out += line.trim_end();
    out.push('\n');
    out.replace('\u{a0}', " ")
}

/// A two-column table (`label  text`), the text wrapped under itself. Labels wider than `max`
/// put their text on the next line.
fn table(rows: &[(String, String)], max: usize) -> String {
    let col = rows
        .iter()
        .map(|(l, _)| width(l))
        .filter(|w| *w <= max)
        .max()
        .unwrap_or(0);
    let indent = " ".repeat(2 + col + 2);
    let mut out = String::new();
    for (label, text) in rows {
        if width(label) > max {
            out += &format!("  {label}\n");
            out += &wrap(text, &indent, &indent, COLUMNS);
        } else {
            let first = format!("  {label}{}  ", pad(label, col));
            out += &wrap(text, &first, &indent, COLUMNS);
        }
    }
    out
}

fn summary(c: &Command) -> String {
    tr(&format!("help.cmd.{}.summary", c.name))
}

fn heading(key: &str) -> String {
    format!("{}:\n", tr(key))
}

/// `jlock help`: what jlock is, the commands by intent, the global options.
pub fn general() -> String {
    let mut out = format!(
        "jlock {} - {}\n\n",
        env!("CARGO_PKG_VERSION"),
        t!("help.tagline")
    );
    out += &wrap(&t!("help.intro"), "", "", COLUMNS);
    out += &format!("\n{}\n", t!("help.usage_line"));
    // The same column for every group: names padded to the longest one.
    let col = COMMANDS
        .iter()
        .map(|c| c.name.len())
        .chain(["--version".len()])
        .max()
        .unwrap_or(0);
    let name = |n: &str| format!("{n:<col$}");
    for group in GROUPS {
        let mut rows: Vec<(String, String)> = COMMANDS
            .iter()
            .filter(|c| c.group == *group)
            .map(|c| (name(c.name), summary(c)))
            .collect();
        if *group == commands::HELP {
            rows.push((name("--version"), t!("help.version")));
        }
        out += "\n";
        out += &heading(group);
        out += &table(&rows, 12);
    }
    out += "\n";
    out += &heading("help.label.global");
    out += &table(
        &[
            ("--json".into(), t!("help.global.json")),
            (
                format!("--lang <{}>", t!("help.meta.tag")),
                t!("help.global.lang"),
            ),
            ("--help, -h".into(), t!("help.global.help")),
        ],
        20,
    );
    out += "\n";
    out += &wrap(&t!("help.keys"), "", "", COLUMNS);
    out += "\n";
    out += &wrap(&t!("help.more"), "", "", COLUMNS);
    out += &wrap(&t!("help.docs", url = about::USAGE_URL), "", "", COLUMNS);
    out += &format!("\n{}\n", about::footer());
    out
}

/// `--owner <id>`
pub fn opt_label(o: &commands::Opt) -> String {
    match o.meta {
        Some(m) => format!("--{} <{}>", o.name, tr(&format!("help.meta.{m}"))),
        None => format!("--{}", o.name),
    }
}

/// `jlock help <command>`: synopsis, description, every option, examples, exit codes, see also.
pub fn command(c: &Command) -> String {
    let key = |part: &str| format!("help.cmd.{}.{part}", c.name);
    let mut out = format!("jlock {} - {}\n\n", c.name, summary(c));
    out += &heading("help.label.usage");
    out += &format!("  jlock {} {}\n\n", c.name, tr(&key("usage")));
    out += &wrap(&tr(&key("desc")), "", "", COLUMNS);
    if has(&key("more")) {
        out += "\n";
        out += &wrap(&tr(&key("more")), "", "", COLUMNS);
    }
    if !c.opts.is_empty() {
        let rows: Vec<(String, String)> = c
            .opts
            .iter()
            .map(|o| {
                let mut text = tr(o.help);
                if !o.example.is_empty() {
                    text += &format!(
                        " {}:\u{a0}--{}\u{a0}{}",
                        t!("help.label.example"),
                        o.name,
                        o.example
                    );
                }
                (opt_label(o), text)
            })
            .collect();
        out += "\n";
        out += &heading("help.label.options");
        out += &table(&rows, 22);
    }
    out += &wrap(
        &t!("help.also_global", tag = tr("help.meta.tag")),
        "  ",
        "  ",
        COLUMNS,
    );
    out += "\n";
    out += &heading("help.label.examples");
    for line in tr(&key("examples")).lines() {
        out += &format!("  {line}\n");
    }
    out += "\n";
    out += &heading("help.label.exit");
    let codes: Vec<(String, String)> = c
        .codes
        .iter()
        .map(|code| {
            let own = format!("help.exit.{}.{code}", c.name);
            let text = if has(&own) {
                tr(&own)
            } else {
                tr(&format!("help.exit.{code}"))
            };
            (code.to_string(), text)
        })
        .collect();
    out += &table(&codes, 6);
    out += "\n";
    out += &wrap(
        &t!("help.see_also", list = c.see.join(", ")),
        "",
        "  ",
        COLUMNS,
    );
    out
}

/// Levenshtein distance, for "did you mean".
fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cur = row[j + 1];
            row[j + 1] = (prev + usize::from(ca != *cb)).min(row[j] + 1).min(cur + 1);
            prev = cur;
        }
    }
    row[b.len()]
}

/// The closest of `names` to `word`, if it is close enough to be a typo.
pub fn closest<'a>(word: &str, names: impl Iterator<Item = &'a str>) -> Option<&'a str> {
    names
        .map(|n| (distance(word, n), n))
        .filter(|(d, n)| *d <= 2.max(n.len() / 3))
        .min_by_key(|(d, _)| *d)
        .map(|(_, n)| n)
}

/// An unknown command: exit 2 with the closest name and the list.
pub fn unknown_command(word: &str) -> Failure {
    let names: Vec<&str> = COMMANDS.iter().map(|c| c.name).collect();
    let list = names.join(", ");
    match closest(word, names.iter().copied()) {
        Some(near) => usage(txt!(
            "error.unknown_command_near",
            cmd = word,
            near = near,
            list = list
        )),
        None => usage(txt!("error.unknown_command", cmd = word, list = list)),
    }
}

/// What the arguments of a known command ask before it runs.
pub enum Check {
    Run,
    Help,
}

/// Every `--option` before a `--` must be in the command's table (the command's own parser then
/// reads them); `--help`/`-h` asks for the command's help.
pub fn check(c: &Command, args: &[String]) -> Result<Check, Failure> {
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        i += 1;
        if a == "--" {
            break;
        }
        if a == "--help" || a == "-h" {
            return Ok(Check::Help);
        }
        let Some(flag) = a.strip_prefix("--") else {
            continue;
        };
        let (name, inline) = match flag.split_once('=') {
            Some((n, _)) => (n, true),
            None => (flag, false),
        };
        let Some(o) = c.opt(name) else {
            let near = closest(name, c.opts.iter().map(|o| o.name));
            return Err(match near {
                Some(near) => usage(txt!(
                    "error.unknown_option_near",
                    cmd = c.name,
                    flag = name,
                    near = near
                )),
                None => usage(txt!("error.unknown_option", cmd = c.name, flag = name)),
            });
        };
        if o.meta.is_some() && !inline {
            i += 1; // its value, whatever it looks like
        }
    }
    Ok(Check::Run)
}

/// Tips on the next step after some results, on stderr (stdout keeps one line per key).
/// Never with `--json`; `JLOCK_HINTS=0` turns them off.
pub fn hint(json: bool, text: String) {
    if json || std::env::var("JLOCK_HINTS").is_ok_and(|v| v.trim() == "0") {
        return;
    }
    eprintln!("{text}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_words_and_wide_text() {
        assert_eq!(wrap("aa bb cc", "", "  ", 5), "aa bb\n  cc\n");
        assert_eq!(wrap("一二三四五", "", "", 4), "一二\n三四\n五\n");
        for line in wrap(&"word ".repeat(60), "  ", "  ", 30).lines() {
            assert!(width(line) <= 30, "{line}");
        }
    }

    #[test]
    fn suggests_close_names() {
        let names = ["acquire", "release", "status"];
        assert_eq!(closest("aquire", names.iter().copied()), Some("acquire"));
        assert_eq!(closest("stauts", names.iter().copied()), Some("status"));
        assert_eq!(closest("zzzzzz", names.iter().copied()), None);
    }
}
