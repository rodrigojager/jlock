mod common;

use common::Env;
use serde_json::Value;
use std::time::Duration;

const SECS5: Duration = Duration::from_secs(5);

fn log_lines(env: &Env, event: &str) -> Vec<Value> {
    std::fs::read_to_string(env.home.join("log.jsonl"))
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .filter(|v| v["event"] == event)
        .collect()
}

#[test]
fn forced_hold_exits_14_and_the_log_says_who_and_why() {
    let env = Env::new("force-hold");
    let key = "name:stuck";
    let mut hold = env.spawn(&["hold", key, "--owner", "agent-1", "--group", "g"]);
    assert_eq!(hold.read_line(), format!("granted {key}"));

    let refused = env.run(&["unlock", key]);
    assert_eq!(refused.code, 2, "unlock without --force is refused");
    assert_eq!(env.holder_owner(key).as_deref(), Some("agent-1"));

    let done = env.run(&[
        "unlock",
        key,
        "--force",
        "--reason",
        "agent hung",
        "--by",
        "tester",
        "--json",
    ]);
    assert_eq!(done.code, 0, "{}", done.stdout);
    assert_eq!(done.json()["forced"][0]["owner"], "agent-1");
    assert_eq!(hold.wait(SECS5), Some(14));
    assert_eq!(hold.read_line(), format!("lost {key}"));

    let forced = log_lines(&env, "forced");
    assert_eq!(forced.len(), 1);
    assert_eq!(forced[0]["key"], key);
    assert_eq!(forced[0]["owner"], "agent-1");
    assert_eq!(forced[0]["by"], "tester");
    assert_eq!(forced[0]["reason"], "agent hung");
    assert_eq!(log_lines(&env, "lost").len(), 1);

    // Free for the next one.
    assert_eq!(
        env.run(&["acquire", key, "--no-wait", "--owner", "next"])
            .code,
        0
    );
    assert_eq!(env.run(&["release", key, "--owner", "next"]).code, 0);
    assert!(env.lock_dirs().is_empty(), "{:?}", env.lock_dirs());
}

#[test]
fn force_by_owner_and_by_group() {
    let mut env = Env::new("force-bulk");
    env.set("JLOCK_STALE_MS", "20000");
    for (key, owner, group) in [
        ("name:a", "x", "g1"),
        ("name:b", "x", "g2"),
        ("name:c", "y", "g1"),
        ("name:d", "z", "g3"),
    ] {
        let args = [
            "acquire",
            key,
            "--no-wait",
            "--owner",
            owner,
            "--group",
            group,
        ];
        assert_eq!(env.run(&args).code, 0);
    }
    let done = env.run(&["unlock", "--owner", "x", "--force", "--json"]);
    assert_eq!(done.json()["forced"].as_array().unwrap().len(), 2);
    let done = env.run(&["unlock", "--group", "g1", "--force", "--json"]);
    assert_eq!(done.json()["forced"].as_array().unwrap().len(), 1);
    assert_eq!(env.holder_owner("name:d").as_deref(), Some("z"));
    assert_eq!(log_lines(&env, "forced").len(), 3);
    assert!(log_lines(&env, "forced")[0]["by"]
        .as_str()
        .unwrap()
        .starts_with("user:"));
    assert_eq!(env.run(&["release", "name:d", "--owner", "z"]).code, 0);
    assert!(env.lock_dirs().is_empty(), "{:?}", env.lock_dirs());
}
