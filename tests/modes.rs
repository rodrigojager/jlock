//! Shared and exclusive modes (0.3): shared holds of a key coexist, an exclusive hold excludes any
//! other, path hierarchy conflicts apply across modes, and an exclusive waiter is never starved by
//! later shared requests.
mod common;

use common::Env;
use serde_json::Value;
use std::sync::{Arc, Barrier};
use std::time::Duration;

const SECS5: Duration = Duration::from_secs(5);

fn env(name: &str) -> Env {
    let mut env = Env::new(name);
    env.set("JLOCK_STALE_MS", "20000");
    env
}

fn take(env: &Env, key: &str, owner: &str, mode: &str) -> common::Done {
    env.run(&[
        "acquire",
        key,
        "--no-wait",
        "--owner",
        owner,
        "--mode",
        mode,
        "--json",
    ])
}

fn modes(env: &Env, key: &str) -> Vec<(String, String)> {
    let st = env.run(&["status", key, "--json"]).json();
    st["holders"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| {
            (
                h["owner"].as_str().unwrap().to_string(),
                h["mode"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

#[test]
fn shared_holds_coexist_and_exclusive_waits_for_them() {
    let env = env("modes-shared");
    assert_eq!(take(&env, "name:db", "r1", "shared").code, 0);
    let second = take(&env, "name:db", "r2", "shared");
    assert_eq!(second.code, 0, "{}", second.stdout);
    assert_eq!(second.json()["mode"], "shared");
    assert_eq!(
        modes(&env, "name:db"),
        [
            ("r1".to_string(), "shared".to_string()),
            ("r2".to_string(), "shared".to_string())
        ]
    );
    let busy = take(&env, "name:db", "w", "exclusive");
    assert_eq!(busy.code, 10);
    let v = busy.json();
    assert_eq!(v["holder"]["mode"], "shared");
    assert_eq!(v["holders"].as_array().unwrap().len(), 2);
    // The text line names the holder, its mode and the others.
    let text = env.run(&["acquire", "name:db", "--no-wait", "--owner", "w"]);
    assert!(
        text.stdout.contains("held by r1 (shared) and 1 other"),
        "{}",
        text.stdout
    );
    // An exclusive hold keeps shared requests out too.
    for o in ["r1", "r2"] {
        assert_eq!(env.run(&["release", "name:db", "--owner", o]).code, 0);
    }
    assert_eq!(take(&env, "name:db", "w", "exclusive").code, 0);
    assert_eq!(take(&env, "name:db", "r3", "shared").code, 10);
    assert_eq!(env.run(&["release", "name:db", "--owner", "w"]).code, 0);
    assert!(env.lock_dirs().is_empty(), "{:?}", env.lock_dirs());
}

#[test]
fn per_key_suffix_overrides_the_mode_option() {
    let env = env("modes-suffix");
    let done = env.run(&[
        "acquire",
        "name:a@exclusive",
        "name:b",
        "--mode",
        "shared",
        "--owner",
        "x",
        "--json",
    ]);
    assert_eq!(done.code, 0, "{}", done.stdout);
    let locks = done.json()["locks"].clone();
    assert_eq!(locks[0]["key"], "name:a");
    assert_eq!(locks[0]["mode"], "exclusive");
    assert_eq!(locks[1]["key"], "name:b");
    assert_eq!(locks[1]["mode"], "shared");
    assert_eq!(take(&env, "name:b@shared", "y", "exclusive").code, 0);
    assert_eq!(take(&env, "name:a", "y", "shared").code, 10);
    // A key named twice is taken once, in the stronger mode.
    let twice = env.run(&[
        "acquire",
        "name:c@shared",
        "name:c",
        "--owner",
        "z",
        "--json",
    ]);
    assert_eq!(twice.json()["locks"][0]["mode"], "exclusive");
    assert_eq!(
        env.run(&["acquire", "name:x", "--mode", "sometimes"]).code,
        2
    );
    for o in ["x", "y", "z"] {
        assert_eq!(env.run(&["release", "--owner", o]).code, 0);
    }
    assert!(env.lock_dirs().is_empty(), "{:?}", env.lock_dirs());
}

#[test]
fn hierarchy_conflicts_apply_across_modes() {
    let env = env("modes-hierarchy");
    let dir = env.path("p/src");
    let file = env.path("p/src/a.ts");
    assert_eq!(take(&env, &dir, "reader", "shared").code, 0);
    assert_eq!(
        take(&env, &file, "other-reader", "shared").code,
        0,
        "shared folder, shared file"
    );
    let busy = take(&env, &file, "writer", "exclusive");
    assert_eq!(busy.code, 10, "shared folder, exclusive file inside");
    let v = busy.json();
    assert_eq!(
        v["holder"]["owner"], "other-reader",
        "the hold on the file itself first"
    );
    let owners: Vec<&str> = v["holders"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["owner"].as_str().unwrap())
        .collect();
    assert_eq!(owners.len(), 2, "{owners:?}");
    assert!(owners.contains(&"reader"), "the folder above blocks it too");
    for (k, o) in [(&dir, "reader"), (&file, "other-reader")] {
        assert_eq!(env.run(&["release", k, "--owner", o]).code, 0);
    }
    assert_eq!(take(&env, &file, "writer", "exclusive").code, 0);
    assert_eq!(
        take(&env, &dir, "reader", "shared").code,
        10,
        "and the other way"
    );
    assert_eq!(env.run(&["release", &file, "--owner", "writer"]).code, 0);
    assert!(env.lock_dirs().is_empty(), "{:?}", env.lock_dirs());
}

#[test]
fn an_exclusive_waiter_is_not_starved_by_later_readers() {
    let env = env("modes-starve");
    assert_eq!(take(&env, "name:s", "r1", "shared").code, 0);
    let writer = env.spawn(&[
        "acquire",
        "name:s",
        "--owner",
        "w",
        "--timeout",
        "20s",
        "--json",
    ]);
    assert!(env.wait_for(SECS5, || env.tickets("name:s") == 1));
    // A reader arriving after the writer may not jump ahead of it, though r1 is only reading.
    let late = take(&env, "name:s", "r2", "shared");
    assert_eq!(late.code, 10, "{}", late.stdout);
    assert!(
        late.json()["holder"].is_null(),
        "only the queue is in the way"
    );
    let reader = env.spawn(&[
        "acquire",
        "name:s",
        "--owner",
        "r2",
        "--mode",
        "shared",
        "--timeout",
        "20s",
    ]);
    assert!(env.wait_for(SECS5, || env.tickets("name:s") == 2));

    // The graph shows the modes: the writer waits (exclusive) for a shared hold.
    let graph = env.run(&["graph", "--json"]).json();
    let edge = graph["edges"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["from"] == "w")
        .unwrap()
        .clone();
    assert_eq!(edge["mode"], "exclusive");
    assert_eq!(edge["to_mode"], "shared");
    let list = env.run(&["list", "--json"]).json();
    assert_eq!(list["locks"][0]["holder"]["mode"], "shared");
    assert_eq!(list["locks"][0]["queue"][0]["mode"], "exclusive");
    assert_eq!(list["locks"][0]["queue"][1]["mode"], "shared");

    assert_eq!(env.run(&["release", "name:s", "--owner", "r1"]).code, 0);
    assert_eq!(writer.finish(SECS5).code, 0, "the writer goes first");
    assert_eq!(
        modes(&env, "name:s"),
        [("w".to_string(), "exclusive".to_string())]
    );
    assert_eq!(env.run(&["release", "name:s", "--owner", "w"]).code, 0);
    assert_eq!(reader.finish(SECS5).code, 0);
    assert_eq!(env.run(&["release", "name:s", "--owner", "r2"]).code, 0);
    assert!(env.lock_dirs().is_empty(), "{:?}", env.lock_dirs());
}

#[test]
fn shared_waiters_are_granted_together() {
    let env = env("modes-together");
    assert_eq!(take(&env, "name:t", "w", "exclusive").code, 0);
    let readers: Vec<_> = (0..3)
        .map(|i| {
            let owner = format!("r{i}");
            env.spawn(&[
                "acquire",
                "name:t",
                "--owner",
                &owner,
                "--mode",
                "shared",
                "--timeout",
                "20s",
            ])
        })
        .collect();
    assert!(env.wait_for(SECS5, || env.tickets("name:t") == 3));
    assert_eq!(env.run(&["release", "name:t", "--owner", "w"]).code, 0);
    for r in readers {
        assert_eq!(r.finish(SECS5).code, 0);
    }
    assert_eq!(env.holds("name:t").len(), 3);
    for i in 0..3 {
        let owner = format!("r{i}");
        assert_eq!(env.run(&["release", "name:t", "--owner", &owner]).code, 0);
    }
    assert!(env.lock_dirs().is_empty(), "{:?}", env.lock_dirs());
}

#[test]
fn twelve_shared_racers_are_all_granted() {
    let env = env("modes-race");
    let barrier = Arc::new(Barrier::new(12));
    let racers: Vec<_> = (0..12)
        .map(|i| {
            let owner = format!("r{i}");
            let mut cmd = env.cmd(&[
                "acquire",
                "name:many",
                "--no-wait",
                "--mode",
                "shared",
                "--owner",
                &owner,
            ]);
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                cmd.output().unwrap().status.code()
            })
        })
        .collect();
    for r in racers {
        assert_eq!(r.join().unwrap(), Some(0));
    }
    assert_eq!(env.holds("name:many").len(), 12);
    for i in 0..12 {
        let owner = format!("r{i}");
        assert_eq!(
            env.run(&["release", "name:many", "--owner", &owner]).code,
            0
        );
    }
    assert!(env.lock_dirs().is_empty(), "{:?}", env.lock_dirs());
}

#[test]
fn an_owner_can_upgrade_once_the_other_readers_leave() {
    let env = env("modes-upgrade");
    assert_eq!(take(&env, "name:u", "a", "shared").code, 0);
    assert_eq!(take(&env, "name:u", "b", "shared").code, 0);
    assert_eq!(
        take(&env, "name:u", "a", "exclusive").code,
        10,
        "b still reads"
    );
    assert_eq!(
        take(&env, "name:u", "a", "shared").code,
        0,
        "asking again: granted"
    );
    assert_eq!(env.run(&["release", "name:u", "--owner", "b"]).code, 0);
    assert_eq!(take(&env, "name:u", "a", "exclusive").code, 0);
    let held: Vec<String> = env
        .holds("name:u")
        .iter()
        .map(|h| h["mode"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(held, ["shared", "exclusive"]);
    assert_eq!(take(&env, "name:u", "c", "shared").code, 10);
    assert_eq!(env.run(&["release", "name:u", "--owner", "a"]).code, 0);
    assert!(env.lock_dirs().is_empty(), "both holds of a went");
}

#[test]
fn list_and_status_text_show_the_mode() {
    let env = env("modes-text");
    assert_eq!(take(&env, "name:v", "r1", "shared").code, 0);
    assert_eq!(take(&env, "name:v", "r1/c", "exclusive").code, 0);
    let list = env.run(&["list"]).stdout;
    assert!(list.contains("MODE"), "{list}");
    assert!(list.contains("shared"), "{list}");
    assert!(list.contains("r1/c (nested)"), "{list}");
    assert!(list.contains("(nested) a hold inside"), "{list}");
    let status = env.run(&["status", "name:v"]).stdout;
    assert!(status.contains("by r1 (shared)"), "{status}");
    assert!(
        status.contains("nested in the hold of r1 on name:v"),
        "{status}"
    );
    let v: Value = env.run(&["status", "name:v", "--json"]).json();
    assert_eq!(v["holders"][1]["mode"], "exclusive");
    assert_eq!(env.run(&["release", "name:v", "--owner", "r1"]).code, 0);
    assert!(env.lock_dirs().is_empty(), "{:?}", env.lock_dirs());
}
