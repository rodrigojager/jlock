//! `exec` and `run` with `--id` and `--custody`: the foreground command leaves the records of
//! `custody` (`mode: "foreground"`, no `output.log`), runs in a named job that `custody status`
//! reads and `custody stop` ends, and still passes its output and exit code through. The commands
//! are only ever this test binary (`fg_helper`), every one bounded in time.

mod common;

use common::Env;
#[cfg(windows)]
use serde_json::Value;
use std::path::PathBuf;
use std::time::Duration;

/// The helpers, by `FG_MODE`. `echo`: prints `hello-from-child` and exits 7. `sleep`: writes
/// `FG_STARTED` (when set), then waits (at most 30 s).
#[test]
fn fg_helper() {
    let Ok(mode) = std::env::var("FG_MODE") else {
        return;
    };
    match mode.as_str() {
        "echo" => {
            println!("hello-from-child");
            std::process::exit(7);
        }
        "sleep" => {
            if let Ok(path) = std::env::var("FG_STARTED") {
                std::fs::write(path, "started").unwrap();
            }
            let start = std::time::Instant::now();
            while start.elapsed() < Duration::from_secs(30) {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        other => panic!("unknown FG_MODE {other}"),
    }
}

fn helper_args() -> Vec<String> {
    let me = std::env::current_exe().unwrap();
    vec![
        me.to_string_lossy().into_owned(),
        "fg_helper".into(),
        "--exact".into(),
        "--nocapture".into(),
    ]
}

struct Case {
    env: Env,
    dir: PathBuf,
}

impl Case {
    fn new(name: &str) -> Case {
        let mut env = Env::new(name);
        env.set("JLOCK_CUSTODY_STOP_MS", "5000");
        env.set("JLOCK_CUSTODY_BEAT_MS", "200");
        let started = env.home.join("started").to_string_lossy().into_owned();
        env.set("FG_STARTED", &started);
        let dir = env.home.join("custody");
        Case { env, dir }
    }

    fn dir(&self) -> &str {
        self.dir.to_str().unwrap()
    }

    /// `jlock <before...> -- <helper>` with `FG_MODE=mode`.
    fn cmd(&self, before: &[&str], mode: &str) -> std::process::Command {
        let helper = helper_args();
        let mut args: Vec<&str> = before.to_vec();
        args.push("--");
        args.extend(helper.iter().map(String::as_str));
        let mut cmd = self.env.cmd(&args);
        cmd.env("FG_MODE", mode);
        cmd
    }

    fn run(&self, before: &[&str], mode: &str) -> common::Done {
        common::Kid::start(self.cmd(before, mode)).finish(Duration::from_secs(60))
    }

    #[cfg(windows)]
    fn read(&self, id: &str, name: &str) -> Value {
        let path = self.dir.join(id).join(name);
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap())
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    #[cfg(windows)]
    fn custody(&self, action: &str, id: &str, extra: &[&str]) -> (i32, Value) {
        let mut args = vec![action, "--id", id, "--custody", self.dir(), "--json"];
        args.insert(0, "custody");
        args.extend_from_slice(extra);
        let done = self.env.run(&args);
        (done.code, done.json())
    }

    #[cfg(windows)]
    fn absent(&self, pid: &Value, identity: &Value) -> bool {
        let pid = pid.as_u64().unwrap().to_string();
        let done = self.env.run(&[
            "process",
            "inspect",
            "--pid",
            &pid,
            "--identity",
            identity.as_str().unwrap(),
            "--json",
        ]);
        done.json()["state"] == "absent"
    }

    /// The records of a command that ended by itself with `exit_code`.
    #[cfg(windows)]
    fn check_exited(&self, id: &str, exit_code: i64) {
        let intent = self.read(id, "intent.json");
        assert_eq!(intent["mode"], "foreground", "{intent}");
        assert_eq!(intent["id"], id, "{intent}");
        assert!(intent["argv"].is_array(), "{intent}");
        let receipt = self.read(id, "receipt.json");
        assert_eq!(receipt["scope"], "job", "{receipt}");
        assert!(
            receipt["job"]
                .as_str()
                .unwrap()
                .starts_with("Local\\jlock-custody-"),
            "{receipt}"
        );
        assert!(receipt["identity"].is_string(), "{receipt}");
        assert!(receipt["custodianIdentity"].is_string(), "{receipt}");
        let fin = self.read(id, "final.json");
        assert_eq!(fin["reason"], "exited", "{fin}");
        assert_eq!(fin["cleanupConfirmed"], true, "{fin}");
        assert_eq!(fin["exitCode"], exit_code, "{fin}");
        assert_eq!(fin["activeProcesses"], 0, "{fin}");
        assert_eq!(fin["by"], "custodian", "{fin}");
        let beat = self.read(id, "beat.json");
        assert!(beat["seq"].as_u64().unwrap() >= 1, "{beat}");
        assert!(
            !self.dir.join(id).join("output.log").exists(),
            "a foreground command has no output.log"
        );
        assert!(self.absent(&receipt["pid"], &receipt["identity"]));
        let (code, st) = self.custody("status", id, &[]);
        assert_eq!(code, 0);
        assert_eq!(st["state"], "ended", "{st}");
        assert_eq!(st["job"]["open"], false, "{st}");
    }
}

#[cfg(windows)]
#[test]
fn exec_in_custody_passes_output_and_code_through_and_leaves_the_records() {
    let c = Case::new("fg-exec");
    let done = c.run(
        &[
            "exec",
            "--id",
            "f1",
            "--custody",
            c.dir(),
            "--timeout",
            "30s",
        ],
        "echo",
    );
    assert_eq!(done.code, 7, "{}{}", done.stdout, done.stderr);
    assert!(done.stdout.contains("hello-from-child"), "{}", done.stdout);
    c.check_exited("f1", 7);
}

#[cfg(windows)]
#[test]
fn a_running_exec_is_seen_by_status_and_ended_by_stop() {
    let c = Case::new("fg-exec-stop");
    let mut exec = common::Kid::start(c.cmd(
        &[
            "exec",
            "--id",
            "s1",
            "--custody",
            c.dir(),
            "--timeout",
            "30s",
        ],
        "sleep",
    ));
    let started = c.env.home.join("started");
    assert!(
        c.env.wait_for(Duration::from_secs(20), || started.exists()),
        "the command never started"
    );
    let (code, st) = c.custody("status", "s1", &[]);
    assert_eq!(code, 0);
    assert_eq!(st["state"], "running", "{st}");
    assert_eq!(st["job"]["open"], true, "{st}");
    assert_eq!(st["intent"]["mode"], "foreground", "{st}");
    let receipt = st["receipt"].clone();
    assert_eq!(receipt["custodianPid"], exec.pid, "{st}");

    let (code, v) = c.custody("stop", "s1", &["--reason", "test"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["cleanupConfirmed"], true, "{v}");
    assert_eq!(v["reason"], "stopped", "{v}");
    assert_eq!(v["by"], "stop", "{v}");
    let code = exec
        .wait(Duration::from_secs(20))
        .expect("exec did not end");
    assert_ne!(code, 0, "a stopped exec must not report success");
    let fin = c.read("s1", "final.json");
    assert_eq!(fin, v, "the exec overwrote the stop's record");
    assert!(c.absent(&receipt["pid"], &receipt["identity"]));
    assert_eq!(c.custody("status", "s1", &[]).1["state"], "ended");
}

#[cfg(windows)]
#[test]
fn run_in_custody_leaves_the_records_and_releases_the_key() {
    let c = Case::new("fg-run");
    let done = c.run(
        &[
            "run",
            "name:k",
            "--owner",
            "o",
            "--id",
            "f2",
            "--custody",
            c.dir(),
            "--total-timeout",
            "30s",
        ],
        "echo",
    );
    assert_eq!(done.code, 7, "{}{}", done.stdout, done.stderr);
    assert!(done.stdout.contains("hello-from-child"), "{}", done.stdout);
    c.check_exited("f2", 7);
    assert!(c.env.lock_dirs().is_empty(), "the key was not released");

    let done = c.run(
        &[
            "run",
            "name:k",
            "--owner",
            "o",
            "--id",
            "f3",
            "--custody",
            c.dir(),
            "--total-timeout",
            "1s",
        ],
        "sleep",
    );
    assert_eq!(done.code, 124, "{}{}", done.stdout, done.stderr);
    let fin = c.read("f3", "final.json");
    assert_eq!(fin["reason"], "limit", "{fin}");
    assert_eq!(fin["cleanupConfirmed"], true, "{fin}");
    assert_eq!(c.read("f3", "intent.json")["limitMs"], 1000);
    let receipt = c.read("f3", "receipt.json");
    assert!(c.absent(&receipt["pid"], &receipt["identity"]));
    assert!(c.env.lock_dirs().is_empty(), "the key was not released");
}

#[cfg(windows)]
#[test]
fn an_id_with_an_intent_is_refused_without_running_the_command() {
    let c = Case::new("fg-duplicate");
    let first = c.run(&["exec", "--id", "d1", "--custody", c.dir()], "echo");
    assert_eq!(first.code, 7, "{}", first.stderr);
    let fin = c.read("d1", "final.json");
    let receipt = c.read("d1", "receipt.json");
    for before in [
        vec!["exec", "--id", "d1", "--custody", c.dir()],
        vec![
            "run",
            "name:d",
            "--owner",
            "o",
            "--id",
            "d1",
            "--custody",
            c.dir(),
        ],
    ] {
        let again = c.run(&before, "echo");
        assert_eq!(again.code, 2, "{before:?}: {}", again.stderr);
        assert!(
            again.stderr.contains("already has an intent"),
            "{}",
            again.stderr
        );
        assert!(!again.stdout.contains("hello-from-child"), "it ran");
    }
    assert_eq!(c.read("d1", "final.json"), fin);
    assert_eq!(c.read("d1", "receipt.json"), receipt);
    assert!(c.env.lock_dirs().is_empty());
}

#[test]
fn only_one_of_id_and_custody_is_a_usage_error() {
    let c = Case::new("fg-usage");
    let dir = c.dir().to_string();
    for before in [
        vec!["exec", "--id", "u1"],
        vec!["exec", "--custody", &dir],
        vec!["run", "name:u", "--owner", "o", "--id", "u1"],
        vec!["run", "name:u", "--owner", "o", "--custody", &dir],
    ] {
        let done = c.run(&before, "echo");
        assert_eq!(done.code, 2, "{before:?}: {}", done.stderr);
        assert!(
            done.stderr.contains("--id and --custody go together"),
            "{}",
            done.stderr
        );
        assert!(!done.stdout.contains("hello-from-child"), "it ran");
    }
    let done = c.run(&["exec", "--id", "a b", "--custody", &dir], "echo");
    assert_eq!(done.code, 2, "{}", done.stderr);
    assert!(!c.dir.exists(), "a refused command wrote records");
}
