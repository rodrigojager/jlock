mod common;

use common::{Env, Kid};
use std::thread::sleep;
use std::time::{Duration, Instant};

const SECS5: Duration = Duration::from_secs(5);

#[test]
fn grants_follow_arrival_order() {
    let env = Env::new("fifo-order");
    let key = "name:fifo";
    let mut kids: Vec<Kid> = Vec::new();
    let mut first_granted = Instant::now();
    for i in 0..8 {
        let owner = format!("w{i}");
        kids.push(env.spawn(&["acquire", key, "--owner", &owner, "--json"]));
        // Start delays of 0, 50, 100 ... ms, each one after the previous is surely in line.
        let registered = env.wait_for(SECS5, || {
            if i == 0 {
                env.holder_owner(key).as_deref() == Some("w0")
            } else {
                env.tickets(key) == i
            }
        });
        assert!(registered, "w{i} did not get in line");
        if i == 0 {
            first_granted = Instant::now();
        }
        sleep(Duration::from_millis(50));
    }
    // The first one holds for 1 s.
    sleep(Duration::from_secs(1).saturating_sub(first_granted.elapsed()));

    let mut order = vec!["w0".to_string()];
    loop {
        let current = order.last().unwrap().clone();
        let done = env.run(&["release", key, "--owner", &current]);
        assert_eq!(done.code, 0, "release {current}: {}", done.stderr);
        if order.len() == 8 {
            break;
        }
        let mut next = None;
        env.wait_for(SECS5, || {
            next = env.holder_owner(key).filter(|o| *o != current);
            next.is_some()
        });
        order.push(next.expect("next holder"));
    }
    let expected: Vec<String> = (0..8).map(|i| format!("w{i}")).collect();
    assert_eq!(order, expected);
    for kid in kids {
        let done = kid.finish(SECS5);
        assert_eq!(done.code, 0, "{}", done.stderr);
        assert_eq!(done.json()["result"], "granted");
    }
    assert!(env.lock_dirs().is_empty(), "{:?}", env.lock_dirs());
}

#[test]
fn timeout_gives_11_quickly_and_leaves_no_ticket() {
    let env = Env::new("fifo-timeout");
    let key = "name:t";
    assert_eq!(
        env.run(&["acquire", key, "--no-wait", "--owner", "a"]).code,
        0
    );
    let start = Instant::now();
    let done = env.run(&[
        "acquire",
        key,
        "--timeout",
        "300ms",
        "--owner",
        "b",
        "--json",
    ]);
    let took = start.elapsed();
    assert_eq!(done.code, 11, "{}", done.stdout);
    assert!(took < Duration::from_secs(1), "took {took:?}");
    let v = done.json();
    assert_eq!(v["result"], "timeout");
    assert_eq!(v["holder"]["owner"], "a");
    assert_eq!(v["position"], 1);
    assert_eq!(env.tickets(key), 0);
    assert_eq!(env.run(&["release", key, "--owner", "a"]).code, 0);
    assert!(env.lock_dirs().is_empty());
}

#[test]
fn no_wait_reports_position_without_a_ticket() {
    let env = Env::new("fifo-nowait");
    let key = "name:n";
    assert_eq!(
        env.run(&["acquire", key, "--no-wait", "--owner", "a"]).code,
        0
    );
    let waiter = env.spawn(&["acquire", key, "--owner", "b"]);
    assert!(env.wait_for(SECS5, || env.tickets(key) == 1));
    let done = env.run(&["acquire", key, "--no-wait", "--owner", "c", "--json"]);
    assert_eq!(done.code, 10);
    let v = done.json();
    assert_eq!(v["result"], "queued");
    assert_eq!(v["holder"]["owner"], "a");
    assert_eq!(v["position"], 2);
    assert_eq!(env.tickets(key), 1);
    // Free but with someone in line: --no-wait still respects the queue.
    assert_eq!(env.run(&["release", key, "--owner", "a"]).code, 0);
    assert_eq!(waiter.finish(SECS5).code, 0);
    assert_eq!(env.run(&["release", key, "--owner", "b"]).code, 0);
    assert!(env.lock_dirs().is_empty());
}

/// Sends the console interrupt to one child of ours. Windows: CTRL+BREAK to the child's own
/// process group (it was started in a new one), so it never reaches this test process.
fn interrupt(kid: &Kid) -> bool {
    #[cfg(windows)]
    // SAFETY: plain Win32 call; the group id is the child's PID, never 0 (which means everyone).
    unsafe {
        use windows_sys::Win32::System::Console::{GenerateConsoleCtrlEvent, CTRL_BREAK_EVENT};
        GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, kid.pid) != 0
    }
    #[cfg(unix)]
    // SAFETY: signals only our own child, by PID.
    unsafe {
        libc::kill(kid.pid as libc::pid_t, libc::SIGINT) == 0
    }
}

#[test]
fn interrupt_while_waiting_removes_the_ticket() {
    let env = Env::new("fifo-interrupt");
    let key = "name:i";
    assert_eq!(
        env.run(&["acquire", key, "--no-wait", "--owner", "a"]).code,
        0
    );
    let mut cmd = env.cmd(&["acquire", key, "--owner", "b"]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0000_0200); // CREATE_NEW_PROCESS_GROUP
    }
    let kid = Kid::start(cmd);
    assert!(env.wait_for(SECS5, || env.tickets(key) == 1));
    if !interrupt(&kid) {
        eprintln!("skipped: no console to deliver CTRL+BREAK in this environment");
        return;
    }
    let done = kid.finish(SECS5);
    assert_eq!(done.code, 130, "{}", done.stdout);
    assert_eq!(env.tickets(key), 0);
    assert_eq!(env.run(&["release", key, "--owner", "a"]).code, 0);
    assert!(env.lock_dirs().is_empty());
}
