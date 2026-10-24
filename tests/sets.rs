mod common;

use common::{Env, Kid};
use std::time::Duration;

const SECS10: Duration = Duration::from_secs(10);

/// The command `jlock run` executes: this test binary, running only this function, which sleeps
/// for `JLOCK_TEST_HELPER` milliseconds.
#[test]
fn helper() {
    if let Ok(ms) = std::env::var("JLOCK_TEST_HELPER") {
        std::thread::sleep(Duration::from_millis(ms.parse().unwrap()));
    }
}

fn run_set(env: &Env, keys: &[&str], owner: &str) -> Kid {
    let me = std::env::current_exe().unwrap();
    let me = me.to_string_lossy().into_owned();
    let mut args = vec!["run"];
    args.extend_from_slice(keys);
    args.extend_from_slice(&["--owner", owner, "--", &me, "helper", "--exact", "--quiet"]);
    let mut cmd = env.cmd(&args);
    cmd.env("JLOCK_TEST_HELPER", "150");
    Kid::start(cmd)
}

#[test]
fn opposite_orders_never_get_stuck() {
    let env = Env::new("sets-order");
    for round in 0..6 {
        let ab = run_set(&env, &["name:a", "name:b"], &format!("ab{round}"));
        let ba = run_set(&env, &["name:b", "name:a"], &format!("ba{round}"));
        let ab = ab.finish(SECS10);
        let ba = ba.finish(SECS10);
        assert_eq!(ab.code, 0, "round {round}: {}", ab.stderr);
        assert_eq!(ba.code, 0, "round {round}: {}", ba.stderr);
    }
    assert!(env.lock_dirs().is_empty(), "{:?}", env.lock_dirs());
}

#[test]
fn all_or_nothing() {
    let mut env = Env::new("sets-all-or-nothing");
    env.set("JLOCK_STALE_MS", "20000");
    assert_eq!(
        env.run(&["acquire", "name:b", "--no-wait", "--owner", "other"])
            .code,
        0
    );
    let done = env.run(&[
        "acquire",
        "name:c",
        "name:a",
        "name:b",
        "--no-wait",
        "--owner",
        "me",
        "--json",
    ]);
    assert_eq!(done.code, 10, "{}", done.stdout);
    let v = done.json();
    assert_eq!(v["failed"], "name:b");
    assert_eq!(v["released"], serde_json::json!(["name:a"]));
    assert_eq!(v["keys"], serde_json::json!(["name:a", "name:b", "name:c"]));
    assert!(env.holder_owner("name:a").is_none());
    assert!(env.holder_owner("name:c").is_none());

    assert_eq!(env.run(&["release", "name:b", "--owner", "other"]).code, 0);
    let done = env.run(&[
        "acquire", "name:c", "name:a", "name:b", "--owner", "me", "--json",
    ]);
    assert_eq!(done.code, 0);
    assert_eq!(
        done.json()["keys"],
        serde_json::json!(["name:a", "name:b", "name:c"])
    );
    let done = env.run(&["release", "name:a", "name:b", "name:c", "--owner", "me"]);
    assert_eq!(done.code, 0);
    assert!(env.lock_dirs().is_empty(), "{:?}", env.lock_dirs());
}
