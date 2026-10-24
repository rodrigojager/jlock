mod common;

use common::{Env, Kid};
use serde_json::Value;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

const SECS5: Duration = Duration::from_secs(5);

fn env(name: &str) -> Env {
    let mut env = Env::new(name);
    env.set("JLOCK_STALE_MS", "20000");
    env
}

/// key -> (owner, group, queue owners), as `list --json` reports it.
fn listed(env: &Env, extra: &[&str]) -> BTreeMap<String, (Value, Value, Vec<Value>)> {
    let mut args = vec!["list", "--json"];
    args.extend_from_slice(extra);
    let v = env.run(&args).json();
    v["locks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| {
            let queue = l["queue"].as_array().unwrap();
            (
                l["key"].as_str().unwrap().to_string(),
                (
                    l["holder"]["owner"].clone(),
                    l["holder"]["group"].clone(),
                    queue.iter().map(|t| t["owner"].clone()).collect(),
                ),
            )
        })
        .collect()
}

/// key -> (owner, queue owners), read straight from the folder.
fn on_disk(env: &Env) -> BTreeMap<String, (Value, Vec<Value>)> {
    let mut out = BTreeMap::new();
    for dir in env.lock_dirs() {
        let dir = env.locks().join(dir);
        let key = std::fs::read_to_string(dir.join("key")).unwrap();
        let owner = std::fs::read_to_string(dir.join("holder.json"))
            .map(|t| serde_json::from_str::<Value>(&t).unwrap()["owner"].clone())
            .unwrap_or(Value::Null);
        let mut tickets: Vec<_> = std::fs::read_dir(dir.join("queue"))
            .map(|d| d.map(|e| e.unwrap().path()).collect())
            .unwrap_or_default();
        tickets.sort();
        let queue = tickets
            .iter()
            .map(|p| {
                serde_json::from_str::<Value>(&std::fs::read_to_string(p).unwrap()).unwrap()
                    ["owner"]
                    .clone()
            })
            .collect();
        out.insert(key, (owner, queue));
    }
    out
}

#[test]
fn list_matches_the_folder_with_or_without_the_index() {
    let env = env("list-folder");
    let p = env.path("proj");
    let a = env.path("proj/src/a.ts");
    let other = env.path("other/b.ts");
    for (key, owner, group) in [
        (&a, "alice", "g1"),
        (&other, "bob", "g2"),
        (&"name:port-3000".to_string(), "carol", "g1"),
    ] {
        let done = env.run(&[
            "acquire",
            key,
            "--no-wait",
            "--owner",
            owner,
            "--group",
            group,
            "--intent",
            "work",
        ]);
        assert_eq!(done.code, 0, "{}", done.stdout);
    }
    let waiter = env.spawn(&["acquire", "name:port-3000", "--owner", "dave"]);
    assert!(env.wait_for(SECS5, || env.tickets("name:port-3000") == 1));

    let listed_all = listed(&env, &[]);
    let disk = on_disk(&env);
    assert_eq!(listed_all.len(), 3);
    for (key, (owner, queue)) in &disk {
        let (l_owner, _, l_queue) = &listed_all[key];
        assert_eq!(l_owner, owner, "{key}");
        assert_eq!(l_queue, queue, "{key}");
    }
    assert_eq!(listed_all["name:port-3000"].2, vec![Value::from("dave")]);

    // The index is derived: written after changes, and its absence changes nothing.
    let index_path = env.home.join("index.json");
    assert!(index_path.exists());
    let index: Value =
        serde_json::from_str(&std::fs::read_to_string(&index_path).unwrap()).unwrap();
    assert_eq!(index["locks"].as_array().unwrap().len(), 3);
    std::fs::remove_file(&index_path).unwrap();
    assert_eq!(listed(&env, &[]), listed_all);
    std::fs::write(&index_path, "{ not json").unwrap();
    assert_eq!(listed(&env, &[]), listed_all);

    let under = listed(&env, &["--under", &p]);
    assert_eq!(under.len(), 1, "{under:?}");
    assert!(under.keys().next().unwrap().ends_with("proj/src/a.ts"));
    let g1 = listed(&env, &["--group", "g1"]);
    assert_eq!(g1.len(), 2);
    assert!(g1.contains_key("name:port-3000"));
    assert_eq!(listed(&env, &["--owner", "bob"]).len(), 1);
    assert_eq!(listed(&env, &["--stale"]).len(), 0);

    let text = env.run(&["list"]).stdout;
    assert!(text.starts_with("KEY"), "{text}");
    assert!(text.contains("alice") && text.contains("g2"), "{text}");

    // release --group frees the group's locks of that owner only.
    let done = env.run(&["release", "--group", "g1", "--owner", "alice", "--json"]);
    assert_eq!(done.code, 0);
    assert_eq!(done.json()["keys"].as_array().unwrap().len(), 1);
    assert_eq!(listed(&env, &["--group", "g1"]).len(), 1);
    let done = env.run(&["release", "--group", "g1", "--any-owner"]);
    assert_eq!(done.code, 0);
    assert_eq!(waiter.finish(SECS5).code, 0); // dave got the port once carol's lock went
    assert_eq!(env.run(&["release", "--owner", "dave"]).code, 0);
    assert_eq!(env.run(&["release", "--owner", "bob"]).code, 0);
    assert_eq!(env.run(&["list"]).stdout.trim(), "nothing locked");
    assert!(env.lock_dirs().is_empty(), "{:?}", env.lock_dirs());
    let index: Value =
        serde_json::from_str(&std::fs::read_to_string(&index_path).unwrap()).unwrap();
    assert!(index["locks"].as_array().unwrap().is_empty());
}

#[test]
fn stale_filter_shows_only_dead_locks() {
    let mut env = Env::new("list-stale");
    env.set("JLOCK_STALE_MS", "300");
    let mut hold = env.spawn(&["hold", "name:alive", "--owner", "a"]);
    assert_eq!(hold.read_line(), "granted name:alive");
    assert_eq!(
        env.run(&["acquire", "name:dead", "--no-wait", "--owner", "b"])
            .code,
        0
    );
    std::thread::sleep(Duration::from_millis(600));
    let stale = listed(&env, &["--stale"]);
    assert_eq!(stale.keys().collect::<Vec<_>>(), vec!["name:dead"]);
    let text = env.run(&["list"]).stdout;
    assert!(
        text.contains("(dead)") && !text.contains("(late)"),
        "{text}"
    );
    // `status` cleans up as it goes: the dead lock is expired and reported free.
    let st = env.run(&["status", "name:dead"]).stdout;
    assert!(st.starts_with("free"), "{st}");
    assert!(env.holder_owner("name:dead").is_none());
    hold.send("release");
    assert_eq!(hold.wait(SECS5), Some(0));
}

#[test]
fn release_while_list_watch_is_reading() {
    let env = env("list-watch");
    let mut watch: Kid = env.spawn(&["list", "--watch", "--json"]);
    let start = Instant::now();
    let mut n = 0;
    while start.elapsed() < Duration::from_millis(1500) {
        let key = format!("name:w{}", n % 3);
        let owner = format!("o{n}");
        assert_eq!(
            env.run(&["acquire", &key, "--no-wait", "--owner", &owner])
                .code,
            0
        );
        let done = env.run(&["release", &key, "--owner", &owner]);
        assert_eq!(
            done.code, 0,
            "release while list --watch reads: {}",
            done.stderr
        );
        n += 1;
    }
    assert!(watch.running(), "list --watch ended");
    let first = watch.read_line();
    assert!(first.contains("\"result\":\"ok\""), "{first}");
    watch.kill();
    assert!(env.lock_dirs().is_empty(), "{:?}", env.lock_dirs());
}

#[test]
fn keys_normalize_the_same_from_any_spelling() {
    let env = env("list-spelling");
    let lower = env.path("dir/file.txt");
    let done = env.run(&["acquire", &lower, "--no-wait", "--owner", "a"]);
    assert_eq!(done.code, 0);
    if cfg!(windows) {
        let upper = lower.to_uppercase();
        let slashes = lower.replace('\\', "/");
        for other in [upper, slashes] {
            let done = env.run(&["acquire", &other, "--no-wait", "--owner", "b", "--json"]);
            assert_eq!(done.code, 10, "{other}");
        }
    }
    // Relative and absolute spellings of the same file give the same key.
    let dir = std::path::Path::new(&lower).parent().unwrap().to_path_buf();
    std::fs::create_dir_all(&dir).unwrap();
    let mut cmd = env.cmd(&["acquire", "file.txt", "--no-wait", "--owner", "b"]);
    cmd.current_dir(&dir);
    assert_eq!(cmd.output().unwrap().status.code(), Some(10));
    assert_eq!(env.run(&["release", &lower, "--owner", "a"]).code, 0);
    assert!(env.lock_dirs().is_empty());
}

#[test]
fn log_rotates_and_stays_under_twice_the_limit() {
    let mut env = Env::new("list-rotation");
    env.set("JLOCK_LOG_MAX_BYTES", "3000");
    for i in 0..40 {
        let owner = format!("owner-{i}");
        assert_eq!(
            env.run(&["acquire", "name:rot", "--no-wait", "--owner", &owner])
                .code,
            0
        );
        assert_eq!(env.run(&["release", "name:rot", "--owner", &owner]).code, 0);
    }
    let size = |name: &str| std::fs::metadata(env.home.join(name)).map_or(0, |m| m.len());
    assert!(size("log.1.jsonl") > 0, "no rotation happened");
    assert!(size("log.jsonl") <= 3000 && size("log.1.jsonl") <= 3000);
    assert!(size("log.jsonl") + size("log.1.jsonl") < 6000);
    let log = std::fs::read_to_string(env.home.join("log.jsonl")).unwrap();
    for line in log.lines() {
        let v: Value = serde_json::from_str(line).unwrap();
        assert!(v["at"].is_string() && v["event"].is_string(), "{line}");
    }
}
