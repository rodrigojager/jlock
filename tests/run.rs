mod common;

use common::{Env, Kid};
use std::time::{Duration, Instant};

const SECS5: Duration = Duration::from_secs(5);

/// The command `jlock run` executes in these tests: this very test binary, running only this
/// function, which sleeps and exits as told by `JLOCK_TEST_HELPER=<sleep_ms>:<exit_code>`.
#[test]
fn helper() {
    if let Ok(spec) = std::env::var("JLOCK_TEST_HELPER") {
        let (ms, code) = spec.split_once(':').unwrap();
        std::thread::sleep(Duration::from_millis(ms.parse().unwrap()));
        std::process::exit(code.parse().unwrap());
    }
}

fn run_cmd(env: &Env, key: &str, spec: &str) -> std::process::Command {
    let me = std::env::current_exe().unwrap();
    let me = me.to_string_lossy();
    let mut cmd = env.cmd(&[
        "run", key, "--owner", "runner", "--", &me, "helper", "--exact", "--quiet",
    ]);
    cmd.env("JLOCK_TEST_HELPER", spec);
    cmd
}

#[test]
fn run_holds_the_lock_while_the_command_runs_and_passes_its_code() {
    let env = Env::new("run-code");
    let key = "name:build";
    let started = Instant::now();
    let mut kid = Kid::start(run_cmd(&env, key, "1500:3"));
    assert!(env.wait_for(SECS5, || env.holder_owner(key).is_some()));
    let status = env.run(&["status", key]);
    assert!(status.stdout.starts_with("held"), "{}", status.stdout);
    assert_eq!(
        env.run(&["acquire", key, "--no-wait", "--owner", "x"]).code,
        10
    );
    assert_eq!(kid.wait(SECS5), Some(3));
    assert!(started.elapsed() >= Duration::from_millis(1500));
    assert!(env.run(&["status", key]).stdout.starts_with("free"));
    assert!(env.lock_dirs().is_empty(), "{:?}", env.lock_dirs());
}

#[test]
fn killed_run_lets_the_lock_expire() {
    let mut env = Env::new("run-killed");
    env.set("JLOCK_STALE_MS", "1000");
    let key = "name:killed";
    // The command outlives `run` by a little and then ends by itself.
    let mut kid = Kid::start(run_cmd(&env, key, "2500:0"));
    assert!(env.wait_for(SECS5, || env.holder_owner(key).is_some()));
    kid.kill();
    let done = env.run(&["acquire", key, "--timeout", "5s", "--owner", "next"]);
    assert_eq!(done.code, 0, "{}", done.stdout);
    assert_eq!(env.run(&["release", key, "--owner", "next"]).code, 0);
    assert!(env.lock_dirs().is_empty(), "{:?}", env.lock_dirs());
}

#[test]
fn run_without_command_is_a_usage_error() {
    let env = Env::new("run-usage");
    assert_eq!(env.run(&["run", "name:x"]).code, 2);
    assert_eq!(env.run(&["run", "name:x", "--"]).code, 2);
}
