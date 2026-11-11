//! Regressions use only bounded children of this test binary and a private registry per test.
mod common;
use common::{Env, Kid};
use std::process::Command;
use std::time::{Duration, Instant};

#[test]
#[allow(clippy::zombie_processes)] // The fixture intentionally exits first; the supervisor owns its bounded leaf.
fn tree_helper() {
    let Ok(mode) = std::env::var("TREE_MODE") else {
        return;
    };
    let ready = std::env::var("TREE_READY").unwrap();
    let late = std::env::var("TREE_LATE").unwrap();
    if mode == "leaf" {
        std::fs::write(&ready, "ready").unwrap();
        std::thread::sleep(Duration::from_millis(1400));
        std::fs::write(&late, "escaped").unwrap();
        // Bounded even against the old implementation: never leave a permanent orphan on failure.
        std::thread::sleep(Duration::from_millis(600));
    } else {
        let mut leaf = Command::new(std::env::current_exe().unwrap())
            .args(["tree_helper", "--exact", "--nocapture"])
            .env("TREE_MODE", "leaf")
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while !std::path::Path::new(&ready).exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        if std::env::var("TREE_EXIT_PARENT").is_err() {
            let _ = leaf.wait();
        }
    }
}

fn command(env: &Env, locked: bool, deadline: bool) -> Command {
    let me = std::env::current_exe().unwrap();
    let mut cmd = if locked {
        env.cmd(&[
            "run",
            "name:tree",
            "--owner",
            "tree",
            "--",
            me.to_str().unwrap(),
            "tree_helper",
            "--exact",
            "--nocapture",
        ])
    } else {
        env.cmd(&[
            "exec",
            "--timeout",
            "700ms",
            "--",
            me.to_str().unwrap(),
            "tree_helper",
            "--exact",
            "--nocapture",
        ])
    };
    if locked && deadline {
        // Insert before the command separator, not among the payload's arguments.
        let args: Vec<_> = cmd.get_args().map(|a| a.to_os_string()).collect();
        let mut rebuilt = env.cmd(&[]);
        for a in args {
            if a == "--" {
                rebuilt.args(["--command-timeout", "700ms"]);
            }
            rebuilt.arg(a);
        }
        cmd = rebuilt;
    }
    if let Ok(bin) = std::env::var("JLOCK_TREE_TEST_BIN") {
        let args: Vec<_> = cmd.get_args().map(|a| a.to_os_string()).collect();
        cmd = Command::new(bin);
        cmd.args(args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        env.apply(&mut cmd);
    }
    cmd.env("TREE_MODE", "parent")
        .env("TREE_READY", env.home.join("ready"))
        .env("TREE_LATE", env.home.join("late"));
    cmd
}

fn assert_stopped(env: &Env) {
    std::thread::sleep(Duration::from_millis(1500));
    assert!(
        !env.home.join("late").exists(),
        "descendant wrote after supervisor completed"
    );
}

#[test]
fn timeout_closes_inherited_pipes_and_does_not_touch_another_process() {
    let env = Env::new("tree-timeout");
    let mut unrelated = Command::new(std::env::current_exe().unwrap())
        .args(["tree_helper", "--exact", "--nocapture"])
        .env("TREE_MODE", "leaf")
        .env("TREE_READY", env.home.join("other-ready"))
        .env("TREE_LATE", env.home.join("other-late"))
        .spawn()
        .unwrap();
    let started = Instant::now();
    let done = Kid::start(command(&env, false, true)).finish(Duration::from_secs(5));
    assert_eq!(done.code, 124, "{}", done.stderr);
    assert!(env.home.join("ready").exists(), "payload never started");
    assert!(
        started.elapsed() < Duration::from_millis(1400),
        "inherited pipe delayed completion"
    );
    assert!(
        unrelated.try_wait().unwrap().is_none(),
        "unrelated sibling was killed"
    );
    unrelated.wait().unwrap();
    assert!(env.home.join("other-late").exists());
    assert_stopped(&env);
}

#[test]
fn normal_wrapper_exit_also_cleans_descendants_before_unlocking() {
    let env = Env::new("tree-wrapper");
    let mut cmd = command(&env, true, false);
    cmd.env("TREE_EXIT_PARENT", "1");
    let started = Instant::now();
    let done = Kid::start(cmd).finish(Duration::from_secs(5));
    assert_eq!(done.code, 0, "{}", done.stderr);
    assert!(env.home.join("ready").exists());
    assert!(started.elapsed() < Duration::from_millis(1400));
    assert!(env.run(&["status", "name:tree"]).stdout.starts_with("free"));
    assert_stopped(&env);
}

#[test]
fn command_deadline_cleans_tree_before_releasing_lock() {
    let env = Env::new("tree-locked-timeout");
    let done = Kid::start(command(&env, true, true)).finish(Duration::from_secs(5));
    assert_eq!(done.code, 124, "{}", done.stderr);
    assert!(env.home.join("ready").exists());
    assert!(env.run(&["status", "name:tree"]).stdout.starts_with("free"));
    assert_stopped(&env);
}

#[test]
fn lock_loss_stops_descendants_too() {
    let env = Env::new("tree-lock-loss");
    let cmd = command(&env, true, false);
    let mut kid = Kid::start(cmd);
    assert!(env.wait_for(Duration::from_secs(3), || env.home.join("ready").exists()));
    assert_eq!(
        env.run(&["unlock", "name:tree", "--force", "--reason", "test"])
            .code,
        0
    );
    assert_eq!(kid.wait(Duration::from_secs(5)), Some(14));
    assert_stopped(&env);
}

#[cfg(windows)]
#[test]
fn hard_killed_supervisor_closes_its_private_job() {
    let env = Env::new("tree-supervisor-death");
    let mut kid = Kid::start(command(&env, false, true));
    assert!(env.wait_for(Duration::from_secs(3), || env.home.join("ready").exists()));
    kid.kill();
    assert_stopped(&env);
}

#[cfg(windows)]
#[test]
fn preserves_batch_execution_and_argument_escaping() {
    let env = Env::new("tree-batch");
    let batch = env.home.join("a script.cmd");
    std::fs::write(
        &batch,
        "@echo off\r\necho \"%~1\"\r\necho \"%~2\"\r\nexit /b 7\r\n",
    )
    .unwrap();
    let result = env.run(&[
        "exec",
        "--timeout",
        "5s",
        "--",
        batch.to_str().unwrap(),
        "a b",
        "x&echo injected",
    ]);
    assert_eq!(result.code, 7, "{}", result.stderr);
    assert_eq!(
        result.stdout.replace('\r', ""),
        "\"a b\"\n\"x&echo injected\"\n"
    );
}
