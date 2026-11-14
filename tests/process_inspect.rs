//! `jlock process inspect`: present with an identity while our own child runs, absent once it has
//! exited and been reaped, absent/replaced for the identity of another incarnation, never absent
//! for a process the system does not let us open. The children are this test binary itself, ended
//! by closing their standard input; nothing is ever stopped by name.

mod common;

use common::Env;
use std::io::Read;
use std::process::{Child, Command, Stdio};

/// The child: waits until its standard input closes (also when the test dies), then exits.
#[test]
fn inspect_helper() {
    if std::env::var("INSPECT_HELPER").is_err() {
        return;
    }
    let mut sink = Vec::new();
    let _ = std::io::stdin().read_to_end(&mut sink);
}

fn child() -> Child {
    Command::new(std::env::current_exe().unwrap())
        .args(["inspect_helper", "--exact", "--nocapture"])
        .env("INSPECT_HELPER", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn helper")
}

/// Closes the child's input and reaps it.
fn end(mut c: Child) {
    drop(c.stdin.take());
    let status = c.wait().expect("reap helper");
    assert!(status.success(), "{status:?}");
}

fn inspect(env: &Env, args: &[&str]) -> serde_json::Value {
    let mut all = vec!["process", "inspect"];
    all.extend_from_slice(args);
    all.push("--json");
    let done = env.run(&all);
    assert_eq!(done.code, 0, "{all:?}: {}", done.stderr);
    done.json()
}

#[test]
fn a_live_child_is_present_and_absent_after_it_exits() {
    let env = Env::new("inspect-live");
    let kid = child();
    let pid = kid.id().to_string();
    let live = inspect(&env, &["--pid", &pid]);
    if !cfg!(windows) {
        assert_eq!(live["state"], "unknown");
        assert_eq!(live["reason"], "unsupported");
        end(kid);
        return;
    }
    assert_eq!(live["state"], "present", "{live}");
    let id = live["identity"].as_str().expect("identity").to_string();
    assert!(id.starts_with(&format!("win:{pid}:")), "{id}");
    // The same incarnation keeps its token.
    let again = inspect(&env, &["--pid", &pid, "--identity", &id]);
    assert_eq!(again["state"], "present", "{again}");
    assert_eq!(again["identity"], id.as_str());
    end(kid);
    let gone = inspect(&env, &["--pid", &pid]);
    assert_eq!(gone["state"], "absent", "{gone}");
    assert!(gone.get("identity").is_none(), "{gone}");
}

#[test]
fn another_incarnation_is_absent_replaced() {
    let env = Env::new("inspect-replaced");
    if !cfg!(windows) {
        return;
    }
    // The token of a child that has already ended, against a pid that is alive: no pid reuse needed.
    let old = child();
    let old_pid = old.id().to_string();
    let old_id = inspect(&env, &["--pid", &old_pid])["identity"]
        .as_str()
        .expect("identity of the first child")
        .to_string();
    end(old);
    let live = child();
    let pid = live.id().to_string();
    let answer = inspect(&env, &["--pid", &pid, "--identity", &old_id]);
    assert_eq!(answer["state"], "absent", "{answer}");
    assert_eq!(answer["reason"], "replaced", "{answer}");
    // The new process was never touched: still present under its own identity.
    let still = inspect(&env, &["--pid", &pid]);
    assert_eq!(still["state"], "present", "{still}");
    end(live);
}

#[test]
fn a_protected_process_is_never_absent() {
    let env = Env::new("inspect-protected");
    // pid 4 is the System process on Windows: always there, often not openable.
    let answer = inspect(&env, &["--pid", "4"]);
    assert_ne!(answer["state"], "absent", "{answer}");
    if answer["state"] == "unknown" {
        assert!(answer["reason"].is_string(), "{answer}");
    }
}

#[test]
fn usage_errors() {
    let env = Env::new("inspect-usage");
    for (args, text) in [
        (&["process"][..], "needs an action"),
        (&["process", "kill", "--pid", "4"][..], "unknown action"),
        (&["process", "inspect"][..], "needs --pid"),
        (&["process", "inspect", "--pid", "abc"][..], "bad pid"),
        (&["process", "inspect", "--pid", "0"][..], "bad pid"),
        (&["process", "inspect", "--pid", "4", "x"][..], "unexpected"),
    ] {
        let done = env.run(args);
        assert_eq!(done.code, 2, "{args:?}");
        assert!(done.stderr.contains(text), "{args:?}: {}", done.stderr);
    }
}
