mod common;

use common::{Env, Kid};
use std::time::Duration;

const SECS5: Duration = Duration::from_secs(5);

fn env(name: &str) -> Env {
    let mut env = Env::new(name);
    env.set("JLOCK_STALE_MS", "20000");
    env
}

/// A `hold` of `key` by `owner`, running until told to release.
fn hold(env: &Env, key: &str, owner: &str) -> Kid {
    let mut kid = env.spawn(&["hold", key, "--owner", owner]);
    assert_eq!(kid.read_line(), format!("granted {key}"));
    kid
}

fn let_go(mut kid: Kid) {
    kid.send("release");
    assert_eq!(kid.wait(SECS5), Some(0));
}

#[test]
fn sequential_request_closing_a_cycle_is_refused() {
    let env = env("deadlock-seq");
    let a = hold(&env, "name:x", "A");
    let b = hold(&env, "name:y", "B");
    let a_wants_y = env.spawn(&["acquire", "name:y", "--owner", "A", "--json"]);
    assert!(env.wait_for(SECS5, || env.tickets("name:y") == 1));

    let done = env.run(&["acquire", "name:x", "--owner", "B", "--json"]);
    assert_eq!(done.code, 12, "{}", done.stdout);
    let v = done.json();
    assert_eq!(v["result"], "deadlock");
    assert_eq!(
        v["chain"],
        "B → name:x (held by A) → A → name:y (held by B) → B"
    );
    assert_eq!(env.tickets("name:x"), 0, "refused before queueing");

    let graph = env.run(&["graph"]);
    assert!(graph.stdout.contains("A -> B"), "{}", graph.stdout);
    assert!(graph.stdout.contains("no cycles"), "{}", graph.stdout);

    let_go(b);
    assert_eq!(a_wants_y.finish(SECS5).code, 0);
    assert_eq!(env.run(&["release", "name:y", "--owner", "A"]).code, 0);
    let_go(a);
    assert_eq!(env.run(&["graph"]).stdout.trim(), "no waiters");
    assert!(env.lock_dirs().is_empty(), "{:?}", env.lock_dirs());
}

#[test]
fn simultaneous_cycle_exactly_one_gives_up() {
    let env = env("deadlock-sim");
    let a = hold(&env, "name:x", "A");
    let b = hold(&env, "name:y", "B");
    let mut a_wants_y = env.spawn(&["acquire", "name:y", "--owner", "A", "--json"]);
    let mut b_wants_x = env.spawn(&["acquire", "name:x", "--owner", "B", "--json"]);

    // One of the two gives up with 12; the other keeps waiting.
    let mut gave_up = None;
    env.wait_for(SECS5, || {
        if !a_wants_y.running() {
            gave_up = Some('A');
        } else if !b_wants_x.running() {
            gave_up = Some('B');
        }
        gave_up.is_some()
    });
    std::thread::sleep(Duration::from_millis(300));
    let (loser, winner, loser_hold, winner_hold, winner_key) = match gave_up {
        Some('A') => (a_wants_y, b_wants_x, a, b, "name:x"),
        Some('B') => (b_wants_x, a_wants_y, b, a, "name:y"),
        _ => panic!("nobody gave up"),
    };
    let mut winner = winner;
    assert!(winner.running(), "both gave up");
    let done = loser.finish(SECS5);
    assert_eq!(done.code, 12, "{}", done.stdout);
    assert_eq!(done.json()["result"], "deadlock");

    // The loser lets its key go; the winner is served.
    let_go(loser_hold);
    let done = winner.finish(SECS5);
    assert_eq!(done.code, 0, "{}", done.stdout);
    let winner_owner = if winner_key == "name:x" { "B" } else { "A" };
    assert_eq!(
        env.run(&["release", winner_key, "--owner", winner_owner])
            .code,
        0
    );
    let_go(winner_hold);
    assert!(env.lock_dirs().is_empty(), "{:?}", env.lock_dirs());
}

#[test]
fn linear_chain_of_three_is_not_a_deadlock() {
    let env = env("deadlock-chain");
    let a = hold(&env, "name:x", "A");
    let b = hold(&env, "name:y", "B");
    let c = hold(&env, "name:z", "C");
    let mut a_wants_y = env.spawn(&["acquire", "name:y", "--owner", "A"]);
    assert!(env.wait_for(SECS5, || env.tickets("name:y") == 1));
    let mut b_wants_z = env.spawn(&["acquire", "name:z", "--owner", "B"]);
    assert!(env.wait_for(SECS5, || env.tickets("name:z") == 1));
    std::thread::sleep(Duration::from_millis(400)); // several deadlock checks go by
    assert!(a_wants_y.running() && b_wants_z.running());
    let graph = env.run(&["graph", "--json"]).json();
    assert_eq!(graph["edges"].as_array().unwrap().len(), 2);
    assert!(graph["cycles"].as_array().unwrap().is_empty());

    let_go(c);
    assert_eq!(b_wants_z.wait(SECS5), Some(0));
    assert_eq!(env.run(&["release", "name:z", "--owner", "B"]).code, 0);
    let_go(b);
    assert_eq!(a_wants_y.wait(SECS5), Some(0));
    assert_eq!(env.run(&["release", "name:y", "--owner", "A"]).code, 0);
    let_go(a);
    assert!(env.lock_dirs().is_empty(), "{:?}", env.lock_dirs());
}

#[test]
fn periodic_check_makes_the_newest_ticket_give_up() {
    let env = env("deadlock-periodic");
    let a = hold(&env, "name:x", "A");
    let b = hold(&env, "name:y", "B");
    let a_wants_y = env.spawn(&["acquire", "name:y", "--owner", "A", "--json"]);
    assert!(env.wait_for(SECS5, || env.tickets("name:y") == 1));
    // A waiter B that arrived "earlier" than A's ticket appears in x's queue after A's check
    // before queueing: only the periodic check can see the cycle, and A's ticket is the newest.
    let fake = env
        .key_dir("name:x")
        .join("queue")
        .join("0000000000000001-beef.json");
    std::fs::create_dir_all(fake.parent().unwrap()).unwrap();
    std::fs::write(&fake, r#"{"key":"name:x","owner":"B"}"#).unwrap();
    let done = a_wants_y.finish(SECS5);
    assert_eq!(done.code, 12, "{}", done.stdout);
    assert_eq!(env.tickets("name:y"), 0);
    std::fs::remove_file(&fake).unwrap();
    let_go(b);
    let_go(a);
    assert!(env.lock_dirs().is_empty(), "{:?}", env.lock_dirs());
}
