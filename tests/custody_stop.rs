//! `custody stop`: another process ends the command under custody by the job's name and confirms
//! that nothing is left, with or without its custodian; when it cannot prove that, it says so and
//! ends nothing by pid. The commands are only ever this test binary (`stop_helper`), every one
//! bounded in time; the only processes a test ends itself are its own (a custodian it started, a
//! helper it spawned), by PID and after checking their identity on the same handle.

mod common;

#[cfg(windows)]
use common::Env;
#[cfg(windows)]
use serde_json::{json, Value};
#[cfg(windows)]
use std::path::PathBuf;
use std::time::Duration;

/// The helpers, by `STOP_MODE`. `parent`: starts a `leaf`, then waits (at most 30 s). `leaf`:
/// writes its pid to `STOP_LEAF`, then waits (at most 30 s). `sleep`: waits (at most 30 s).
/// `hold-job`: opens the job `STOP_JOB` by name, writes `STOP_HELD`, and keeps the handle until
/// `STOP_RELEASE` exists (at most 30 s).
#[test]
fn stop_helper() {
    let Ok(mode) = std::env::var("STOP_MODE") else {
        return;
    };
    let var = |name: &str| std::env::var(name).unwrap();
    let wait = |until: Option<PathBuf>| {
        let start = std::time::Instant::now();
        while start.elapsed() < Duration::from_secs(30) {
            if until.as_ref().is_some_and(|p| p.exists()) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    };
    match mode.as_str() {
        "parent" => {
            let mut leaf = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["stop_helper", "--exact", "--nocapture"])
                .env("STOP_MODE", "leaf")
                .spawn()
                .unwrap();
            wait(None);
            let _ = leaf.wait();
        }
        "leaf" => {
            let path = PathBuf::from(var("STOP_LEAF"));
            let tmp = path.with_extension("tmp");
            std::fs::write(&tmp, std::process::id().to_string()).unwrap();
            std::fs::rename(&tmp, &path).unwrap();
            wait(None);
        }
        "sleep" => wait(None),
        #[cfg(windows)]
        "hold-job" => {
            let name: Vec<u16> = var("STOP_JOB")
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            // SAFETY: a NUL-terminated name that outlives the call; the handle is closed by the
            // end of this process.
            let h = unsafe {
                windows_sys::Win32::System::JobObjects::OpenJobObjectW(0x0004, 0, name.as_ptr())
            };
            assert!(!h.is_null(), "cannot open the job");
            std::fs::write(var("STOP_HELD"), "held").unwrap();
            wait(Some(PathBuf::from(var("STOP_RELEASE"))));
        }
        other => panic!("unknown STOP_MODE {other}"),
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
        env.set("JLOCK_CUSTODY_BEAT_MS", "200");
        let home = env.home.clone();
        let p = |n: &str| home.join(n).to_string_lossy().into_owned();
        env.set("STOP_LEAF", &p("leaf"));
        env.set("STOP_HELD", &p("held"));
        env.set("STOP_RELEASE", &p("release"));
        let dir = env.home.join("custody");
        Case { env, dir }
    }

    fn record(&self, id: &str, name: &str) -> PathBuf {
        self.dir.join(id).join(name)
    }

    fn helper(&self, mode: &str) -> std::process::Command {
        let mut cmd = std::process::Command::new(std::env::current_exe().unwrap());
        cmd.args(["stop_helper", "--exact", "--nocapture"])
            .env("STOP_MODE", mode)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        self.env.apply(&mut cmd);
        cmd
    }

    /// Starts a `parent` under custody and waits for its `leaf`; returns the receipt and the
    /// leaf's pid and identity.
    fn start(&self, id: &str) -> (Value, Value, Value) {
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
            "stop_helper",
            "--exact",
            "--nocapture",
        ]);
        cmd.env("STOP_MODE", "parent");
        let done = common::Kid::start(cmd).finish(Duration::from_secs(30));
        assert_eq!(done.code, 0, "{}{}", done.stdout, done.stderr);
        let v = done.json();
        assert_eq!(v["state"], "running", "{v}");
        let leaf = self.env.home.join("leaf");
        assert!(
            self.env.wait_for(Duration::from_secs(10), || leaf.exists()),
            "the grandchild never started"
        );
        let pid: u64 = std::fs::read_to_string(&leaf).unwrap().parse().unwrap();
        let identity = self.inspect(pid, None)["identity"].clone();
        assert!(identity.is_string(), "the grandchild is not present");
        (v["receipt"].clone(), json!(pid), identity)
    }

    fn inspect(&self, pid: u64, identity: Option<&str>) -> Value {
        let pid = pid.to_string();
        let mut args = vec!["process", "inspect", "--pid", &pid, "--json"];
        if let Some(i) = identity {
            args.extend_from_slice(&["--identity", i]);
        }
        self.env.run(&args).json()
    }

    fn absent(&self, pid: &Value, identity: &Value) -> bool {
        self.inspect(pid.as_u64().unwrap(), identity.as_str())["state"] == "absent"
    }

    fn present(&self, pid: &Value, identity: &Value) -> bool {
        self.inspect(pid.as_u64().unwrap(), identity.as_str())["state"] == "present"
    }

    /// `custody stop` with `extra` options and environment; returns the exit code and the answer.
    fn stop(&self, id: &str, extra: &[&str], vars: &[(&str, &str)]) -> (i32, Value) {
        let mut args = vec![
            "custody",
            "stop",
            "--id",
            id,
            "--custody",
            self.dir.to_str().unwrap(),
            "--reason",
            "test",
            "--json",
        ];
        args.extend_from_slice(extra);
        let mut cmd = self.env.cmd(&args);
        for (k, v) in vars {
            cmd.env(k, v);
        }
        let done = common::Kid::start(cmd).finish(Duration::from_secs(30));
        assert!(done.stderr.is_empty(), "{}", done.stderr);
        (done.code, done.json())
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

    fn read(&self, id: &str, name: &str) -> Value {
        serde_json::from_str(&std::fs::read_to_string(self.record(id, name)).unwrap()).unwrap()
    }

    /// Waits for every process of `receipt` (command and custodian) to be gone.
    fn gone(&self, receipt: &Value) {
        for (pid, identity) in [("pid", "identity"), ("custodianPid", "custodianIdentity")] {
            assert!(
                self.env.wait_for(Duration::from_secs(10), || self
                    .absent(&receipt[pid], &receipt[identity])),
                "{pid} is still alive"
            );
        }
    }
}

/// Ends our own process `pid` (a custodian this test started), only if it is still the
/// incarnation `identity`: the identity is checked on the handle that ends it.
#[cfg(windows)]
fn kill_own(pid: &Value, identity: &Value) {
    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME};
    use windows_sys::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, TerminateProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        PROCESS_TERMINATE,
    };
    let pid = pid.as_u64().unwrap() as u32;
    let want = identity.as_str().unwrap();
    // SAFETY: plain Win32 calls on a handle closed before returning; out pointers are locals.
    unsafe {
        let h = OpenProcess(
            PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION,
            0,
            pid,
        );
        assert!(!h.is_null(), "cannot open the custodian {pid}");
        let zero = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let (mut c, mut e, mut k, mut u) = (zero, zero, zero, zero);
        assert_ne!(GetProcessTimes(h, &mut c, &mut e, &mut k, &mut u), 0);
        let ft = (u64::from(c.dwHighDateTime) << 32) | u64::from(c.dwLowDateTime);
        let same = format!("win:{pid}:{ft}") == want;
        if same {
            TerminateProcess(h, 1);
        }
        CloseHandle(h);
        assert!(same, "the custodian {pid} is another process");
    }
}

#[cfg(windows)]
#[test]
fn stop_ends_the_tree_and_records_the_end_once() {
    let c = Case::new("custody-stop");
    let (receipt, leaf, leaf_id) = c.start("a1");
    let (code, v) = c.stop("a1", &[], &[]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["cleanupConfirmed"], true, "{v}");
    assert_eq!(v["reason"], "stopped", "{v}");
    assert_eq!(v["detail"], "test", "{v}");
    assert_eq!(v["by"], "stop", "{v}");
    assert_eq!(v["activeProcesses"], 0, "{v}");
    let fin = c.read("a1", "final.json");
    assert_eq!(fin, v, "the answer is not final.json");
    assert!(c.absent(&receipt["pid"], &receipt["identity"]));
    assert!(c.absent(&leaf, &leaf_id), "the grandchild survived");
    c.gone(&receipt);
    // The custodian saw its command end and left the record to the stop.
    assert_eq!(c.read("a1", "final.json"), fin);
    assert_eq!(c.status("a1")["state"], "ended");

    let path = c.record("a1", "final.json");
    let (bytes, modified) = (
        std::fs::read(&path).unwrap(),
        std::fs::metadata(&path).unwrap().modified().unwrap(),
    );
    std::thread::sleep(Duration::from_millis(50));
    let (code, again) = c.stop("a1", &[], &[]);
    assert_eq!(code, 0, "{again}");
    assert_eq!(again, fin, "a second stop answered something else");
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert_eq!(
        std::fs::metadata(&path).unwrap().modified().unwrap(),
        modified
    );
}

#[cfg(windows)]
#[test]
fn a_dead_custodian_is_lost_and_stop_confirms_by_identity() {
    let c = Case::new("custody-stop-lost");
    let (receipt, leaf, leaf_id) = c.start("l1");
    kill_own(&receipt["custodianPid"], &receipt["custodianIdentity"]);
    // Kill-on-close: the custodian's handle was the last one, so the tree ends with it.
    c.gone(&receipt);
    assert!(
        c.env
            .wait_for(Duration::from_secs(10), || c.absent(&leaf, &leaf_id)),
        "the grandchild outlived the custodian"
    );
    let st = c.status("l1");
    assert_eq!(st["state"], "lost", "{st}");
    assert_eq!(st["job"]["open"], false, "{st}");
    let (code, v) = c.stop("l1", &[], &[]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["cleanupConfirmed"], true, "{v}");
    assert_eq!(v["reason"], "custodian-lost", "{v}");
    assert_eq!(v["by"], "stop", "{v}");
    assert_eq!(c.read("l1", "final.json"), v);
    assert_eq!(c.status("l1")["state"], "ended");
}

#[cfg(windows)]
#[test]
fn a_held_job_outlives_its_custodian_and_stop_ends_it_by_name() {
    let c = Case::new("custody-stop-held");
    let (receipt, leaf, leaf_id) = c.start("h1");
    let mut holder = c.helper("hold-job");
    holder.env("STOP_JOB", receipt["job"].as_str().unwrap());
    let mut holder = common::Kid::start(holder);
    let held = c.env.home.join("held");
    assert!(
        c.env.wait_for(Duration::from_secs(10), || held.exists()),
        "the helper never opened the job"
    );
    kill_own(&receipt["custodianPid"], &receipt["custodianIdentity"]);
    assert!(c.env.wait_for(Duration::from_secs(10), || c
        .absent(&receipt["custodianPid"], &receipt["custodianIdentity"])));
    std::thread::sleep(Duration::from_millis(500));
    assert!(
        c.present(&receipt["pid"], &receipt["identity"]),
        "the command died with its custodian although the job is held"
    );
    assert!(c.present(&leaf, &leaf_id));
    let st = c.status("h1");
    assert_eq!(st["state"], "lost", "{st}");
    assert_eq!(st["job"]["open"], true, "{st}");

    let (code, v) = c.stop("h1", &[], &[]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["cleanupConfirmed"], true, "{v}");
    assert_eq!(v["by"], "stop", "{v}");
    assert!(c.absent(&receipt["pid"], &receipt["identity"]));
    assert!(c.absent(&leaf, &leaf_id), "the grandchild survived");
    assert!(holder.running(), "the helper holding the job was touched");
    std::fs::write(c.env.home.join("release"), "go").unwrap();
    assert!(holder.wait(Duration::from_secs(10)).is_some());
}

#[cfg(windows)]
#[test]
fn a_live_root_outside_any_job_is_never_ended() {
    let c = Case::new("custody-stop-orphan");
    let mut kid = common::Kid::start(c.helper("sleep"));
    let pid = json!(kid.pid);
    let identity = c.inspect(kid.pid as u64, None)["identity"].clone();
    assert!(identity.is_string());
    let rec = c.dir.join("f1");
    std::fs::create_dir_all(&rec).unwrap();
    let intent = json!({ "v": 1, "id": "f1", "argv": ["x"], "cwd": ".", "mode": "detached",
        "createdAt": "2026-01-01T00:00:00Z" });
    // A receipt for our own helper, in a job that does not exist, by a custodian that is gone
    // (its identity names another incarnation of the helper's pid).
    let receipt = json!({ "v": 1, "id": "f1", "pid": pid, "identity": identity,
        "job": "Local\\jlock-custody-000000000000-f1", "scope": "job", "custodianPid": pid,
        "custodianIdentity": format!("win:{}:1", kid.pid), "startedAt": "2026-01-01T00:00:00Z" });
    std::fs::write(rec.join("intent.json"), intent.to_string()).unwrap();
    std::fs::write(rec.join("receipt.json"), receipt.to_string()).unwrap();
    let (code, v) = c.stop("f1", &[], &[]);
    assert_eq!(code, 15, "{v}");
    assert_eq!(v["cleanupConfirmed"], false, "{v}");
    assert!(
        v["error"].as_str().unwrap().contains("still running"),
        "{v}"
    );
    assert!(kid.running(), "stop ended a process outside any job");
    assert!(c.present(&pid, &identity));
    assert!(!rec.join("final.json").exists());
    assert!(
        !rec.join("stop.json").exists(),
        "a failed stop left its request"
    );
    kid.kill();
}

#[cfg(windows)]
#[test]
fn a_wait_that_runs_out_is_not_confirmed() {
    let c = Case::new("custody-stop-timeout");
    let (receipt, leaf, leaf_id) = c.start("t1");
    let fault = ("JLOCK_TEST_CUSTODY_STOP_FAULT", "skip-terminate");
    let (code, v) = c.stop("t1", &[], &[fault, ("JLOCK_CUSTODY_STOP_MS", "300")]);
    assert_eq!(code, 15, "{v}");
    assert_eq!(v["cleanupConfirmed"], false, "{v}");
    assert!(v["activeProcesses"].as_u64().unwrap() >= 2, "{v}");
    assert!(
        v["error"]
            .as_str()
            .unwrap()
            .contains("timed out after 300 ms"),
        "{v}"
    );
    let (code, v) = c.stop("t1", &["--timeout", "200ms"], &[fault]);
    assert_eq!(code, 15, "{v}");
    assert_eq!(v["cleanupConfirmed"], false, "{v}");
    assert!(
        v["error"]
            .as_str()
            .unwrap()
            .contains("timed out after 200 ms"),
        "{v}"
    );
    assert!(!c.record("t1", "final.json").exists());
    assert!(c.present(&receipt["pid"], &receipt["identity"]));
    assert_eq!(c.status("t1")["state"], "running");

    let (code, v) = c.stop("t1", &[], &[]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["cleanupConfirmed"], true, "{v}");
    assert_eq!(v["reason"], "stopped", "{v}");
    assert!(c.absent(&leaf, &leaf_id));
    c.gone(&receipt);
}

#[cfg(windows)]
#[test]
fn stop_usage_and_nothing_to_stop() {
    let c = Case::new("custody-stop-usage");
    let dir = c.dir.to_str().unwrap();
    let done = c
        .env
        .run(&["custody", "stop", "--id", "n1", "--custody", dir]);
    assert_eq!(done.code, 2, "{}", done.stderr);
    assert!(done.stderr.contains("needs --reason"), "{}", done.stderr);
    let done = c.env.run(&[
        "custody",
        "stop",
        "--id",
        "n1",
        "--custody",
        dir,
        "--reason",
        "x",
        "--",
        "y",
    ]);
    assert_eq!(done.code, 2, "{}", done.stderr);
    let (code, v) = c.stop("n1", &[], &[]);
    assert_eq!(code, 15, "{v}");
    assert_eq!(v["cleanupConfirmed"], false, "{v}");
    assert!(!c.dir.join("n1").exists());
}
