//! `jlock doctor`: 0 with no `fail`, 20 with one; warnings do not change the code; it never leaves a
//! file behind and never creates `JLOCK_HOME`.

mod common;

use common::Env;
use serde_json::Value;
use std::path::Path;

fn checks(done: &common::Done) -> Vec<Value> {
    done.json()["checks"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

fn status_of<'a>(checks: &'a [Value], name: &str) -> &'a str {
    checks
        .iter()
        .find(|c| c["name"] == name)
        .and_then(|c| c["status"].as_str())
        .unwrap_or_else(|| panic!("no check {name}: {checks:?}"))
}

/// Nothing named `doctor-*` is left in the registry folder.
fn no_leftovers(home: &Path) {
    if let Ok(entries) = std::fs::read_dir(home) {
        for e in entries {
            let name = e.unwrap().file_name().to_string_lossy().into_owned();
            assert!(!name.starts_with("doctor-"), "left behind: {name}");
        }
    }
}

#[test]
fn healthy_registry_is_all_ok() {
    let env = Env::new("doctor-ok");
    let done = env.run(&["doctor", "--json"]);
    assert_eq!(done.code, 0, "{}", done.stdout);
    let all = checks(&done);
    let names: Vec<&str> = all.iter().map(|c| c["name"].as_str().unwrap()).collect();
    assert_eq!(
        names,
        [
            "version",
            "home",
            "primitives",
            "oslock",
            "filesystem",
            "variables",
            "path",
            "registry",
            "curl"
        ]
    );
    for c in &all {
        let name = c["name"].as_str().unwrap();
        // The test binary is not the jlock in PATH, and a minimal system may lack curl.
        if name != "path" && name != "curl" {
            assert_eq!(c["status"], "ok", "{c}");
        }
    }
    assert_eq!(done.json()["result"], "ok");
    assert_eq!(done.json()["summary"]["fail"], 0);
    no_leftovers(&env.home);

    let text = env.run(&["doctor"]);
    assert_eq!(text.code, 0);
    assert!(text.stdout.contains("primitives"), "{}", text.stdout);
    assert!(text.stdout.contains("failures: 0"), "{}", text.stdout);
    no_leftovers(&env.home);
}

#[test]
fn home_that_is_a_file_fails() {
    let mut env = Env::new("doctor-file");
    let file = env.home.join("not-a-folder");
    std::fs::write(&file, "x").unwrap();
    env.set("JLOCK_HOME", &file.to_string_lossy());
    let done = env.run(&["doctor", "--json"]);
    assert_eq!(done.code, 20, "{}", done.stdout);
    assert_eq!(status_of(&checks(&done), "home"), "fail");
    assert_eq!(done.json()["result"], "error");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "x");
}

#[test]
fn missing_home_fails_and_is_not_created() {
    let mut env = Env::new("doctor-missing");
    let missing = env.home.join("nowhere");
    env.set("JLOCK_HOME", &missing.to_string_lossy());
    let done = env.run(&["doctor", "--json"]);
    assert_eq!(done.code, 20);
    let all = checks(&done);
    assert_eq!(status_of(&all, "home"), "fail");
    let fix = all[1]["fix"].as_str().unwrap();
    assert!(fix.contains("mkdir"), "{fix}");
    assert!(!missing.exists(), "the doctor created JLOCK_HOME");
}

#[test]
fn bad_times_are_a_warning_only() {
    let mut env = Env::new("doctor-times");
    env.set("JLOCK_STALE_MS", "100");
    env.set("JLOCK_HEARTBEAT_MS", "5000");
    let done = env.run(&["doctor", "--json"]);
    assert_eq!(done.code, 0, "{}", done.stdout);
    let all = checks(&done);
    assert_eq!(status_of(&all, "variables"), "warn");
    let detail = all[5]["detail"].as_str().unwrap();
    assert!(detail.contains("JLOCK_STALE_MS (100)"), "{detail}");

    env.set("JLOCK_STALE_MS", "soon");
    let done = env.run(&["doctor", "--json"]);
    assert_eq!(status_of(&checks(&done), "variables"), "warn");
}

#[test]
fn strange_folder_in_locks_suggests_gc() {
    let env = Env::new("doctor-debris");
    let odd = env.locks().join("0123456789abcdef");
    std::fs::create_dir_all(odd.join("queue")).unwrap();
    std::fs::write(odd.join("queue").join("x.json"), "{}").unwrap();
    let done = env.run(&["doctor", "--json"]);
    assert_eq!(done.code, 0);
    let all = checks(&done);
    assert_eq!(status_of(&all, "registry"), "warn");
    assert!(all[7]["fix"].as_str().unwrap().contains("jlock gc"));
    // The doctor suggests; it does not clean.
    assert!(odd.join("queue").join("x.json").exists());
}

#[test]
fn read_only_home_fails() {
    let env = Env::new("doctor-readonly");
    let dir = env.home.join("ro");
    std::fs::create_dir_all(&dir).unwrap();
    if !make_read_only(&dir) {
        eprintln!("cannot make a folder read-only here; skipped");
        return;
    }
    let mut env2 = Env::new("doctor-readonly-run");
    env2.set("JLOCK_HOME", &dir.to_string_lossy());
    let done = env2.run(&["doctor", "--json"]);
    undo_read_only(&dir);
    assert_eq!(done.code, 20, "{}", done.stdout);
    assert_eq!(status_of(&checks(&done), "home"), "fail");
    no_leftovers(&dir);
}

/// Denies creating files in `dir` (our own test folder). False if the system lets us write anyway.
#[cfg(windows)]
fn make_read_only(dir: &Path) -> bool {
    let user = std::env::var("USERNAME").unwrap_or_default();
    let ok = std::process::Command::new("icacls")
        .arg(dir)
        .args(["/deny", &format!("{user}:(WD,AD)")])
        .output()
        .is_ok_and(|o| o.status.success());
    ok && std::fs::write(dir.join("probe"), "x").is_err()
}

#[cfg(windows)]
fn undo_read_only(dir: &Path) {
    let user = std::env::var("USERNAME").unwrap_or_default();
    let _ = std::process::Command::new("icacls")
        .arg(dir)
        .args(["/remove:d", &user])
        .output();
}

#[cfg(unix)]
fn make_read_only(dir: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o555)).unwrap();
    // root writes anyway
    let writable = std::fs::write(dir.join("probe"), "x").is_ok();
    if writable {
        let _ = std::fs::remove_file(dir.join("probe"));
        undo_read_only(dir);
    }
    !writable
}

#[cfg(unix)]
fn undo_read_only(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755));
}
