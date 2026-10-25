//! Argument parsing by hand, exit codes (plan section 6) and command dispatch.

use crate::i18n::{txt, Text};
use std::time::Duration;

pub const EXIT_OK: i32 = 0;
pub const EXIT_USAGE: i32 = 2;
pub const EXIT_BUSY: i32 = 10;
pub const EXIT_TIMEOUT: i32 = 11;
pub const EXIT_DEADLOCK: i32 = 12;
pub const EXIT_NOT_OWNER: i32 = 13;
pub const EXIT_LOST: i32 = 14;
pub const EXIT_FS: i32 = 20;

#[derive(Debug)]
pub struct Failure {
    pub code: i32,
    pub msg: Text,
}

pub type CmdResult = Result<i32, Failure>;

pub fn usage(msg: Text) -> Failure {
    Failure {
        code: EXIT_USAGE,
        msg,
    }
}

/// A file system failure: what jlock was doing, then the OS message (which comes in the OS's own
/// language and names the path).
pub fn fs_error(context: Text, err: &std::io::Error) -> Failure {
    Failure {
        code: EXIT_FS,
        msg: context.with(err),
    }
}

/// `<cmd>: unknown option --<flag>`.
pub fn unknown_option(cmd: &str, flag: &str) -> Failure {
    usage(txt!("error.unknown_option", cmd = cmd, flag = flag))
}

/// `<cmd>: unexpected '--'`.
pub fn unexpected_rest(cmd: &str) -> Failure {
    usage(txt!("error.unexpected_rest", cmd = cmd))
}

pub enum Arg {
    Flag(String),
    Pos(String),
    Rest(Vec<String>),
}

/// Walks the arguments: `--flag`, `--flag=value`, `--flag value`, positionals, and `--` (rest).
pub struct Parser {
    args: Vec<String>,
    pos: usize,
    pending: Option<String>,
}

impl Parser {
    pub fn new(args: Vec<String>) -> Parser {
        Parser {
            args,
            pos: 0,
            pending: None,
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Option<Arg> {
        self.pending = None;
        let arg = self.args.get(self.pos)?.clone();
        self.pos += 1;
        if arg == "--" {
            let rest = self.args[self.pos..].to_vec();
            self.pos = self.args.len();
            return Some(Arg::Rest(rest));
        }
        if let Some(flag) = arg.strip_prefix("--") {
            if let Some((name, value)) = flag.split_once('=') {
                self.pending = Some(value.to_string());
                return Some(Arg::Flag(name.to_string()));
            }
            return Some(Arg::Flag(flag.to_string()));
        }
        match arg.as_str() {
            "-h" => Some(Arg::Flag("help".into())),
            "-V" => Some(Arg::Flag("version".into())),
            _ => Some(Arg::Pos(arg)),
        }
    }

    pub fn value(&mut self, flag: &str) -> Result<String, Failure> {
        if let Some(v) = self.pending.take() {
            return Ok(v);
        }
        match self.args.get(self.pos) {
            Some(v) => {
                self.pos += 1;
                Ok(v.clone())
            }
            None => Err(usage(txt!("error.needs_value", flag = flag))),
        }
    }
}

/// `500ms`, `30s`, `2m`, `2h`, `1d`; a bare number is seconds.
pub fn parse_duration(text: &str) -> Result<Duration, Failure> {
    let t = text.trim();
    let split = t
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .unwrap_or(t.len());
    let (num, unit) = t.split_at(split);
    let bad = || usage(txt!("error.bad_duration", text = text));
    let n: f64 = num.parse().map_err(|_| bad())?;
    let ms = match unit {
        "ms" => n,
        "" | "s" => n * 1000.0,
        "m" => n * 60_000.0,
        "h" => n * 3_600_000.0,
        "d" => n * 86_400_000.0,
        _ => return Err(bad()),
    };
    Ok(Duration::from_millis(ms as u64))
}

pub fn print_json(value: &serde_json::Value) {
    println!("{value}");
}

/// Runs the command line and returns the process exit code.
pub fn main(mut args: Vec<String>) -> i32 {
    crate::i18n::select(&mut args);
    let json = args.iter().any(|a| a == "--json");
    let result = dispatch(args);
    // Commands that reached the registry keep it tidy: the index after a change, and the global
    // sweep when it is due. Neither changes the command's result.
    if crate::store::opened() {
        if let Ok(store) = crate::store::Store::open() {
            if crate::log::changed() {
                let _ = crate::index::rewrite(&store);
            }
            crate::sweep::maybe_sweep(&store);
        }
    }
    match result {
        Ok(code) => code,
        Err(f) => {
            if json {
                print_json(&serde_json::json!({ "result": "error", "error": f.msg.en }));
            } else {
                eprintln!("jlock: {}", f.msg.local);
            }
            f.code
        }
    }
}

/// The version `--version` prints. Debug builds (the ones the tests run) take `JLOCK_FAKE_VERSION`,
/// so the update test can tell an installed "new" binary from the old one; release builds never do.
fn version() -> String {
    if cfg!(debug_assertions) {
        if let Ok(v) = std::env::var("JLOCK_FAKE_VERSION") {
            return v;
        }
    }
    env!("CARGO_PKG_VERSION").to_string()
}

/// `jlock help [<command>]`.
fn help(args: Vec<String>) -> CmdResult {
    let me = crate::commands::find("help").expect("help is in the table");
    if let crate::help::Check::Help = crate::help::check(me, &args)? {
        print!("{}", crate::help::command(me));
        return Ok(EXIT_OK);
    }
    match args.first().map(String::as_str) {
        None => print!("{}", crate::help::general()),
        Some(name) => match crate::commands::find(name) {
            Some(c) => print!("{}", crate::help::command(c)),
            None => return Err(crate::help::unknown_command(name)),
        },
    }
    Ok(EXIT_OK)
}

fn dispatch(args: Vec<String>) -> CmdResult {
    let mut iter = args.into_iter();
    let Some(cmd) = iter.next() else {
        print!("{}", crate::help::general());
        return Err(usage(txt!("error.missing_command")));
    };
    let rest: Vec<String> = iter.collect();
    match cmd.as_str() {
        "--version" | "-V" | "version" => {
            println!("jlock {}", version());
            return Ok(EXIT_OK);
        }
        "--help" | "-h" | "help" => return help(rest),
        _ => {}
    }
    let Some(c) = crate::commands::find(&cmd) else {
        return Err(crate::help::unknown_command(&cmd));
    };
    // Options are checked against the table the help is made from: an option either has a line
    // of help or is refused here.
    if let crate::help::Check::Help = crate::help::check(c, &rest)? {
        print!("{}", crate::help::command(c));
        return Ok(EXIT_OK);
    }
    match cmd.as_str() {
        "about" => crate::cmd::about::run(rest),
        "doctor" => crate::cmd::doctor::run(rest),
        "update" => crate::cmd::update::run(rest),
        "status" => crate::cmd::status::run(rest),
        "acquire" => crate::cmd::acquire::run(rest),
        "release" => crate::cmd::release::run(rest),
        "hold" => crate::cmd::hold::run(rest),
        "run" => crate::cmd::run::run(rest),
        "heartbeat" => crate::cmd::heartbeat::run(rest),
        "renew" => crate::cmd::renew::run(rest),
        "graph" => crate::cmd::graph::run(rest),
        "list" => crate::cmd::list::run(rest),
        "unlock" => crate::cmd::unlock::run(rest),
        "gc" => crate::cmd::gc::run(rest),
        other => Err(crate::help::unknown_command(other)),
    }
}
