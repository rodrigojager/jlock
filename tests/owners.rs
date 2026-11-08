//! Hierarchical owners (0.3): an owner is a `/`-separated path. A key held by an ancestor is
//! granted at once to a descendant (a nested hold, which skips the queue and ends with the hold it
//! is nested in); an ancestor is granted a key whose only other holders are its descendants;
//! siblings and unrelated owners conflict as before.
mod common;

use common::{Env, Kid};
use serde_json::Value;
use std::time::Duration;

const SECS5: Duration = Duration::from_secs(5);

fn env(name: &str) -> Env {
    let mut env = Env::new(name);
    // Plain acquires have no heartbeat: keep them alive for the whole test.
    env.set("JLOCK_STALE_MS", "20000");
    env
}

fn take(env: &Env, key: &str, owner: &str) -> common::Done {
    env.run(&["acquire", key, "--no-wait", "--owner", owner, "--json"])
}

fn hold(env: &Env, key: &str, owner: &str) -> Kid {
    let mut kid = env.spawn(&["hold", key, "--owner", owner]);
    let line = kid.read_line();
    assert!(line.starts_with("granted "), "{line}");
    kid
}

fn norm(env: &Env, key: &str) -> String {
    env.run(&["status", key, "--json"]).json()["key"]
        .as_str()
        .unwrap()
        .to_string()
}

fn status(env: &Env, key: &str) -> Value {
    env.run(&["status", key, "--json"]).json()
}

/// The jtasks deadlock, as a regression test: the task holds everything (`*`) or a folder, its
/// own exec child asks for a key under it and gets it at once, even with a sibling's exec already
/// waiting for that key; the sibling's exec keeps waiting for the task.
#[test]
fn jtasks_child_of_the_holder_is_granted_and_the_sibling_waits() {
    let env = env("owners-jtasks");
    for parent_key in ["*".to_string(), env.path("proj")] {
        let file = env.path("proj/src/a.ts");
        let mut task = hold(&env, &parent_key, "gt:r/T1");

        // A sibling's exec waits for the task's hold...
        let sibling = env.spawn(&[
            "acquire",
            &file,
            "--owner",
            "gt:r/T2/exec-1",
            "--timeout",
            "20s",
        ]);
        assert!(env.wait_for(SECS5, || env.tickets(&norm(&env, &file)) == 1));

        // ...and the task's own exec, coming later, is granted at once, nested in the task's hold.
        let child = take(&env, &file, "gt:r/T1/exec-1");
        assert_eq!(child.code, 0, "{}", child.stdout);
        let v = child.json();
        assert_eq!(v["result"], "granted");
        assert_eq!(v["locks"][0]["nested"], true);
        assert_eq!(v["locks"][0]["parent"]["owner"], "gt:r/T1");
        assert_eq!(
            v["locks"][0]["parent"]["key"],
            norm(&env, &parent_key).as_str()
        );

        // Another sibling asking now is busy, held by the task.
        let busy = take(&env, &file, "gt:r/T2/exec-2");
        assert_eq!(busy.code, 10, "{}", busy.stdout);
        let held_by = busy.json()["holder"]["owner"].as_str().unwrap().to_string();
        assert!(held_by.starts_with("gt:r/T1"), "{held_by}");

        let st = status(&env, &file);
        assert_eq!(st["holders"].as_array().unwrap().len(), 1);
        assert_eq!(st["holder"]["owner"], "gt:r/T1/exec-1");
        assert_eq!(st["holder"]["nested"], true);

        // The child's release does not touch the task; the task's release lets the sibling in.
        assert_eq!(
            env.run(&["release", &file, "--owner", "gt:r/T1/exec-1"])
                .code,
            0
        );
        assert!(sibling_still_waiting(&env, &file));
        task.send("release");
        assert_eq!(task.wait(SECS5), Some(0));
        assert_eq!(sibling.finish(SECS5).code, 0);
        assert_eq!(
            env.run(&["release", &file, "--owner", "gt:r/T2/exec-1"])
                .code,
            0
        );
        assert!(env.lock_dirs().is_empty(), "{:?}", env.lock_dirs());
    }
}

fn sibling_still_waiting(env: &Env, file: &str) -> bool {
    env.tickets(&norm(env, file)) == 1
}

#[test]
fn same_owner_asking_again_is_granted_without_a_new_hold() {
    let env = env("owners-same");
    assert_eq!(take(&env, "name:x", "a/b").code, 0);
    let again = take(&env, "name:x", "a/b");
    assert_eq!(again.code, 0);
    assert!(again.json()["locks"][0]["grant"].is_null(), "no new grant");
    assert_eq!(env.holds("name:x").len(), 1);
    assert_eq!(env.run(&["release", "name:x", "--owner", "a/b"]).code, 0);
    assert!(env.lock_dirs().is_empty());
}

#[test]
fn ancestor_is_granted_when_only_its_descendants_are_in_the_way() {
    let env = env("owners-ancestor");
    let dir = env.path("p/src");
    let a = env.path("p/src/a.ts");
    let b = env.path("p/src/b.ts");
    assert_eq!(take(&env, &a, "run/T1/exec").code, 0);
    let v = take(&env, &dir, "run/T1");
    assert_eq!(v.code, 0, "{}", v.stdout);
    assert_eq!(v.json()["locks"][0]["nested"], false);
    assert_eq!(env.run(&["release", &dir, "--owner", "run/T1"]).code, 0);

    // With an unrelated holder inside too, the ancestor waits.
    assert_eq!(take(&env, &b, "run/T2").code, 0);
    let busy = take(&env, &dir, "run/T1");
    assert_eq!(busy.code, 10);
    assert_eq!(busy.json()["holder"]["owner"], "run/T2");
    for (k, o) in [(&a, "run/T1/exec"), (&b, "run/T2")] {
        assert_eq!(env.run(&["release", k, "--owner", o]).code, 0);
    }
    assert!(env.lock_dirs().is_empty(), "{:?}", env.lock_dirs());
}

#[test]
fn releasing_the_parent_ends_the_nested_holds_and_not_the_other_way() {
    let env = env("owners-cascade");
    assert_eq!(take(&env, "name:x", "p").code, 0);
    assert_eq!(take(&env, "name:x", "p/c").code, 0);
    assert_eq!(take(&env, "name:x", "p/c/g").code, 0);
    let st = status(&env, "name:x");
    let owners: Vec<&str> = st["holders"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["owner"].as_str().unwrap())
        .collect();
    assert_eq!(owners, ["p", "p/c", "p/c/g"]);
    assert_eq!(st["holders"][1]["parent"]["owner"], "p");
    assert_eq!(st["holders"][2]["parent"]["owner"], "p/c");

    // The child gives its hold back: the parent keeps its own, and the grandchild (nested in the
    // child's hold) ends with it.
    assert_eq!(env.run(&["release", "name:x", "--owner", "p/c"]).code, 0);
    assert_eq!(env.holder_owners("name:x"), ["p"]);

    // Nested again, then the parent goes: everything nested in it goes too.
    assert_eq!(take(&env, "name:x", "p/c").code, 0);
    let path_key = env.path("q/file");
    assert_eq!(
        take(&env, &path_key, "p/c").code,
        0,
        "not nested: p holds name:x"
    );
    assert_eq!(env.run(&["release", "name:x", "--owner", "p"]).code, 0);
    assert!(env.holder_owners("name:x").is_empty());
    assert_eq!(
        env.holder_owner(&norm(&env, &path_key)).as_deref(),
        Some("p/c")
    );
    let log = std::fs::read_to_string(env.home.join("log.jsonl")).unwrap();
    assert!(log.contains("\"reason\":\"parent\""), "{log}");
    assert_eq!(env.run(&["release", "--owner", "p/c"]).code, 0);
    assert!(env.lock_dirs().is_empty(), "{:?}", env.lock_dirs());
}

#[test]
fn a_hold_nested_in_a_released_parent_is_lost() {
    let env = env("owners-lost");
    let mut parent = hold(&env, "name:k", "t");
    let mut child = env.spawn(&["hold", "name:k", "--owner", "t/exec", "--json"]);
    let line: Value = serde_json::from_str(&child.read_line()).unwrap();
    assert_eq!(line["locks"][0]["nested"], true);
    parent.send("release");
    assert_eq!(parent.wait(SECS5), Some(0));
    assert_eq!(
        child.wait(SECS5),
        Some(14),
        "the nested hold ended with its parent"
    );
    let lost: Value = serde_json::from_str(&child.read_line()).unwrap();
    assert_eq!(lost["result"], "lost");
    assert!(env.lock_dirs().is_empty(), "{:?}", env.lock_dirs());
}

#[test]
fn parent_dead_or_forced_ends_the_nested_holds() {
    let mut env = Env::new("owners-dead");
    env.set("JLOCK_STALE_MS", "1500");
    // The parent dies (an acquire without heartbeat goes stale); the nested hold, kept alive by a
    // hold process, ends with it as soon as someone looks.
    assert_eq!(take(&env, "name:d", "P").code, 0);
    let mut child = env.spawn(&["hold", "name:d", "--owner", "P/c"]);
    assert!(child.read_line().starts_with("granted name:d"));
    std::thread::sleep(Duration::from_millis(1700));
    let next = env.run(&[
        "acquire",
        "name:d",
        "--owner",
        "Q",
        "--timeout",
        "5s",
        "--json",
    ]);
    assert_eq!(next.code, 0, "{}", next.stdout);
    assert_eq!(child.wait(SECS5), Some(14));
    assert_eq!(env.run(&["release", "name:d", "--owner", "Q"]).code, 0);

    // unlock --force of the parent ends the child too.
    assert_eq!(take(&env, "name:f", "P").code, 0);
    assert_eq!(take(&env, "name:f", "P/c").code, 0);
    let forced = env.run(&["unlock", "--owner", "P", "--force", "--json"]);
    assert_eq!(forced.code, 0);
    assert!(env.holder_owners("name:f").is_empty());
    assert!(env.lock_dirs().is_empty(), "{:?}", env.lock_dirs());
}

#[test]
fn release_group_takes_the_nested_holds_of_the_group() {
    let env = env("owners-group");
    let run = |args: &[&str]| env.run(args).code;
    assert_eq!(
        run(&["acquire", "name:a", "--owner", "g/T1", "--group", "g1"]),
        0
    );
    assert_eq!(
        run(&["acquire", "name:a", "--owner", "g/T1/x", "--group", "g1"]),
        0
    );
    assert_eq!(
        run(&["acquire", "name:b", "--owner", "g/T2", "--group", "g2"]),
        0
    );
    assert_eq!(run(&["release", "--group", "g1", "--any-owner"]), 0);
    assert!(env.holder_owners("name:a").is_empty());
    assert_eq!(env.holder_owners("name:b"), ["g/T2"]);
    assert_eq!(run(&["release", "--owner", "g/T2"]), 0);
    assert!(env.lock_dirs().is_empty(), "{:?}", env.lock_dirs());
}

/// A descendant's wait is its ancestor's wait: P holds x; its child waits for y, held by S; S
/// asking for x would wait for P, which waits for its child: a deadlock, refused.
#[test]
fn deadlock_through_a_descendant_is_refused() {
    let env = env("owners-deadlock");
    assert_eq!(take(&env, "name:x", "P").code, 0);
    assert_eq!(take(&env, "name:y", "S").code, 0);
    let child = env.spawn(&["acquire", "name:y", "--owner", "P/exec", "--timeout", "20s"]);
    assert!(env.wait_for(SECS5, || env.tickets("name:y") == 1));
    let done = env.run(&["acquire", "name:x", "--owner", "S", "--json"]);
    assert_eq!(done.code, 12, "{}", done.stdout);
    let chain = done.json()["chain"].as_str().unwrap().to_string();
    assert!(chain.contains("P ⊃ P/exec"), "{chain}");
    let graph = env.run(&["graph", "--json"]).json();
    assert_eq!(graph["edges"][0]["from"], "P/exec");
    assert_eq!(graph["edges"][0]["to"], "S");
    assert_eq!(env.run(&["release", "name:y", "--owner", "S"]).code, 0);
    assert_eq!(child.finish(SECS5).code, 0);
    for (k, o) in [("name:x", "P"), ("name:y", "P/exec")] {
        assert_eq!(env.run(&["release", k, "--owner", o]).code, 0);
    }
    assert!(env.lock_dirs().is_empty(), "{:?}", env.lock_dirs());
}

#[test]
fn siblings_queue_in_arrival_order_as_before() {
    let env = env("owners-fifo");
    assert_eq!(take(&env, "name:q", "r/T1").code, 0);
    let first = env.spawn(&["acquire", "name:q", "--owner", "r/T2", "--json"]);
    assert!(env.wait_for(SECS5, || env.tickets("name:q") == 1));
    let second = env.spawn(&["acquire", "name:q", "--owner", "r/T3", "--json"]);
    assert!(env.wait_for(SECS5, || env.tickets("name:q") == 2));
    assert_eq!(env.run(&["release", "name:q", "--owner", "r/T1"]).code, 0);
    assert_eq!(first.finish(SECS5).code, 0);
    assert_eq!(env.holder_owner("name:q").as_deref(), Some("r/T2"));
    assert_eq!(env.run(&["release", "name:q", "--owner", "r/T2"]).code, 0);
    assert_eq!(second.finish(SECS5).code, 0);
    assert_eq!(env.run(&["release", "name:q", "--owner", "r/T3"]).code, 0);
    assert!(env.lock_dirs().is_empty(), "{:?}", env.lock_dirs());
}

#[test]
fn bad_owner_paths_are_refused() {
    let env = env("owners-bad");
    for bad in ["a//b", "/a", "a/", "a/ /b"] {
        let done = env.run(&["acquire", "name:x", "--owner", bad]);
        assert_eq!(done.code, 2, "{bad}: {}", done.stderr);
        assert!(done.stderr.contains("bad owner"), "{}", done.stderr);
    }
}
