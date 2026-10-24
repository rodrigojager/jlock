//! `hold --detach`: one call that answers like `hold` and returns, while a second process keeps the
//! lock; and the owner's `jlock release` ending a `hold` as a release, not as a loss. Every detached
//! keeper here also has a `--ttl`, so none can outlive its test even if the test fails midway (the
//! tests start them but never stop them: they end by themselves).

mod common;

use common::Env;
use std::time::Duration;

fn log_events(env: &Env, key: &str) -> Vec<String> {
    let text = std::fs::read_to_string(env.home.join("log.jsonl")).unwrap_or_default();
    text.lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|v| v["key"] == key)
        .map(|v| v["event"].as_str().unwrap_or("").to_string())
        .collect()
}

#[test]
fn detach_answers_and_keeps_the_lock_alive() {
    let env = Env::new("detach-keep");
    let done = env.run(&["hold", "name:x", "--owner", "a", "--detach", "--ttl", "30s"]);
    assert_eq!(done.code, 0, "{}{}", done.stdout, done.stderr);
    assert_eq!(done.stdout, "granted name:x\n");
    // Well past JLOCK_STALE_MS (1500 ms in the tests): still held, heartbeat fresh.
    std::thread::sleep(Duration::from_millis(2500));
    let st = env.run(&["status", "name:x", "--json"]).json();
    assert_eq!(st["holder"]["owner"], "a", "{st}");
    assert!(
        st["holder"]["heartbeat_ms"].as_u64().unwrap() < 1500,
        "{st}"
    );
    assert_eq!(st["holder"]["stale"], false);

    let busy = env.run(&["hold", "name:x", "--owner", "b", "--detach", "--no-wait"]);
    assert_eq!(busy.code, 10, "{}", busy.stdout);
    assert!(
        busy.stdout.starts_with("busy name:x held by a"),
        "{}",
        busy.stdout
    );

    // The owner gives it back: the keeper ends quietly, nothing is left, no loss is logged.
    assert_eq!(env.run(&["release", "name:x", "--owner", "a"]).code, 0);
    assert!(env.wait_for(Duration::from_secs(3), || env.lock_dirs().is_empty()));
    std::thread::sleep(Duration::from_millis(500));
    assert!(env.lock_dirs().is_empty(), "the keeper took the key again");
    let events = log_events(&env, "name:x");
    assert!(events.contains(&"release".to_string()), "{events:?}");
    assert!(!events.contains(&"lost".to_string()), "{events:?}");
}

#[test]
fn detach_waits_its_turn_and_speaks_json() {
    let env = Env::new("detach-wait");
    let mut first = env.spawn(&["hold", "name:y", "--owner", "a"]);
    assert_eq!(first.read_line(), "granted name:y");
    let waiter = env.spawn(&[
        "hold", "name:y", "--owner", "b", "--detach", "--ttl", "30s", "--json",
    ]);
    std::thread::sleep(Duration::from_millis(300));
    first.send("release");
    let done = waiter.finish(Duration::from_secs(10));
    assert_eq!(done.code, 0, "{}{}", done.stdout, done.stderr);
    let v = done.json();
    assert_eq!(v["result"], "granted");
    assert_eq!(v["owner"], "b");
    assert_eq!(first.wait(Duration::from_secs(5)), Some(0));
    assert_eq!(env.holder_owner("name:y").as_deref(), Some("b"));
    assert_eq!(env.run(&["release", "--owner", "b"]).code, 0);
    assert!(env.wait_for(Duration::from_secs(3), || env.lock_dirs().is_empty()));
}

#[test]
fn a_forced_detached_lock_is_logged_as_lost() {
    let env = Env::new("detach-forced");
    let done = env.run(&["hold", "name:z", "--owner", "a", "--detach", "--ttl", "30s"]);
    assert_eq!(done.code, 0);
    assert_eq!(env.run(&["unlock", "name:z", "--force"]).code, 0);
    assert!(env.wait_for(Duration::from_secs(3), || {
        log_events(&env, "name:z").contains(&"lost".to_string())
    }));
    assert!(env.lock_dirs().is_empty());
}

#[test]
fn release_by_the_owner_ends_a_foreground_hold_with_0() {
    let env = Env::new("detach-foreground");
    let mut hold = env.spawn(&["hold", "name:a", "name:b", "--owner", "a"]);
    assert_eq!(hold.read_line(), "granted name:a");
    assert_eq!(hold.read_line(), "granted name:b");
    // One key back: the other stays held.
    assert_eq!(env.run(&["release", "name:a", "--owner", "a"]).code, 0);
    assert_eq!(hold.read_line(), "released name:a");
    assert!(hold.running());
    assert_eq!(env.holder_owner("name:b").as_deref(), Some("a"));
    // The last one: hold ends with 0, not 14.
    assert_eq!(env.run(&["release", "name:b", "--owner", "a"]).code, 0);
    assert_eq!(hold.wait(Duration::from_secs(5)), Some(0));
    for key in ["name:a", "name:b"] {
        assert!(
            !log_events(&env, key).contains(&"lost".to_string()),
            "{key}"
        );
    }
}
