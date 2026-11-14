//! `jlock custody start` and `jlock custody status`: a command that keeps running after the call
//! returns, in a named job, with durable records (`crate::custody`).
//!
//! `start` (the first process): takes the launch lock, writes `intent.json` (an id that already has
//! one never starts a second process: the answer is its state, with `duplicate: true`), starts the
//! keeper — this same binary, detached like the keeper of `hold --detach` — and answers once the
//! keeper has written `receipt.json`.
//!
//! The keeper: creates the named job (kill-on-close, no breakaway, a handle nobody inherits),
//! creates the command suspended with its output in `output.log`, puts it in the job, writes
//! `receipt.json`, lets it run, tells the first process, and stays out of the job. When the command
//! ends it ends whatever is left in the job, confirms that nothing is, and writes `final.json`.
#![cfg_attr(not(windows), allow(dead_code, unused_imports))]

use crate::cli::{
    fs_error, parse_duration, print_json, unexpected_rest, unknown_option, usage, Arg, CmdResult,
    Failure, Parser, EXIT_FS, EXIT_OK,
};
use crate::custody::{self, KEEPER_ENV};
use crate::i18n::{txt, Text};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// How long the first process waits for the keeper's receipt before giving up on it.
const LAUNCH_TIMEOUT: Duration = Duration::from_secs(30);

struct Opts {
    action: String,
    id: String,
    dir: PathBuf,
    cwd: Option<String>,
    limit: Option<Duration>,
    quiet: Option<Duration>,
    json: bool,
    argv: Vec<String>,
}

fn parse(args: Vec<String>) -> Result<Opts, Failure> {
    let mut p = Parser::new(args);
    let mut action: Option<String> = None;
    let (mut id, mut dir, mut cwd) = (None, None, None);
    let (mut limit, mut quiet) = (None, None);
    let mut json = false;
    let mut argv = Vec::new();
    let mut rest = false;
    while let Some(arg) = p.next() {
        match arg {
            Arg::Flag(f) if f == "json" => json = true,
            Arg::Flag(f) if f == "id" => id = Some(p.value("id")?),
            Arg::Flag(f) if f == "custody" => dir = Some(p.value("custody")?),
            Arg::Flag(f) if f == "cwd" => cwd = Some(p.value("cwd")?),
            Arg::Flag(f) if f == "limit" => limit = Some(parse_duration(&p.value("limit")?)?),
            Arg::Flag(f) if f == "quiet" => quiet = Some(parse_duration(&p.value("quiet")?)?),
            Arg::Flag(f) => return Err(unknown_option("custody", &f)),
            Arg::Pos(a) if action.is_none() => action = Some(a),
            Arg::Pos(a) => return Err(usage(txt!("error.custody_extra", arg = a))),
            Arg::Rest(r) => {
                argv = r;
                rest = true;
            }
        }
    }
    let action = match action.as_deref() {
        Some(a @ ("start" | "status")) => a.to_string(),
        Some(other) => return Err(usage(txt!("error.custody_action", action = other))),
        None => return Err(usage(txt!("error.custody_needs_action"))),
    };
    let Some(id) = id else {
        return Err(usage(txt!("error.custody_needs_id", action = action)));
    };
    if !custody::valid_id(&id) {
        return Err(usage(txt!("error.custody_bad_id", value = id)));
    }
    let Some(dir) = dir.filter(|d| !d.trim().is_empty()) else {
        return Err(usage(txt!("error.custody_needs_dir", action = action)));
    };
    let dir = custody::absolute(Path::new(&dir))
        .map_err(|e| fs_error(txt!("error.custody_records"), &e))?;
    if action == "start" && argv.is_empty() {
        return Err(usage(txt!("error.custody_needs_command")));
    }
    if action == "status" && rest {
        return Err(unexpected_rest("custody status"));
    }
    Ok(Opts {
        action,
        id,
        dir,
        cwd,
        limit,
        quiet,
        json,
        argv,
    })
}

pub fn run(args: Vec<String>) -> CmdResult {
    if !cfg!(windows) {
        return Err(usage(txt!("error.unsupported_platform", cmd = "custody")));
    }
    let o = parse(args)?;
    match o.action.as_str() {
        "start" if std::env::var_os(KEEPER_ENV).is_some() => keeper(&o),
        "start" => start(&o),
        _ => {
            show(&custody::status(&o.dir, &o.id), o.json);
            Ok(EXIT_OK)
        }
    }
}

/// Prints a state answer: the JSON as is, or one `field: value` line per main field.
fn show(v: &Value, json: bool) {
    if json {
        print_json(v);
        return;
    }
    println!("state: {}", v["state"].as_str().unwrap_or(""));
    if v["duplicate"] == true {
        println!("duplicate: true");
    }
    if let Some(r) = v["reason"].as_str() {
        println!("reason: {r}");
    }
    let receipt = if v["receipt"].is_object() {
        &v["receipt"]
    } else {
        &Value::Null
    };
    if let Some(pid) = receipt["pid"].as_u64() {
        println!("pid: {pid}");
    }
    if let Some(job) = receipt["job"].as_str() {
        println!("job: {job}");
    }
    if v["job"].is_object() {
        println!("job open: {}", v["job"]["open"]);
        if let Some(n) = v["job"]["activeProcesses"].as_u64() {
            println!("active processes: {n}");
        }
    }
    let fin = &v["final"];
    if let Some(reason) = fin["reason"].as_str() {
        println!("ended: {reason}");
    }
    if let Some(code) = fin["exitCode"].as_i64() {
        println!("exit code: {code}");
    }
    if let Some(c) = fin["cleanupConfirmed"].as_bool() {
        println!("cleanup confirmed: {c}");
    }
}

fn records_error(e: &std::io::Error) -> Failure {
    fs_error(txt!("error.custody_records"), e)
}

fn duplicate(o: &Opts) -> CmdResult {
    let mut v = custody::status(&o.dir, &o.id);
    v["duplicate"] = json!(true);
    show(&v, o.json);
    Ok(EXIT_OK)
}

fn launch_failed(o: &Opts, reason: &str) -> CmdResult {
    show(
        &json!({ "state": "launch-failed", "id": o.id, "reason": reason }),
        o.json,
    );
    Ok(EXIT_FS)
}

/// The first process.
#[cfg(windows)]
fn start(o: &Opts) -> CmdResult {
    use crate::oslock::{try_lock, Try};
    use std::process::{Command, Stdio};
    use std::time::Instant;

    let cwd = match &o.cwd {
        Some(c) => custody::absolute(Path::new(c)).map_err(|e| records_error(&e))?,
        None => std::env::current_dir().map_err(|e| records_error(&e))?,
    };
    if !cwd.is_dir() {
        return Err(usage(txt!("error.custody_bad_cwd", path = cwd.display())));
    }
    let rec = o.dir.join(&o.id);
    crate::fsx::ensure_dir(&rec).map_err(|e| records_error(&e))?;
    let lock_path = rec.join(custody::LAUNCH_LOCK);
    crate::fsx::retry(&lock_path, || {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&lock_path)
    })
    .map_err(|e| records_error(&e))?;
    let intent_path = rec.join(custody::INTENT);
    // Held from before the intent until the answer: `status` reads `launching` meanwhile.
    let deadline = Instant::now() + Duration::from_secs(10);
    let launch = loop {
        match try_lock(&lock_path).map_err(|e| records_error(&e))? {
            Try::Locked(l) => break Some(l),
            Try::Unsupported => break None,
            Try::Busy if intent_path.exists() => return duplicate(o),
            Try::Busy if Instant::now() >= deadline => {
                return Err(Failure {
                    code: EXIT_FS,
                    msg: txt!("error.custody_busy", id = o.id),
                });
            }
            Try::Busy => std::thread::sleep(Duration::from_millis(10)),
        }
    };
    let mut intent = json!({
        "v": 1,
        "id": o.id,
        "argv": o.argv,
        "cwd": cwd.to_string_lossy(),
        "mode": "detached",
        "createdAt": custody::now(),
    });
    if let Some(l) = o.limit {
        intent["limitMs"] = json!(l.as_millis() as u64);
    }
    if let Some(q) = o.quiet {
        intent["quietMs"] = json!(q.as_millis() as u64);
    }
    match crate::fsx::create_exclusive(&intent_path, format!("{intent}\n").as_bytes()) {
        Ok(()) => {}
        Err(e) if crate::fsx::is_exists(&e) => {
            drop(launch); // the answer must not see our own lock as a launch in progress
            return duplicate(o);
        }
        Err(e) => return Err(records_error(&e)),
    }

    let exe =
        std::env::current_exe().map_err(|e| crate::cli::fs_error(txt!("update.no_exe"), &e))?;
    let mut cmd = Command::new(exe);
    cmd.args(["custody", "start", "--id", &o.id, "--custody"])
        .arg(&o.dir)
        .arg("--")
        .args(&o.argv)
        .current_dir(&cwd)
        .env(KEEPER_ENV, "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = match crate::detach::spawn(&mut cmd) {
        Ok(c) => c,
        Err(e) => return launch_failed(o, &format!("cannot start the custodian: {e}")),
    };
    // Our handle keeps the pid from being reused until this identity is recorded.
    if let Some(identity) = crate::process_identity::inspect(child.id(), None).identity() {
        let record = json!({ "pid": child.id(), "identity": identity });
        let _ = custody::publish_once(&rec.join(custody::CUSTODIAN), &record);
    }
    let (out, err) = (reader(child.stdout.take()), reader(child.stderr.take()));
    let deadline = Instant::now() + LAUNCH_TIMEOUT;
    while !out.is_finished() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    if !out.is_finished() {
        // Our own child, before its receipt: the job it may have made closes with it.
        let _ = child.kill();
        let _ = child.wait();
        return launch_failed(o, "the custodian did not answer in time");
    }
    let ready = out
        .join()
        .unwrap_or_default()
        .lines()
        .any(|l| l == crate::detach::READY);
    if ready {
        if let Some(receipt) = custody::read_json(&rec.join(custody::RECEIPT)) {
            let answer = json!({ "state": "running", "id": o.id, "receipt": receipt });
            show(&answer, o.json);
            return Ok(EXIT_OK);
        }
        return launch_failed(o, "the custodian answered without a receipt");
    }
    let _ = child.wait();
    let text = err.join().unwrap_or_default();
    let reason = text
        .lines()
        .map(|l| l.trim().trim_start_matches("jlock: ").trim())
        .rfind(|l| !l.is_empty())
        .unwrap_or("the custodian ended before the receipt")
        .to_string();
    launch_failed(o, &reason)
}

#[cfg(windows)]
fn reader<R: std::io::Read + Send + 'static>(pipe: Option<R>) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut text = String::new();
        if let Some(mut p) = pipe {
            let _ = p.read_to_string(&mut text);
        }
        text
    })
}

/// How long the keeper waits for the job to be empty after ending it.
fn stop_wait() -> Duration {
    let ms = std::env::var("JLOCK_CUSTODY_STOP_MS")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(15_000);
    Duration::from_millis(ms)
}

fn keeper_error(text: String) -> Failure {
    Failure {
        code: EXIT_FS,
        msg: Text::plain(&text),
    }
}

/// The keeper: the detached `jlock` that owns the job of one command.
#[cfg(windows)]
fn keeper(o: &Opts) -> CmdResult {
    use crate::custody::Job;
    use crate::process_identity::inspect;
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    use windows_sys::Win32::System::Threading::{CREATE_NO_WINDOW, CREATE_SUSPENDED};

    let rec = o.dir.join(&o.id);
    let me = std::process::id();
    let my_identity = inspect(me, None).identity().map(str::to_string);
    if let Some(identity) = &my_identity {
        let _ = custody::publish_once(
            &rec.join(custody::CUSTODIAN),
            &json!({ "pid": me, "identity": identity }),
        );
    }
    let intent = custody::read_json(&rec.join(custody::INTENT))
        .ok_or_else(|| keeper_error("cannot read intent.json".into()))?;
    let argv: Vec<String> = intent["argv"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|s| s.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let cwd = intent["cwd"].as_str().unwrap_or(".").to_string();
    if argv.is_empty() {
        return Err(keeper_error("intent.json has no command".into()));
    }
    let name = custody::job_name(&o.dir, &o.id);
    let job = Job::create(&name).map_err(|e| keeper_error(format!("cannot create job: {e}")))?;
    let log_path = rec.join(custody::OUTPUT);
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|e| keeper_error(format!("cannot open output.log: {e}")))?;
    let log_err = log
        .try_clone()
        .map_err(|e| keeper_error(format!("cannot open output.log: {e}")))?;
    // The command must not inherit the pipes to the first process (it would wait for them).
    crate::detach::private_std_handles();
    let mut child = Command::new(&argv[0])
        .args(&argv[1..])
        .current_dir(&cwd)
        .env_remove(KEEPER_ENV)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log_err))
        .creation_flags(CREATE_SUSPENDED | CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| keeper_error(format!("cannot run {}: {e}", argv[0])))?;
    if let Err(e) = job.assign(&child) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(keeper_error(format!(
            "cannot put the command in its job: {e}"
        )));
    }
    // Suspended: it cannot have exited, so this identity is the command's own.
    let Some(identity) = inspect(child.id(), None).identity().map(str::to_string) else {
        let _ = job.terminate(stop_wait());
        let _ = child.wait();
        return Err(keeper_error("cannot read the command's identity".into()));
    };
    let receipt = json!({
        "v": 1,
        "id": o.id,
        "pid": child.id(),
        "identity": identity,
        "job": name,
        "scope": "job",
        "custodianPid": me,
        "custodianIdentity": my_identity,
        "startedAt": custody::now(),
    });
    if let Err(e) = custody::publish_once(&rec.join(custody::RECEIPT), &receipt) {
        let _ = job.terminate(stop_wait());
        let _ = child.wait();
        return Err(keeper_error(format!("cannot write receipt.json: {e}")));
    }
    if let Err(e) = crate::process_tree::resume(&child) {
        let left = job.terminate(stop_wait());
        let _ = child.wait();
        write_final(o, &rec, "interrupted", None, left);
        return Err(keeper_error(format!("cannot start the command: {e}")));
    }
    crate::detach::ready();
    let code = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code(),
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(_) => break None,
        }
    };
    // Whatever the command left behind in its job ends before the end is recorded.
    let left = job.terminate(stop_wait());
    write_final(o, &rec, "exited", code, left);
    Ok(EXIT_OK)
}

#[cfg(windows)]
fn write_final(o: &Opts, rec: &Path, reason: &str, code: Option<i32>, left: std::io::Result<u32>) {
    let mut fin = json!({
        "v": 1,
        "id": o.id,
        "reason": reason,
        "cleanupConfirmed": matches!(left, Ok(0)),
        "scope": "job",
        "activeProcesses": left.as_ref().ok(),
        "endedAt": custody::now(),
        "by": "custodian",
    });
    if let Some(c) = code {
        fin["exitCode"] = json!(c);
    }
    // `custody stop` may have recorded the end first; its record stays.
    let _ = custody::publish_once(&rec.join(custody::FINAL), &fin);
}

#[cfg(not(windows))]
fn start(_o: &Opts) -> CmdResult {
    Err(usage(txt!("error.unsupported_platform", cmd = "custody")))
}

#[cfg(not(windows))]
fn keeper(_o: &Opts) -> CmdResult {
    Err(usage(txt!("error.unsupported_platform", cmd = "custody")))
}
