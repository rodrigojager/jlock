//! `jlock renew` (ticket 19): the deadline of a live lock moves to now + ttl without the key ever
//! being free; the wrong owner is refused, a lock already past its deadline expires, and a
//! `lease.json` of another grant counts for nothing.
mod common;

use common::Env;
use std::thread::sleep;
use std::time::{Duration, Instant};

const SECS5: Duration = Duration::from_secs(5);

/// Only the ttl can end a lock here: the heartbeat goes stale long after every test is over.
fn env(name: &str) -> Env {
    let mut env = Env::new(name);
    env.set("JLOCK_STALE_MS", "20000");
    env
}

fn log(env: &Env) -> String {
    std::fs::read_to_string(env.home.join("log.jsonl")).unwrap_or_default()
}

#[test]
fn renew_keeps_a_hold_alive_past_its_ttl() {
    let env = env("renew-hold");
    let key = "name:long-task";
    let mut hold = env.spawn(&["hold", key, "--owner", "a", "--ttl", "2s"]);
    assert_eq!(hold.read_line(), format!("granted {key}"));
    let started = Instant::now();
    sleep(Duration::from_millis(1000));
    let done = env.run(&["renew", key, "--owner", "a", "--ttl", "3s", "--json"]);
    assert_eq!(done.code, 0, "{} {}", done.stdout, done.stderr);
    let out = done.json();
    assert_eq!(out["result"], "ok");
    assert_eq!(out["keys"][0], key);
    assert!(out["until"].is_string(), "{out}");

    // Past the original 2 s: still held, by the same hold, which never saw a loss.
    sleep(Duration::from_millis(1500));
    assert!(hold.running(), "the hold ended at {:?}", started.elapsed());
    assert_eq!(env.holder_owner(key).as_deref(), Some("a"));
    let status = env.run(&["status", key, "--json"]).json();
    assert_eq!(status["holder"]["expires"], out["until"]);
    assert_eq!(status["holder"]["stale"], false);
    let text = env.run(&["status", key]).stdout;
    assert!(text.contains(", expires in "), "{text}");

    // Without another renew, the new deadline ends it like the ttl always did.
    assert_eq!(hold.wait(SECS5), Some(14));
    assert!(started.elapsed() >= Duration::from_millis(3900));
    assert!(env.holder_owner(key).is_none());
    assert!(env.lock_dirs().is_empty(), "{:?}", env.lock_dirs());
    let log = log(&env);
    assert_eq!(log.matches("\"event\":\"renew\"").count(), 1, "{log}");
    assert!(
        !log.contains("\"event\":\"release\""),
        "never let go: {log}"
    );
}

#[test]
fn without_ttl_the_grant_ttl_is_reused() {
    let env = env("renew-default");
    let key = "name:default-ttl";
    let granted = env.run(&["acquire", key, "--no-wait", "--owner", "a", "--ttl", "2s"]);
    assert_eq!(granted.code, 0);
    sleep(Duration::from_millis(1500));
    let done = env.run(&["renew", key, "--owner", "a"]);
    assert_eq!(done.code, 0, "{}", done.stdout);
    assert!(done.stdout.starts_with(&format!("renewed {key} until ")));
    // 2.5 s after the grant: past since + ttl, within the renewed deadline.
    sleep(Duration::from_millis(1000));
    let busy = env.run(&["acquire", key, "--no-wait", "--owner", "b", "--json"]);
    assert_eq!(busy.json()["result"], "queued", "{}", busy.stdout);
    assert_eq!(env.holder_owner(key).as_deref(), Some("a"));
    assert_eq!(env.run(&["release", key, "--owner", "a"]).code, 0);
    assert!(env.lock_dirs().is_empty(), "{:?}", env.lock_dirs());
}

#[test]
fn a_lock_without_a_deadline_only_gets_its_heartbeat() {
    let env = env("renew-no-deadline");
    let key = "name:no-ttl";
    assert_eq!(
        env.run(&["acquire", key, "--no-wait", "--owner", "a"]).code,
        0
    );
    let done = env.run(&["renew", key, "--owner", "a", "--json"]);
    assert_eq!(done.code, 0);
    assert!(done.json()["until"].is_null());
    assert!(!env.key_dir(key).join("lease.json").exists());
    assert_eq!(env.run(&["release", key, "--owner", "a"]).code, 0);
    assert!(env.lock_dirs().is_empty(), "{:?}", env.lock_dirs());
}

#[test]
fn wrong_owner_is_refused_and_an_expired_lock_is_lost() {
    let env = env("renew-refused");
    let key = "name:refused";
    assert_eq!(
        env.run(&["acquire", key, "--no-wait", "--owner", "a", "--ttl", "1s"])
            .code,
        0
    );
    let done = env.run(&["renew", key, "--owner", "z", "--ttl", "1h", "--json"]);
    assert_eq!(done.code, 13);
    assert_eq!(done.json()["result"], "not_owner");
    assert_eq!(done.json()["not_owner"][0]["holder"], "a");
    assert!(!env.key_dir(key).join("lease.json").exists());
    let free = env.run(&["renew", "name:nobody", "--owner", "a"]);
    assert_eq!(free.code, 13);

    sleep(Duration::from_millis(1300));
    let late = env.run(&["renew", key, "--owner", "a", "--ttl", "1h", "--json"]);
    assert_eq!(late.code, 14, "{}", late.stdout);
    assert_eq!(late.json()["result"], "lost");
    assert!(env.holder_owner(key).is_none());
    assert!(env.lock_dirs().is_empty(), "{:?}", env.lock_dirs());
    assert!(log(&env).contains("\"event\":\"expired\""));
}

#[test]
fn without_keys_every_lock_of_the_owner_and_only_of_the_group() {
    let env = env("renew-all");
    let take = |key: &str, owner: &str, group: &str| {
        let args = [
            "acquire",
            key,
            "--no-wait",
            "--owner",
            owner,
            "--group",
            group,
            "--ttl",
            "1h",
        ];
        assert_eq!(env.run(&args).code, 0);
    };
    take("name:a1", "a", "g1");
    take("name:a2", "a", "g2");
    take("name:b1", "b", "g1");
    let one = env.run(&[
        "renew", "--owner", "a", "--group", "g1", "--ttl", "2h", "--json",
    ]);
    assert_eq!(one.code, 0);
    assert_eq!(one.json()["keys"], serde_json::json!(["name:a1"]));
    let all = env.run(&["renew", "--owner", "a", "--json"]).json();
    let mut keys: Vec<String> = all["keys"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| k.as_str().unwrap().to_string())
        .collect();
    keys.sort();
    assert_eq!(keys, ["name:a1", "name:a2"]);
    assert!(!env.key_dir("name:b1").join("lease.json").exists());
    let none = env.run(&["renew", "--owner", "nobody"]);
    assert_eq!((none.code, none.stdout.trim()), (0, "nothing to renew"));
    assert_eq!(env.run(&["renew"]).code, 2);
    for owner in ["a", "b"] {
        assert_eq!(env.run(&["release", "--owner", owner]).code, 0);
    }
    assert!(env.lock_dirs().is_empty(), "{:?}", env.lock_dirs());
}

#[test]
fn a_lease_of_another_grant_counts_for_nothing() {
    let env = env("renew-stale-lease");
    let key = "name:old-lease";
    assert_eq!(
        env.run(&["acquire", key, "--no-wait", "--owner", "a", "--ttl", "1h"])
            .code,
        0
    );
    assert_eq!(env.run(&["renew", key, "--owner", "a"]).code, 0);
    let lease = env.key_dir(key).join("lease.json");
    let old = std::fs::read(&lease).unwrap();
    assert_eq!(env.run(&["release", key, "--owner", "a"]).code, 0);
    assert!(env.lock_dirs().is_empty(), "the lease went with the folder");

    // The same owner takes it again with a short ttl; a lease left from the first grant (far in
    // the future) must not keep the new one alive.
    assert_eq!(
        env.run(&["acquire", key, "--no-wait", "--owner", "a", "--ttl", "1s"])
            .code,
        0
    );
    std::fs::write(&lease, old).unwrap();
    sleep(Duration::from_millis(1300));
    let next = env.run(&["acquire", key, "--no-wait", "--owner", "b", "--json"]);
    assert_eq!(next.json()["result"], "granted", "{}", next.stdout);
    assert_eq!(env.run(&["release", key, "--owner", "b"]).code, 0);
    assert!(env.lock_dirs().is_empty(), "{:?}", env.lock_dirs());
}

#[test]
fn renew_racing_release_never_fails_and_leaves_nothing() {
    let env = env("renew-race");
    let key = "name:race";
    let mut codes = std::collections::BTreeMap::new();
    for _ in 0..60 {
        let granted = env.run(&["acquire", key, "--no-wait", "--owner", "a", "--ttl", "1h"]);
        assert_eq!(granted.code, 0, "{}", granted.stdout);
        let renew = std::thread::scope(|s| {
            let renew = s.spawn(|| env.run(&["renew", "--owner", "a", "--json"]));
            assert_eq!(env.run(&["release", key, "--owner", "a"]).code, 0);
            renew.join().unwrap()
        });
        assert!(
            renew.code == 0,
            "exit {}: {} {}",
            renew.code,
            renew.stdout,
            renew.stderr
        );
        *codes.entry(renew.code).or_insert(0) += 1;
        assert!(env.lock_dirs().is_empty(), "{:?}", env.lock_dirs());
    }
    let doctor = env.run(&["doctor", "--json"]).json();
    let registry = doctor["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "registry")
        .unwrap()
        .clone();
    assert_eq!(registry["status"], "ok", "{registry} {codes:?}");
}
