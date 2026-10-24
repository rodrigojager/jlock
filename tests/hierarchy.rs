mod common;

use common::Env;
use std::time::Duration;

const SECS5: Duration = Duration::from_secs(5);

fn env(name: &str) -> Env {
    let mut env = Env::new(name);
    // Plain acquires have no heartbeat: keep them alive for the whole test.
    env.set("JLOCK_STALE_MS", "20000");
    env
}

fn take(env: &Env, key: &str, owner: &str) -> i32 {
    env.run(&["acquire", key, "--no-wait", "--owner", owner])
        .code
}

fn free(env: &Env, key: &str, owner: &str) {
    let done = env.run(&["release", key, "--owner", owner]);
    assert_eq!(
        done.code, 0,
        "release {key}: {}{}",
        done.stdout, done.stderr
    );
}

/// The normalized text of a key, as jlock sees it.
fn norm(env: &Env, key: &str) -> String {
    env.run(&["status", key, "--json"]).json()["key"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn folder_blocks_files_inside_and_vice_versa() {
    let env = env("hier-paths");
    let src = env.path("p/src");
    let file = env.path("p/src/a.ts");
    let docs = env.path("p/docs");

    assert_eq!(take(&env, &src, "a"), 0);
    let done = env.run(&["acquire", &file, "--no-wait", "--owner", "b", "--json"]);
    assert_eq!(done.code, 10);
    let v = done.json();
    assert_eq!(v["via"], norm(&env, &src).as_str());
    assert_eq!(v["holder"]["owner"], "a");
    assert_eq!(take(&env, &docs, "b"), 0, "a sibling folder is not blocked");

    // status of the file shows the folder above as a conflict.
    let st = env.run(&["status", &file]);
    assert!(st.stdout.starts_with("free"), "{}", st.stdout);
    assert!(st.stdout.contains("conflicts:"), "{}", st.stdout);
    assert!(st.stdout.contains("held by a"), "{}", st.stdout);

    // A waiter on the file gets it when the folder is released.
    let waiter = env.spawn(&["acquire", &file, "--owner", "c", "--json"]);
    assert!(env.wait_for(SECS5, || env.tickets(&norm(&env, &file)) == 1));
    free(&env, &src, "a");
    let done = waiter.finish(SECS5);
    assert_eq!(done.code, 0, "{}", done.stdout);

    // And now the folder waits for the file inside it.
    let done = env.run(&["acquire", &src, "--no-wait", "--owner", "a", "--json"]);
    assert_eq!(done.code, 10);
    assert_eq!(done.json()["via"], norm(&env, &file).as_str());
    let st = env.run(&["status", &src, "--json"]).json();
    assert_eq!(st["conflicts"][0]["key"], norm(&env, &file).as_str());
    assert_eq!(st["conflicts"][0]["holder"]["owner"], "c");

    free(&env, &file, "c");
    free(&env, &docs, "b");
    assert_eq!(take(&env, &src, "a"), 0);
    free(&env, &src, "a");
    assert!(env.lock_dirs().is_empty(), "{:?}", env.lock_dirs());
}

#[test]
fn names_only_conflict_with_the_same_name() {
    let env = env("hier-names");
    assert_eq!(take(&env, "name:x", "a"), 0);
    assert_eq!(take(&env, "name:x", "b"), 10);
    assert_eq!(take(&env, "name:xy", "b"), 0);
    assert_eq!(take(&env, "name:y", "b"), 0);
    assert_eq!(take(&env, &env.path("x"), "b"), 0);
    free(&env, "name:x", "a");
    free(&env, "name:xy", "b");
    free(&env, "name:y", "b");
    free(&env, &env.path("x"), "b");
    assert!(env.lock_dirs().is_empty(), "{:?}", env.lock_dirs());
}

#[test]
fn star_blocks_everything_and_is_blocked_by_anything() {
    let env = env("hier-star");
    assert_eq!(take(&env, "*", "a"), 0);
    assert_eq!(take(&env, "name:x", "b"), 10);
    assert_eq!(take(&env, &env.path("q/file"), "b"), 10);
    free(&env, "*", "a");

    assert_eq!(take(&env, "name:x", "b"), 0);
    let done = env.run(&["acquire", "*", "--no-wait", "--owner", "a", "--json"]);
    assert_eq!(done.code, 10);
    assert_eq!(done.json()["via"], "name:x");
    // A waiting `*` is served once the other lock goes.
    let waiter = env.spawn(&["acquire", "*", "--owner", "a"]);
    assert!(env.wait_for(SECS5, || env.tickets("*") == 1));
    free(&env, "name:x", "b");
    assert_eq!(waiter.finish(SECS5).code, 0);
    free(&env, "*", "a");
    assert!(env.lock_dirs().is_empty(), "{:?}", env.lock_dirs());
}
