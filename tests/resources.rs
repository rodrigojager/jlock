//! `resource:` keys (0.3): names from a catalogue (built-in, plus the user's `resources.json` in
//! the registry folder); a typo is refused with a suggestion; a resource only conflicts with the
//! very same resource and argument, never with paths or `name:` keys (only `*` covers it).
mod common;

use common::Env;

fn env(name: &str) -> Env {
    let mut env = Env::new(name);
    env.set("JLOCK_STALE_MS", "20000");
    env
}

fn take(env: &Env, key: &str, owner: &str) -> common::Done {
    env.run(&["acquire", key, "--no-wait", "--owner", owner, "--json"])
}

#[test]
fn a_typo_is_refused_with_a_suggestion() {
    let env = env("res-typo");
    let done = env.run(&["acquire", "resource:desktp", "--owner", "a"]);
    assert_eq!(done.code, 2);
    assert!(
        done.stderr.contains("did you mean resource:desktop?"),
        "{}",
        done.stderr
    );
    let done = env.run(&["acquire", "resource:zzzzzzzzzz", "--owner", "a", "--json"]);
    assert_eq!(done.code, 2);
    assert!(done.json()["error"]
        .as_str()
        .unwrap()
        .contains("unknown resource"));
    // status refuses it too; release does not need the catalogue.
    assert_eq!(env.run(&["status", "resource:clipbord"]).code, 2);
    assert_eq!(
        env.run(&["release", "resource:clipbord", "--owner", "a"])
            .code,
        13
    );
    assert!(env.lock_dirs().is_empty(), "nothing was created");
}

#[test]
fn arguments_are_checked() {
    let env = env("res-args");
    for (key, why) in [
        ("resource:port", "needs an argument"),
        ("resource:port:99999", "bad port"),
        ("resource:port:http", "bad port"),
        ("resource:desktop:2", "takes no argument"),
        ("resource:", "empty name"),
    ] {
        let done = env.run(&["acquire", key, "--owner", "a"]);
        assert_eq!(done.code, 2, "{key}");
        assert!(done.stderr.contains(why), "{key}: {}", done.stderr);
    }
    let done = take(&env, "resource:Port:08080", "a");
    assert_eq!(done.code, 0, "{}", done.stdout);
    assert_eq!(done.json()["key"], "resource:port:8080");
    assert_eq!(
        env.run(&["release", "resource:port:8080", "--owner", "a"])
            .code,
        0
    );
}

#[test]
fn resources_paths_and_names_never_conflict() {
    let env = env("res-kinds");
    let path = env.path("desktop");
    assert_eq!(take(&env, "resource:desktop", "a").code, 0);
    assert_eq!(take(&env, "name:desktop", "b").code, 0);
    assert_eq!(take(&env, "name:resource:desktop", "b").code, 0);
    assert_eq!(take(&env, &path, "c").code, 0);
    assert_eq!(take(&env, "resource:port:8080", "a").code, 0);
    assert_eq!(
        take(&env, "resource:port:8081", "b").code,
        0,
        "another port"
    );
    let busy = take(&env, "resource:desktop", "z");
    assert_eq!(busy.code, 10, "the same resource");
    assert_eq!(busy.json()["holder"]["owner"], "a");
    assert_eq!(take(&env, "resource:port:8080", "z").code, 10);
    // `*` still covers everything, resources included.
    assert_eq!(take(&env, "*", "z").code, 10);
    for o in ["a", "b", "c"] {
        assert_eq!(env.run(&["release", "--owner", o]).code, 0);
    }
    assert_eq!(take(&env, "*", "z").code, 0);
    assert_eq!(take(&env, "resource:machine", "y").code, 10);
    assert_eq!(env.run(&["release", "*", "--owner", "z"]).code, 0);
    assert!(env.lock_dirs().is_empty(), "{:?}", env.lock_dirs());
}

#[test]
fn a_quiet_machine_is_resource_machine_exclusive() {
    let env = env("res-machine");
    let run = |args: &[&str]| env.run(args).code;
    assert_eq!(
        run(&[
            "acquire",
            "resource:machine@shared",
            "--owner",
            "build-1",
            "--no-wait"
        ]),
        0
    );
    assert_eq!(
        run(&[
            "acquire",
            "resource:machine",
            "--mode",
            "shared",
            "--owner",
            "build-2",
            "--no-wait"
        ]),
        0
    );
    assert_eq!(
        run(&[
            "acquire",
            "resource:machine",
            "--owner",
            "bench",
            "--no-wait"
        ]),
        10
    );
    // Other projects' locks are not affected by it.
    assert_eq!(take(&env, "name:deploy", "bench").code, 0);
    for o in ["build-1", "build-2", "bench"] {
        assert_eq!(env.run(&["release", "--owner", o]).code, 0);
    }
    assert!(env.lock_dirs().is_empty(), "{:?}", env.lock_dirs());
}

#[test]
fn the_catalogue_lists_builtin_and_user_resources() {
    let env = env("res-list");
    let list = env.run(&["resources", "--json"]).json();
    let names: Vec<&str> = list["resources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["name"].as_str().unwrap())
        .collect();
    for n in ["desktop", "clipboard", "port", "machine", "gpu", "browser"] {
        assert!(names.contains(&n), "{n} in {names:?}");
    }
    let port = list["resources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == "port")
        .unwrap()
        .clone();
    assert_eq!(port["key"], "resource:port:<n>");
    assert_eq!(port["source"], "builtin");
    let text = env.run(&["resources"]).stdout;
    assert!(text.contains("desktop"), "{text}");
    assert!(text.contains("No resources of your own"), "{text}");

    std::fs::write(
        env.home.join("resources.json"),
        r#"{ "resources": {
            "db": { "description": "the local test database", "mode": "shared" },
            "vm": { "description": { "en": "a VM", "pt-BR": "uma VM" }, "arg": "name" },
            "desktop": { "description": "not allowed" } } }"#,
    )
    .unwrap();
    let list = env.run(&["resources", "--json"]);
    let v = list.json();
    let db = v["resources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == "db")
        .unwrap()
        .clone();
    assert_eq!(db["source"], "user");
    assert_eq!(db["mode"], "shared");
    assert_eq!(
        v["warnings"].as_array().unwrap().len(),
        1,
        "desktop is built in"
    );
    let pt = env.run(&["--lang", "pt-BR", "resources"]).stdout;
    assert!(pt.contains("uma VM"), "{pt}");

    // The resource's own default mode: two takers of the shared db coexist.
    assert_eq!(take(&env, "resource:db", "a").code, 0);
    let second = take(&env, "resource:db", "b");
    assert_eq!(second.code, 0, "{}", second.stdout);
    assert_eq!(second.json()["mode"], "shared");
    assert_eq!(take(&env, "resource:vm:web-1", "a").code, 0);
    assert_eq!(env.run(&["acquire", "resource:vm", "--owner", "a"]).code, 2);
    for o in ["a", "b"] {
        assert_eq!(env.run(&["release", "--owner", o]).code, 0);
    }
    assert!(env.lock_dirs().is_empty(), "{:?}", env.lock_dirs());
}
