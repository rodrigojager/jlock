//! Argument parsing by hand, exit codes (plan section 6) and command dispatch.

use std::time::Duration;

pub const EXIT_OK: i32 = 0;
pub const EXIT_USAGE: i32 = 2;
pub const EXIT_BUSY: i32 = 10;
pub const EXIT_TIMEOUT: i32 = 11;
pub const EXIT_DEADLOCK: i32 = 12;
pub const EXIT_NOT_OWNER: i32 = 13;
pub const EXIT_LOST: i32 = 14;
pub const EXIT_FS: i32 = 20;

pub const HELP: &str = "\
jlock - machine-wide locks for files, folders and names

Usage:
  jlock acquire <key...> [--owner id] [--group g] [--intent \"...\"] [--meta k=v...]
                [--ttl 2h] [--wait|--no-wait] [--timeout 30s] [--json]
  jlock run <key...> [acquire options] -- <command...>
  jlock hold <key...> [acquire options]
  jlock release [<key...>] [--owner id] [--group g] [--any-owner] [--json]
  jlock heartbeat <key> --owner id [--json]
  jlock status <key> [--json]
  jlock list [--owner x] [--group g] [--under path] [--stale] [--json] [--watch]
  jlock graph [--json]
  jlock unlock (<key> | --owner x | --group g) --force [--reason \"...\"] [--by id] [--json]
  jlock gc [--json]
  jlock --version | --help

Keys: a path (file or folder, any form), name:<text>, or * (conflicts with everything).

Exit codes: 0 ok, 2 usage, 10 busy, 11 timeout, 12 deadlock, 13 not owner,
            14 lock lost, 20 file system error.
";

#[derive(Debug)]
pub struct Failure {
    pub code: i32,
    pub msg: String,
}

pub type CmdResult = Result<i32, Failure>;

pub fn usage(msg: impl Into<String>) -> Failure {
    Failure {
        code: EXIT_USAGE,
        msg: msg.into(),
    }
}

pub fn fs_error(context: impl std::fmt::Display, err: &std::io::Error) -> Failure {
    Failure {
        code: EXIT_FS,
        msg: format!("{context}: {err}"),
    }
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
            None => Err(usage(format!("--{flag} needs a value"))),
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
    let n: f64 = num
        .parse()
        .map_err(|_| usage(format!("bad duration '{text}' (use 500ms, 30s, 2m, 2h)")))?;
    let ms = match unit {
        "ms" => n,
        "" | "s" => n * 1000.0,
        "m" => n * 60_000.0,
        "h" => n * 3_600_000.0,
        "d" => n * 86_400_000.0,
        _ => {
            return Err(usage(format!(
                "bad duration '{text}' (use 500ms, 30s, 2m, 2h)"
            )))
        }
    };
    Ok(Duration::from_millis(ms as u64))
}

pub fn print_json(value: &serde_json::Value) {
    println!("{value}");
}

/// Runs the command line and returns the process exit code.
pub fn main(args: Vec<String>) -> i32 {
    let json = args.iter().any(|a| a == "--json");
    match dispatch(args) {
        Ok(code) => code,
        Err(f) => {
            if json {
                print_json(&serde_json::json!({ "result": "error", "error": f.msg }));
            } else {
                eprintln!("jlock: {}", f.msg);
            }
            f.code
        }
    }
}

fn dispatch(args: Vec<String>) -> CmdResult {
    let mut iter = args.into_iter();
    let Some(cmd) = iter.next() else {
        print!("{HELP}");
        return Err(usage("missing command"));
    };
    let rest: Vec<String> = iter.collect();
    match cmd.as_str() {
        "--version" | "-V" | "version" => {
            println!("jlock {}", env!("CARGO_PKG_VERSION"));
            Ok(EXIT_OK)
        }
        "--help" | "-h" | "help" => {
            print!("{HELP}");
            Ok(EXIT_OK)
        }
        "status" => status(rest),
        "acquire" => crate::cmd::acquire::run(rest),
        "release" => crate::cmd::release::run(rest),
        "run" | "hold" | "heartbeat" | "list" | "graph" | "unlock" | "gc" => {
            Err(usage(format!("{cmd}: not implemented yet")))
        }
        other => Err(usage(format!(
            "unknown command '{other}' (see jlock --help)"
        ))),
    }
}

fn status(args: Vec<String>) -> CmdResult {
    let mut p = Parser::new(args);
    let mut json = false;
    let mut input = None;
    while let Some(arg) = p.next() {
        match arg {
            Arg::Flag(f) if f == "json" => json = true,
            Arg::Pos(k) if input.is_none() => input = Some(k),
            Arg::Pos(k) => return Err(usage(format!("status takes one key (extra '{k}')"))),
            Arg::Flag(f) => return Err(usage(format!("status: unknown option --{f}"))),
            Arg::Rest(_) => return Err(usage("status: unexpected '--'")),
        }
    }
    let input = input.ok_or_else(|| usage("status needs a key"))?;
    let cwd =
        std::env::current_dir().map_err(|e| fs_error("cannot read the current directory", &e))?;
    let key = crate::key::normalize(&input, &cwd).map_err(usage)?;
    let root = crate::home::root().map_err(usage)?;
    let locks = crate::home::locks_dir(&root).map_err(|e| fs_error(root.display(), &e))?;
    let held = locks.join(&key.hash).join("holder.json").exists();
    let state = if held { "held" } else { "free" };
    if json {
        print_json(&serde_json::json!({ "result": "ok", "key": key.text, "state": state }));
    } else {
        println!("{state} {}", key.text);
    }
    Ok(EXIT_OK)
}
