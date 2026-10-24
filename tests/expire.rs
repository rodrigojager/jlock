mod common;

use common::Env;
use std::time::{Duration, Instant};

const SECS5: Duration = Duration::from_secs(5);
const STALE_MS: u64 = 1000;

fn env(name: &str) -> Env {
    let mut env = Env::new(name);
    env.set("JLOCK_HEARTBEAT_MS", "100");
    env.set("JLOCK_STALE_MS", &STALE_MS.to_string());
    env
}

fn no_broken_left(env: &Env, key: &str) -> bool {
    let broken = env.key_dir(key).join("broken");
    std::fs::read_dir(&broken).map_or(true, |mut d| d.next().is_none())
}

#[test]
fn dead_holder_is_recovered_within_two_stale_periods() {
    let env = env("expire-holder");
    let key = "name:dead-holder";
    let mut hold = env.spawn(&["hold", key, "--owner", "a"]);
    assert_eq!(hold.read_line(), format!("granted {key}"));
    // The hold stays alive past STALE: its heartbeat keeps the lock.
    std::thread::sleep(Duration::from_millis(STALE_MS * 2));
    assert_eq!(env.holder_owner(key).as_deref(), Some("a"));

    hold.kill(); // our own child, by its handle/PID
    let killed = Instant::now();
    let done = env.run(&["acquire", key, "--timeout", "5s", "--owner", "b", "--json"]);
    assert_eq!(done.code, 0, "{}", done.stdout);
    assert!(
        killed.elapsed() < Duration::from_millis(2 * STALE_MS + 500),
        "took {:?}",
        killed.elapsed()
    );
    assert_eq!(env.holder_owner(key).as_deref(), Some("b"));
    assert!(no_broken_left(&env, key));
    assert_eq!(env.run(&["release", key, "--owner", "b"]).code, 0);
    assert!(env.lock_dirs().is_empty(), "{:?}", env.lock_dirs());
}

#[test]
fn dead_waiter_ticket_expires_and_the_queue_closes_the_gap() {
    let env = env("expire-waiter");
    let key = "name:dead-waiter";
    let mut hold = env.spawn(&["hold", key, "--owner", "a"]);
    assert_eq!(hold.read_line(), format!("granted {key}"));
    let mut b = env.spawn(&["acquire", key, "--owner", "b"]);
    assert!(env.wait_for(SECS5, || env.tickets(key) == 1));
    let c = env.spawn(&["acquire", key, "--owner", "c", "--json"]);
    assert!(env.wait_for(SECS5, || env.tickets(key) == 2));

    b.kill(); // b dies while waiting: its ticket stays behind, unrenewed
    assert!(
        env.wait_for(Duration::from_millis(3 * STALE_MS), || env.tickets(key)
            == 1),
        "b's ticket did not expire"
    );
    hold.send("release");
    assert_eq!(hold.wait(SECS5), Some(0));
    let done = c.finish(SECS5);
    assert_eq!(done.code, 0, "{}", done.stdout);
    assert_eq!(env.holder_owner(key).as_deref(), Some("c"));
    assert!(no_broken_left(&env, key));
    assert_eq!(env.run(&["release", key, "--owner", "c"]).code, 0);
    assert!(env.lock_dirs().is_empty(), "{:?}", env.lock_dirs());
}

#[test]
fn hold_whose_lock_was_deleted_exits_14() {
    let env = env("expire-lost");
    let key = "name:lost";
    let mut hold = env.spawn(&["hold", key, "--owner", "a"]);
    assert_eq!(hold.read_line(), format!("granted {key}"));
    std::fs::remove_file(env.key_dir(key).join("holder.json")).unwrap();
    assert_eq!(hold.wait(SECS5), Some(14));
    assert_eq!(hold.read_line(), format!("lost {key}"));
}

#[test]
fn ttl_ends_hold_with_14_and_frees_plain_locks() {
    let mut env = env("expire-ttl");
    env.set("JLOCK_STALE_MS", "20000"); // only the ttl can end these locks
    let key = "name:ttl";
    let mut hold = env.spawn(&["hold", key, "--owner", "a", "--ttl", "1s"]);
    assert_eq!(hold.read_line(), format!("granted {key}"));
    assert_eq!(hold.wait(SECS5), Some(14));
    assert!(env.holder_owner(key).is_none());

    assert_eq!(
        env.run(&["acquire", key, "--no-wait", "--owner", "b", "--ttl", "1s"])
            .code,
        0
    );
    let started = Instant::now();
    let done = env.run(&["acquire", key, "--timeout", "5s", "--owner", "c"]);
    assert_eq!(done.code, 0);
    assert!(started.elapsed() < Duration::from_secs(3));
    assert_eq!(env.run(&["release", key, "--owner", "c"]).code, 0);
    assert!(env.lock_dirs().is_empty(), "{:?}", env.lock_dirs());
}

#[test]
fn heartbeat_command_renews_or_reports_lost() {
    let env = env("expire-beat");
    let key = "name:beat";
    assert_eq!(
        env.run(&["acquire", key, "--no-wait", "--owner", "a"]).code,
        0
    );
    for _ in 0..3 {
        std::thread::sleep(Duration::from_millis(STALE_MS / 2));
        assert_eq!(env.run(&["heartbeat", key, "--owner", "a"]).code, 0);
    }
    // Still ours after 1.5 x STALE thanks to the renewals.
    assert_eq!(env.holder_owner(key).as_deref(), Some("a"));
    let done = env.run(&["heartbeat", key, "--owner", "z", "--json"]);
    assert_eq!(done.code, 13);
    assert_eq!(done.json()["result"], "lost");
    assert_eq!(env.run(&["release", key, "--owner", "a"]).code, 0);
}
