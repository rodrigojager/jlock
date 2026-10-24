mod common;

#[path = "../src/oslock.rs"]
#[allow(dead_code)]
mod oslock;

use common::{Env, Kid};
use oslock::{try_lock, Try};
use std::time::{Duration, Instant};

const SECS5: Duration = Duration::from_secs(5);

fn hold(env: &Env, key: &str, owner: &str) -> Kid {
    let mut kid = env.spawn(&["hold", key, "--owner", owner]);
    assert_eq!(kid.read_line(), format!("granted {key}"));
    kid
}

fn state(path: &std::path::Path) -> &'static str {
    match try_lock(path) {
        Ok(Try::Locked(_)) => "locked",
        Ok(Try::Busy) => "busy",
        Ok(Try::Unsupported) => "unsupported",
        Err(_) => "error",
    }
}

#[test]
fn live_hold_keeps_the_os_lock_and_death_frees_it_at_once() {
    let mut env = Env::new("oslock-death");
    env.set("JLOCK_STALE_MS", "20000");
    let key = "name:os";
    let mut kid = hold(&env, key, "a");
    let holder = env.key_dir(key).join("holder.json");
    assert_eq!(
        state(&holder),
        "busy",
        "another process sees the OS lock taken"
    );
    // The OS lock does not stop anyone from reading the file.
    let body = std::fs::read_to_string(&holder).unwrap();
    assert!(body.contains("\"owner\":\"a\""));

    kid.kill(); // our own child, by its handle/PID
    let killed = Instant::now();
    assert!(
        env.wait_for(Duration::from_secs(1), || state(&holder) == "locked"),
        "the kernel did not free the lock within 1 s"
    );
    assert!(killed.elapsed() < Duration::from_secs(1));
    // The jlock lock itself still waits for the heartbeat to go stale (20 s here).
    assert_eq!(env.holder_owner(key).as_deref(), Some("a"));
    std::fs::remove_file(&holder).unwrap();
    assert_eq!(env.run(&["gc"]).code, 0);
    assert!(env.lock_dirs().is_empty(), "{:?}", env.lock_dirs());
}

#[test]
fn renaming_the_held_file_is_not_blocked() {
    let env = Env::new("oslock-rename");
    let key = "name:rename";
    let mut kid = hold(&env, key, "a");
    let dir = env.key_dir(key);
    let moved = dir.join("moved.json");
    std::fs::rename(dir.join("holder.json"), &moved).expect("rename while held");
    // The hold finds its lock gone at the next heartbeat.
    assert_eq!(kid.wait(SECS5), Some(14));
    std::fs::remove_file(&moved).unwrap();
    assert_eq!(env.run(&["gc"]).code, 0);
    assert!(env.lock_dirs().is_empty(), "{:?}", env.lock_dirs());
}

#[test]
fn late_heartbeat_of_a_live_hold_is_not_broken() {
    let mut env = Env::new("oslock-late");
    env.set("JLOCK_STALE_MS", "300");
    env.set("JLOCK_HEARTBEAT_MS", "10000"); // the hold renews far too late
    let key = "name:late";
    let mut kid = hold(&env, key, "a");
    std::thread::sleep(Duration::from_millis(600)); // heartbeat now older than STALE
    let done = env.run(&[
        "acquire",
        key,
        "--timeout",
        "1500ms",
        "--owner",
        "b",
        "--json",
    ]);
    assert_eq!(done.code, 11, "{}", done.stdout);
    assert_eq!(done.json()["holder"]["owner"], "a");
    assert_eq!(env.holder_owner(key).as_deref(), Some("a"));
    assert!(kid.running());
    // list and status agree with the expiration: late, not dead.
    let row = env.run(&["list", "--json"]).json()["locks"][0]["holder"].clone();
    assert_eq!(row["late"], true, "{row}");
    assert_eq!(row["stale"], false, "{row}");
    let stale = env.run(&["list", "--stale", "--json"]).json();
    assert!(stale["locks"].as_array().unwrap().is_empty(), "{stale}");
    assert!(env.run(&["list"]).stdout.contains("(late)"));
    let st = env.run(&["status", key]).stdout;
    assert!(st.contains("(late, process alive)"), "{st}");
    kid.send("release");
    assert_eq!(kid.wait(SECS5), Some(0));
    assert!(env.lock_dirs().is_empty(), "{:?}", env.lock_dirs());
}

#[test]
fn plain_acquire_still_expires_by_heartbeat() {
    let mut env = Env::new("oslock-plain");
    env.set("JLOCK_STALE_MS", "300");
    let key = "name:plain";
    assert_eq!(
        env.run(&["acquire", key, "--no-wait", "--owner", "a"]).code,
        0
    );
    std::thread::sleep(Duration::from_millis(500));
    let done = env.run(&["acquire", key, "--timeout", "2s", "--owner", "b"]);
    assert_eq!(done.code, 0);
    assert_eq!(env.run(&["release", key, "--owner", "b"]).code, 0);
}
