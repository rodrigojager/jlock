mod common;

use common::{Env, Kid, JLOCK};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const WORKERS: usize = 8;
const CYCLES: usize = 1000;

fn cycles() -> usize {
    std::env::var("JLOCK_TEST_CYCLES").map_or(CYCLES, |v| v.parse().unwrap())
}
const DEATHS: usize = 20;
const LOG_MAX: u64 = 300_000;

/// One worker: this test binary running only this function, doing `JLOCK_TEST_WORKER=<id>:<n>`
/// acquire/release cycles over the keys in `JLOCK_TEST_KEYS` (separated by `|`). It inherits
/// `JLOCK_HOME` and the short times from the test.
#[test]
fn worker() {
    let Ok(spec) = std::env::var("JLOCK_TEST_WORKER") else {
        return;
    };
    let (id, cycles) = spec.split_once(':').unwrap();
    let (id, cycles): (u64, usize) = (id.parse().unwrap(), cycles.parse().unwrap());
    let keys: Vec<String> = std::env::var("JLOCK_TEST_KEYS")
        .unwrap()
        .split('|')
        .map(str::to_string)
        .collect();
    let mut seed = id * 7919 + 17;
    let jlock = |args: &[&str]| {
        let out = Command::new(JLOCK).args(args).output().unwrap();
        let code = out.status.code().unwrap_or(-1);
        if code != 0 && code != 11 && code != 13 {
            eprintln!(
                "{args:?} -> {code}: {}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
        }
        code
    };
    for i in 0..cycles {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let key = &keys[(seed >> 33) as usize % keys.len()];
        let owner = format!("w{id}-{i}");
        match jlock(&["acquire", key, "--owner", &owner, "--timeout", "60s"]) {
            0 => {
                // 13 is possible: a lock slower to release than STALE was expired by a waiter.
                let code = jlock(&["release", key, "--owner", &owner]);
                assert!(code == 0 || code == 13, "release {key}: exit {code}");
            }
            11 => {}
            code => panic!("acquire {key}: exit {code}"),
        }
    }
}

fn keys(env: &Env) -> Vec<String> {
    let mut keys: Vec<String> = (0..8).map(|i| format!("name:k{i}")).collect();
    for p in [
        "base/p",
        "base/p/a",
        "base/p/b",
        "base/p/a/x",
        "base/q",
        "base/q/1",
        "base/q/2",
        "base/r",
        "base/r/s/t",
        "base/u",
        "base/v",
        "base/w/z",
    ] {
        keys.push(env.path(p));
    }
    assert_eq!(keys.len(), 20);
    keys
}

fn broken_dirs(env: &Env) -> Vec<String> {
    env.lock_dirs()
        .into_iter()
        .filter(|d| env.locks().join(d).join("broken").exists())
        .collect()
}

#[test]
fn stress_with_deaths_leaves_nothing_behind() {
    let mut env = Env::new("leftovers-stress");
    env.set("JLOCK_HEARTBEAT_MS", "200");
    env.set("JLOCK_STALE_MS", "1500");
    env.set("JLOCK_SWEEP_MS", "500");
    env.set("JLOCK_LOG_MAX_BYTES", &LOG_MAX.to_string());
    let keys = keys(&env);
    let me = std::env::current_exe().unwrap();

    let workers: Vec<Kid> = (0..WORKERS)
        .map(|id| {
            let mut w = Command::new(&me);
            w.args(["worker", "--exact", "--quiet"])
                .env("JLOCK_TEST_WORKER", format!("{id}:{}", cycles()))
                .env("JLOCK_TEST_KEYS", keys.join("|"))
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            env.apply(&mut w);
            Kid::start(w)
        })
        .collect();

    // Deaths: victims of ours that hold or wait on the same keys, stopped by PID mid-way.
    let start = Instant::now();
    for d in 0..DEATHS {
        let key = &keys[(d * 7) % keys.len()];
        let owner = format!("victim{d}");
        let mut victim = if d % 2 == 0 {
            env.spawn(&["hold", key, "--owner", &owner])
        } else {
            env.spawn(&["acquire", key, "--owner", &owner])
        };
        std::thread::sleep(Duration::from_millis(150 + (d as u64 % 5) * 70));
        victim.kill();
    }
    eprintln!("deaths done after {:?}", start.elapsed());

    for (i, w) in workers.into_iter().enumerate() {
        let done = w.finish(Duration::from_secs(280));
        assert_eq!(done.code, 0, "worker {i}: {}{}", done.stdout, done.stderr);
    }
    eprintln!("workers done after {:?}", start.elapsed());

    // Whatever the victims left expires after STALE; gc then removes it.
    std::thread::sleep(Duration::from_millis(1700));
    let gc = env.run(&["gc", "--json"]);
    assert_eq!(gc.code, 0, "{}", gc.stdout);
    eprintln!("gc: {}", gc.stdout.trim());
    assert!(env.lock_dirs().is_empty(), "left: {:?}", env.lock_dirs());
    assert!(broken_dirs(&env).is_empty());
    let size = |name: &str| std::fs::metadata(env.home.join(name)).map_or(0, |m| m.len());
    let total = size("log.jsonl") + size("log.1.jsonl");
    assert!(total <= 2 * LOG_MAX, "log {total} bytes");
}

#[test]
fn empty_key_folder_disappears_on_the_next_call() {
    let mut env = Env::new("leftovers-empty");
    env.set("JLOCK_SWEEP_MS", "100");
    let stray = env.locks().join("0123456789abcdef0123456789abcdef01234567");
    std::fs::create_dir_all(stray.join("queue")).unwrap();
    std::fs::create_dir_all(stray.join("broken")).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(env.run(&["status", "name:anything"]).code, 0);
    assert!(!stray.exists(), "stray folder still there");
    assert!(env.lock_dirs().is_empty());
}

#[test]
fn release_racing_a_new_ticket_on_the_same_key() {
    let mut env = Env::new("leftovers-race");
    env.set("JLOCK_STALE_MS", "20000");
    let key = "name:edge";
    for round in 0..40 {
        let a = format!("a{round}");
        let b = format!("b{round}");
        assert_eq!(
            env.run(&["acquire", key, "--no-wait", "--owner", &a]).code,
            0
        );
        let waiter = env.spawn(&["acquire", key, "--owner", &b, "--timeout", "10s"]);
        // Release at varying moments around the waiter's arrival.
        std::thread::sleep(Duration::from_millis((round % 8) as u64 * 3));
        let done = env.run(&["release", key, "--owner", &a]);
        assert_eq!(done.code, 0, "round {round}: {}", done.stderr);
        let done = waiter.finish(Duration::from_secs(10));
        assert_eq!(
            done.code, 0,
            "round {round}: {}{}",
            done.stdout, done.stderr
        );
        assert_eq!(env.run(&["release", key, "--owner", &b]).code, 0);
    }
    assert!(env.lock_dirs().is_empty(), "{:?}", env.lock_dirs());
}
