//! `custody start` and `custody status`: a command that keeps running after the call returns, in a
//! named job, with its intent, receipt, output and final record. The commands are only ever this
//! test binary (`custody_helper`), every one bounded in time, and each test waits for its keeper to
//! be gone before it ends.

mod common;

use common::Env;
#[cfg(windows)]
use serde_json::Value;
#[cfg(windows)]
use std::path::{Path, PathBuf};
use std::time::Duration;
#[cfg(windows)]
use std::time::Instant;

/// The command under custody. `run`: leaves one file per execution in `CUSTODY_RUNS`, writes to its
/// output and error, waits for `CUSTODY_RELEASE` (at most 20 s) and exits with `CUSTODY_EXIT`.
/// `parent`: starts a `leaf` and exits as soon as it is up. `leaf`: writes `CUSTODY_READY`, and
/// `CUSTODY_LATE` 1.5 s later (a file that must never appear when the leaf is ended in time).
#[test]
fn custody_helper() {
    let Ok(mode) = std::env::var("CUSTODY_MODE") else {
        return;
    };
    let var = |name: &str| std::env::var(name).unwrap();
    match mode.as_str() {
        "run" => {
            let runs = std::path::PathBuf::from(var("CUSTODY_RUNS"));
            std::fs::create_dir_all(&runs).unwrap();
            std::fs::write(runs.join(std::process::id().to_string()), "run").unwrap();
            println!("helper-out");
            eprintln!("helper-err");
            let release = std::path::PathBuf::from(var("CUSTODY_RELEASE"));
            let start = std::time::Instant::now();
            while !release.exists() && start.elapsed() < Duration::from_secs(20) {
                std::thread::sleep(Duration::from_millis(10));
            }
            std::process::exit(var("CUSTODY_EXIT").parse().unwrap());
        }
        "parent" => {
            let ready = std::path::PathBuf::from(var("CUSTODY_READY"));
            let _leaf = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["custody_helper", "--exact", "--nocapture"])
                .env("CUSTODY_MODE", "leaf")
                .spawn()
                .unwrap();
            let start = std::time::Instant::now();
            while !ready.exists() && start.elapsed() < Duration::from_secs(5) {
                std::thread::sleep(Duration::from_millis(10));
            }
            std::process::exit(0);
        }
        "leaf" => {
            std::fs::write(var("CUSTODY_READY"), "ready").unwrap();
            std::thread::sleep(Duration::from_millis(1500));
            std::fs::write(var("CUSTODY_LATE"), "escaped").unwrap();
            std::thread::sleep(Duration::from_millis(500));
        }
        other => panic!("unknown CUSTODY_MODE {other}"),
    }
}

#[cfg(windows)]
struct Case {
    env: Env,
    dir: PathBuf,
}

#[cfg(windows)]
impl Case {
    fn new(name: &str) -> Case {
        let mut env = Env::new(name);
        env.set("JLOCK_CUSTODY_STOP_MS", "5000");
        let home = env.home.clone();
        let p = |n: &str| home.join(n).to_string_lossy().into_owned();
        env.set("CUSTODY_RUNS", &p("runs"));
        env.set("CUSTODY_RELEASE", &p("release"));
        env.set("CUSTODY_READY", &p("ready"));
        env.set("CUSTODY_LATE", &p("late"));
        env.set("CUSTODY_EXIT", "7");
        let dir = env.home.join("custody");
        Case { env, dir }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.env.home.join(name)
    }

    fn start(&self, id: &str, mode: &str) -> common::Done {
        let me = std::env::current_exe().unwrap();
        let mut cmd = self.env.cmd(&[
            "custody",
            "start",
            "--id",
            id,
            "--custody",
            self.dir.to_str().unwrap(),
            "--json",
            "--",
            me.to_str().unwrap(),
            "custody_helper",
            "--exact",
            "--nocapture",
        ]);
        cmd.env("CUSTODY_MODE", mode);
        common::Kid::start(cmd).finish(Duration::from_secs(30))
    }

    fn status(&self, id: &str) -> Value {
        let done = self.env.run(&[
            "custody",
            "status",
            "--id",
            id,
            "--custody",
            self.dir.to_str().unwrap(),
            "--json",
        ]);
        assert_eq!(done.code, 0, "{}", done.stderr);
        done.json()
    }

    /// Executions of the helper so far (one file each).
    fn runs(&self) -> usize {
        std::fs::read_dir(self.path("runs"))
            .map(|d| d.count())
            .unwrap_or(0)
    }

    /// Waits for `ended`, then for the keeper to be gone; returns `final.json`.
    fn ended(&self, id: &str) -> Value {
        let mut last = Value::Null;
        let ok = self.env.wait_for(Duration::from_secs(15), || {
            last = self.status(id);
            last["state"] == "ended"
        });
        assert!(ok, "never ended: {last}");
        let receipt = &last["receipt"];
        let pid = receipt["custodianPid"].as_u64().unwrap().to_string();
        let identity = receipt["custodianIdentity"].as_str().unwrap().to_string();
        let gone = self.env.wait_for(Duration::from_secs(10), || {
            let v = self
                .env
                .run(&[
                    "process",
                    "inspect",
                    "--pid",
                    &pid,
                    "--identity",
                    &identity,
                    "--json",
                ])
                .json();
            v["state"] == "absent"
        });
        assert!(gone, "the custodian is still alive after final.json");
        last["final"].clone()
    }

    fn record(&self, id: &str, name: &str) -> PathBuf {
        self.dir.join(id).join(name)
    }
}

#[cfg(windows)]
fn read(path: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[cfg(windows)]
#[test]
fn start_returns_with_a_receipt_and_the_end_is_recorded() {
    let c = Case::new("custody-start");
    let started = Instant::now();
    let done = c.start("a1", "run");
    let took = started.elapsed();
    assert_eq!(done.code, 0, "{}{}", done.stdout, done.stderr);
    assert!(took < Duration::from_secs(5), "start took {took:?}");
    let v = done.json();
    assert_eq!(v["state"], "running", "{v}");
    assert_eq!(v["id"], "a1");
    let receipt = &v["receipt"];
    assert!(receipt["pid"].as_u64().is_some(), "{v}");
    assert!(receipt["identity"].as_str().unwrap().starts_with("win:"));
    assert!(receipt["job"]
        .as_str()
        .unwrap()
        .starts_with("Local\\jlock-custody-"));
    assert!(receipt["job"].as_str().unwrap().ends_with("-a1"));
    assert_eq!(receipt["scope"], "job");
    assert!(receipt["custodianPid"].as_u64().is_some(), "{v}");
    assert_ne!(receipt["custodianPid"], receipt["pid"]);
    assert!(c.record("a1", "intent.json").exists());
    assert_eq!(&read(&c.record("a1", "receipt.json")), receipt);
    let intent = read(&c.record("a1", "intent.json"));
    assert_eq!(intent["mode"], "detached");
    assert_eq!(intent["v"], 1);

    let st = c.status("a1");
    assert_eq!(st["state"], "running", "{st}");
    assert_eq!(st["job"]["open"], true, "{st}");
    assert!(st["job"]["activeProcesses"].as_u64().unwrap() >= 1, "{st}");
    assert_eq!(&st["receipt"], receipt);

    assert!(c.env.wait_for(Duration::from_secs(10), || c.runs() == 1));
    std::fs::write(c.path("release"), "go").unwrap();
    let fin = c.ended("a1");
    assert_eq!(fin["reason"], "exited", "{fin}");
    assert_eq!(fin["exitCode"], 7, "{fin}");
    assert_eq!(fin["cleanupConfirmed"], true, "{fin}");
    assert_eq!(fin["activeProcesses"], 0, "{fin}");
    assert_eq!(fin["scope"], "job");
    assert_eq!(fin["by"], "custodian");
    assert_eq!(read(&c.record("a1", "final.json")), fin);
    let log = std::fs::read_to_string(c.record("a1", "output.log")).unwrap();
    assert!(log.contains("helper-out"), "{log}");
    assert!(log.contains("helper-err"), "{log}");
    assert_eq!(c.status("a1")["job"]["open"], false);
}

#[cfg(windows)]
#[test]
fn the_same_id_never_starts_a_second_process() {
    let c = Case::new("custody-duplicate");
    assert_eq!(c.start("a2", "run").json()["state"], "running");
    let again = c.start("a2", "run");
    assert_eq!(again.code, 0, "{}", again.stderr);
    let v = again.json();
    assert_eq!(v["duplicate"], true, "{v}");
    assert_eq!(v["state"], "running", "{v}");

    // Two at the same time: exactly one starts.
    let me = std::env::current_exe().unwrap();
    let kids: Vec<_> = (0..2)
        .map(|_| {
            let mut cmd = c.env.cmd(&[
                "custody",
                "start",
                "--id",
                "a3",
                "--custody",
                c.dir.to_str().unwrap(),
                "--json",
                "--",
                me.to_str().unwrap(),
                "custody_helper",
                "--exact",
                "--nocapture",
            ]);
            cmd.env("CUSTODY_MODE", "run");
            common::Kid::start(cmd)
        })
        .collect();
    let answers: Vec<Value> = kids
        .into_iter()
        .map(|k| k.finish(Duration::from_secs(30)).json())
        .collect();
    let fresh = answers.iter().filter(|a| a["duplicate"] != true).count();
    assert_eq!(fresh, 1, "{answers:?}");

    assert!(c.env.wait_for(Duration::from_secs(10), || c.runs() >= 2));
    std::thread::sleep(Duration::from_millis(700));
    assert_eq!(c.runs(), 2, "one process for a2 and one for a3");
    std::fs::write(c.path("release"), "go").unwrap();
    assert_eq!(c.ended("a2")["cleanupConfirmed"], true);
    assert_eq!(c.ended("a3")["cleanupConfirmed"], true);
    assert_eq!(c.runs(), 2);
}

#[cfg(windows)]
#[test]
fn an_intent_without_receipt_or_custodian_is_launch_failed() {
    let c = Case::new("custody-launch-failed");
    assert_eq!(c.status("b1")["state"], "absent");
    std::fs::create_dir_all(c.dir.join("b1")).unwrap();
    let intent = serde_json::json!({
        "v": 1, "id": "b1", "argv": ["x"], "cwd": ".", "mode": "detached",
        "createdAt": "2026-11-14T00:00:00.000Z",
    });
    std::fs::write(c.record("b1", "intent.json"), intent.to_string()).unwrap();
    let st = c.status("b1");
    assert_eq!(st["state"], "launch-failed", "{st}");
    assert_eq!(st["job"]["open"], false, "{st}");
    let again = c.start("b1", "run");
    assert_eq!(again.code, 0, "{}", again.stderr);
    let v = again.json();
    assert_eq!(v["duplicate"], true, "{v}");
    assert_eq!(v["state"], "launch-failed", "{v}");
    std::thread::sleep(Duration::from_millis(700));
    assert_eq!(c.runs(), 0, "a duplicate start ran the command");
    assert!(!c.record("b1", "receipt.json").exists());
}

#[cfg(windows)]
#[test]
fn a_grandchild_left_behind_is_ended_before_final() {
    let c = Case::new("custody-grandchild");
    assert_eq!(c.start("g1", "parent").json()["state"], "running");
    let fin = c.ended("g1");
    assert!(c.path("ready").exists(), "the grandchild never started");
    assert!(
        !c.path("late").exists(),
        "the grandchild outlived final.json"
    );
    assert_eq!(fin["reason"], "exited");
    assert_eq!(fin["exitCode"], 0);
    assert_eq!(fin["cleanupConfirmed"], true, "{fin}");
    assert_eq!(fin["activeProcesses"], 0);
    std::thread::sleep(Duration::from_millis(2000));
    assert!(
        !c.path("late").exists(),
        "the grandchild wrote after the end"
    );
}

#[cfg(windows)]
#[test]
fn a_command_that_cannot_run_is_launch_failed() {
    let c = Case::new("custody-cannot-run");
    let missing = c.path("no-such-program.exe");
    let done = c.env.run(&[
        "custody",
        "start",
        "--id",
        "c1",
        "--custody",
        c.dir.to_str().unwrap(),
        "--json",
        "--",
        missing.to_str().unwrap(),
    ]);
    assert_ne!(done.code, 0, "{}", done.stdout);
    let v = done.json();
    assert_eq!(v["state"], "launch-failed", "{v}");
    assert!(v["reason"].as_str().unwrap().contains("cannot run"), "{v}");
    let st = c.status("c1");
    assert!(c
        .env
        .wait_for(Duration::from_secs(5), || c.status("c1")["state"]
            == "launch-failed"));
    assert!(st["receipt"].is_null(), "{st}");
}

#[cfg(windows)]
#[test]
fn usage_errors() {
    let c = Case::new("custody-usage");
    let dir = c.dir.to_str().unwrap();
    for (args, want) in [
        (vec!["custody"], "needs an action"),
        (
            vec!["custody", "go", "--id", "a", "--custody", dir],
            "unknown action",
        ),
        (vec!["custody", "status", "--custody", dir], "needs --id"),
        (
            vec!["custody", "status", "--id", "a/b", "--custody", dir],
            "bad custody id",
        ),
        (
            vec!["custody", "status", "--id", "..", "--custody", dir],
            "bad custody id",
        ),
        (vec!["custody", "status", "--id", "a"], "needs --custody"),
        (
            vec!["custody", "start", "--id", "a", "--custody", dir],
            "needs a command",
        ),
    ] {
        let done = c.env.run(&args);
        assert_eq!(done.code, 2, "{args:?}: {}", done.stderr);
        assert!(done.stderr.contains(want), "{args:?}: {}", done.stderr);
    }
    assert!(!c.dir.join("a").exists(), "a usage error wrote records");
}

#[cfg(not(windows))]
#[test]
fn custody_is_unsupported_elsewhere() {
    let env = Env::new("custody-unsupported");
    let dir = env.home.join("custody");
    let done = env.run(&[
        "custody",
        "status",
        "--id",
        "a1",
        "--custody",
        dir.to_str().unwrap(),
    ]);
    assert_eq!(done.code, 2);
    assert!(
        done.stderr.contains("unsupported on this platform"),
        "{}",
        done.stderr
    );
}
