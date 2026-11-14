//! The custodian watches its command: `beat.json` while it runs, `limit` and `quiet` deadlines that
//! end the job with the reason in `final.json`, and the self-fence that ends the job when no beat
//! can be written. The commands are only ever this test binary (`watch_helper`), every one bounded
//! in time, and each test waits for its custodian to be gone before it ends.

mod common;

#[cfg(windows)]
use common::Env;
#[cfg(windows)]
use serde_json::Value;
#[cfg(windows)]
use std::path::PathBuf;
use std::time::Duration;
#[cfg(windows)]
use std::time::Instant;

/// The command under custody, by `WATCH_MODE`. `talk`: a line every 50 ms until `WATCH_HUSH`
/// exists, then silent until `WATCH_RELEASE` exists (at most 20 s). `sleep`: silent for
/// `WATCH_SLEEP_MS`. `once`: one line, then silent for `WATCH_SLEEP_MS`. `chatty`: a line every
/// 200 ms for 3 s.
#[test]
fn watch_helper() {
    let Ok(mode) = std::env::var("WATCH_MODE") else {
        return;
    };
    let var = |name: &str| std::env::var(name).unwrap();
    let sleep_ms = || Duration::from_millis(var("WATCH_SLEEP_MS").parse().unwrap());
    let start = std::time::Instant::now();
    match mode.as_str() {
        "talk" => {
            let hush = std::path::PathBuf::from(var("WATCH_HUSH"));
            let release = std::path::PathBuf::from(var("WATCH_RELEASE"));
            while !hush.exists() && start.elapsed() < Duration::from_secs(20) {
                println!("talking");
                std::thread::sleep(Duration::from_millis(50));
            }
            while !release.exists() && start.elapsed() < Duration::from_secs(20) {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        "sleep" => std::thread::sleep(sleep_ms()),
        "once" => {
            println!("one line");
            std::thread::sleep(sleep_ms());
        }
        "chatty" => {
            while start.elapsed() < Duration::from_secs(3) {
                println!("still here");
                std::thread::sleep(Duration::from_millis(200));
            }
        }
        other => panic!("unknown WATCH_MODE {other}"),
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
        env.set("WATCH_HUSH", &p("hush"));
        env.set("WATCH_RELEASE", &p("release"));
        env.set("WATCH_SLEEP_MS", "30000");
        let dir = env.home.join("custody");
        Case { env, dir }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.env.home.join(name)
    }

    fn record(&self, id: &str, name: &str) -> PathBuf {
        self.dir.join(id).join(name)
    }

    /// `custody start` with `opts` before `--`; returns the receipt.
    fn start(&self, id: &str, mode: &str, opts: &[&str]) -> Value {
        let me = std::env::current_exe().unwrap();
        let dir = self.dir.to_str().unwrap();
        let mut args = vec!["custody", "start", "--id", id, "--custody", dir, "--json"];
        args.extend_from_slice(opts);
        args.extend_from_slice(&[
            "--",
            me.to_str().unwrap(),
            "watch_helper",
            "--exact",
            "--nocapture",
        ]);
        let mut cmd = self.env.cmd(&args);
        cmd.env("WATCH_MODE", mode);
        let done = common::Kid::start(cmd).finish(Duration::from_secs(30));
        assert_eq!(done.code, 0, "{}{}", done.stdout, done.stderr);
        let v = done.json();
        assert_eq!(v["state"], "running", "{v}");
        v["receipt"].clone()
    }

    /// A record, once it is whole JSON (`beat.json` is replaced while we read).
    fn read(&self, id: &str, name: &str) -> Option<Value> {
        let text = std::fs::read_to_string(self.record(id, name)).ok()?;
        serde_json::from_str(&text).ok()
    }

    /// Waits up to `timeout` for `final.json`.
    fn final_within(&self, id: &str, timeout: Duration) -> Option<Value> {
        let mut fin = None;
        self.env.wait_for(timeout, || {
            fin = self.read(id, "final.json");
            fin.is_some()
        });
        fin
    }

    /// Whether the process `pid` with `identity` is gone.
    fn absent(&self, pid: &Value, identity: &Value) -> bool {
        let pid = pid.as_u64().unwrap().to_string();
        let v = self
            .env
            .run(&[
                "process",
                "inspect",
                "--pid",
                &pid,
                "--identity",
                identity.as_str().unwrap(),
                "--json",
            ])
            .json();
        v["state"] == "absent"
    }

    /// Waits for the command and its custodian to be gone.
    fn gone(&self, receipt: &Value) {
        let ok = self.env.wait_for(Duration::from_secs(10), || {
            self.absent(&receipt["pid"], &receipt["identity"])
        });
        assert!(ok, "the command is still alive");
        let ok = self.env.wait_for(Duration::from_secs(10), || {
            self.absent(&receipt["custodianPid"], &receipt["custodianIdentity"])
        });
        assert!(ok, "the custodian is still alive");
    }
}

/// Milliseconds since the epoch of an ISO time `YYYY-MM-DDTHH:MM:SS.mmmZ`.
#[cfg(windows)]
fn iso_ms(text: &str) -> i64 {
    let n = |r: std::ops::Range<usize>| text[r].parse::<i64>().unwrap();
    let (y, m, d) = (n(0..4), n(5..7), n(8..10));
    let (y, m) = if m <= 2 { (y - 1, m + 9) } else { (y, m - 3) };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * m + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let secs = days * 86_400 + n(11..13) * 3600 + n(14..16) * 60 + n(17..19);
    secs * 1000 + n(20..23)
}

#[cfg(windows)]
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

#[cfg(windows)]
#[test]
fn beat_shows_life_and_the_last_output() {
    let c = Case::new("watch-beat");
    let receipt = c.start("w1", "talk", &[]);

    // While the command writes: seq grows, at is recent, the output moves.
    let mut seen: Vec<Value> = Vec::new();
    let talking = c.env.wait_for(Duration::from_secs(10), || {
        if let Some(b) = c.read("w1", "beat.json") {
            if seen.last().map(|l| l["seq"] != b["seq"]).unwrap_or(true) {
                seen.push(b);
            }
        }
        seen.len() >= 3 && seen.iter().filter(|b| !b["lastOutputAt"].is_null()).count() >= 2
    });
    assert!(talking, "{seen:?}");
    for pair in seen.windows(2) {
        assert!(
            pair[1]["seq"].as_u64() > pair[0]["seq"].as_u64(),
            "{seen:?}"
        );
    }
    let last = seen.last().unwrap();
    assert_eq!(last["v"], 1);
    assert_eq!(last["id"], "w1");
    let age = now_ms() - iso_ms(last["at"].as_str().unwrap());
    assert!((-2000..2000).contains(&age), "at is {age} ms old: {last}");
    let bytes: Vec<u64> = seen
        .iter()
        .map(|b| b["outputBytes"].as_u64().unwrap())
        .collect();
    assert!(bytes.last() > bytes.first(), "{seen:?}");
    let talked: Vec<&Value> = seen
        .iter()
        .filter(|b| !b["lastOutputAt"].is_null())
        .collect();
    assert_ne!(
        talked[0]["lastOutputAt"],
        talked.last().unwrap()["lastOutputAt"],
        "{seen:?}"
    );

    // Once it is silent: seq and at go on, the output stands still.
    std::fs::write(c.path("hush"), "hush").unwrap();
    std::thread::sleep(Duration::from_millis(500));
    let mut quiet = None;
    assert!(c.env.wait_for(Duration::from_secs(5), || {
        quiet = c.read("w1", "beat.json");
        quiet.is_some()
    }));
    let quiet = quiet.unwrap();
    let mut later = None;
    let moved = c.env.wait_for(Duration::from_secs(5), || {
        later = c
            .read("w1", "beat.json")
            .filter(|b| b["seq"].as_u64().unwrap() >= quiet["seq"].as_u64().unwrap() + 2);
        later.is_some()
    });
    assert!(moved, "{quiet}");
    let later = later.unwrap();
    assert_eq!(
        later["outputBytes"], quiet["outputBytes"],
        "{quiet} / {later}"
    );
    assert_eq!(
        later["lastOutputAt"], quiet["lastOutputAt"],
        "{quiet} / {later}"
    );
    assert!(
        iso_ms(later["at"].as_str().unwrap()) > iso_ms(quiet["at"].as_str().unwrap()),
        "{quiet} / {later}"
    );

    std::fs::write(c.path("release"), "go").unwrap();
    let fin = c.final_within("w1", Duration::from_secs(10)).unwrap();
    assert_eq!(fin["reason"], "exited", "{fin}");
    c.gone(&receipt);
}

#[cfg(windows)]
#[test]
fn past_the_limit_the_job_is_ended() {
    let c = Case::new("watch-limit");
    let receipt = c.start("l1", "sleep", &["--limit", "1s"]);
    let started = Instant::now();
    let fin = c.final_within("l1", Duration::from_secs(5));
    assert!(fin.is_some(), "no final.json after {:?}", started.elapsed());
    let fin = fin.unwrap();
    assert_eq!(fin["reason"], "limit", "{fin}");
    assert_eq!(fin["cleanupConfirmed"], true, "{fin}");
    assert_eq!(fin["activeProcesses"], 0, "{fin}");
    assert_eq!(fin["by"], "custodian", "{fin}");
    assert!(
        c.absent(&receipt["pid"], &receipt["identity"]),
        "the command outlived final.json"
    );
    c.gone(&receipt);
}

#[cfg(windows)]
#[test]
fn quiet_ends_a_silent_command_but_not_a_talking_one() {
    let c = Case::new("watch-quiet");
    let receipt = c.start("q1", "once", &["--quiet", "1s"]);
    let fin = c.final_within("q1", Duration::from_secs(8)).unwrap();
    assert_eq!(fin["reason"], "quiet", "{fin}");
    assert_eq!(fin["cleanupConfirmed"], true, "{fin}");
    c.gone(&receipt);

    let receipt = c.start("q2", "chatty", &["--quiet", "1s"]);
    let fin = c.final_within("q2", Duration::from_secs(10)).unwrap();
    assert_eq!(fin["reason"], "exited", "{fin}");
    assert_eq!(fin["exitCode"], 0, "{fin}");
    c.gone(&receipt);
}

#[cfg(windows)]
#[test]
fn without_quiet_silence_is_not_ended() {
    let mut c = Case::new("watch-no-quiet");
    c.env.set("WATCH_SLEEP_MS", "2500");
    let receipt = c.start("n1", "once", &[]);
    let fin = c.final_within("n1", Duration::from_secs(10)).unwrap();
    assert_eq!(fin["reason"], "exited", "{fin}");
    assert_eq!(fin["exitCode"], 0, "{fin}");
    c.gone(&receipt);
}

#[cfg(windows)]
#[test]
fn a_custodian_that_cannot_beat_ends_its_own_job() {
    let mut c = Case::new("watch-fence");
    c.env.set("JLOCK_CUSTODY_SELF_FENCE_MS", "600");
    let receipt = c.start("f1", "sleep", &[]);
    assert!(c.env.wait_for(Duration::from_secs(5), || c
        .record("f1", "beat.json")
        .exists()));
    // The command's folder cannot be swapped for a file while the command holds output.log open
    // (Windows refuses), so the beat's temporary name becomes a folder: every beat fails.
    let blocker = c.record("f1", "beat.json.tmp");
    assert!(c.env.wait_for(Duration::from_secs(5), || {
        std::fs::create_dir(&blocker).is_ok()
    }));
    let blocked = Instant::now();
    let ended = c.env.wait_for(Duration::from_secs(5), || {
        c.absent(&receipt["pid"], &receipt["identity"])
    });
    assert!(ended, "the command outlived its custodian's fence");
    assert!(blocked.elapsed() < Duration::from_secs(5));
    c.gone(&receipt);
    assert!(
        !c.record("f1", "final.json").exists(),
        "a fenced custodian recorded an end"
    );
    let st = c
        .env
        .run(&[
            "custody",
            "status",
            "--id",
            "f1",
            "--custody",
            c.dir.to_str().unwrap(),
            "--json",
        ])
        .json();
    assert_eq!(st["state"], "lost", "{st}");
    let _ = std::fs::remove_dir(&blocker);
}
