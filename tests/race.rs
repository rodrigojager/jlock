mod common;

use common::Env;
use std::sync::{Arc, Barrier};
use std::thread;

const RACERS: usize = 12;
const ROUNDS: usize = 30;

#[test]
fn twelve_racers_exactly_one_winner_per_round() {
    let env = Env::new("race");
    let key = "name:race";
    for round in 0..ROUNDS {
        // Common start: every racer is launched at the same moment, behind a barrier.
        let barrier = Arc::new(Barrier::new(RACERS));
        let racers: Vec<_> = (0..RACERS)
            .map(|i| {
                let owner = format!("r{round}-{i}");
                let mut cmd = env.cmd(&["acquire", key, "--no-wait", "--json", "--owner", &owner]);
                let barrier = barrier.clone();
                thread::spawn(move || {
                    barrier.wait();
                    let out = cmd.output().expect("run racer");
                    (owner, out)
                })
            })
            .collect();
        let results: Vec<_> = racers.into_iter().map(|t| t.join().unwrap()).collect();

        let winners: Vec<&String> = results
            .iter()
            .filter(|(_, out)| out.status.code() == Some(0))
            .map(|(owner, _)| owner)
            .collect();
        let losers = results
            .iter()
            .filter(|(_, out)| out.status.code() == Some(10))
            .count();
        assert_eq!(winners.len(), 1, "round {round}: winners {winners:?}");
        assert_eq!(losers, RACERS - 1, "round {round}");
        let winner = winners[0].clone();

        let dir = env.key_dir(key);
        let holder: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("holder.json")).unwrap())
                .unwrap();
        assert_eq!(holder["owner"], winner.as_str());
        assert_eq!(holder["key"], key);
        for (owner, out) in &results {
            if *owner != winner {
                let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
                assert_eq!(v["result"], "queued");
                assert_eq!(v["holder"]["owner"], winner.as_str());
            }
        }

        let done = env.run(&["release", key, "--owner", &winner]);
        assert_eq!(done.code, 0, "{}", done.stderr);
    }
    assert!(env.lock_dirs().is_empty(), "{:?}", env.lock_dirs());
}

#[test]
fn release_by_someone_else_is_refused() {
    let env = Env::new("race-not-owner");
    assert_eq!(
        env.run(&["acquire", "name:x", "--no-wait", "--owner", "a"])
            .code,
        0
    );
    let done = env.run(&["release", "name:x", "--owner", "b", "--json"]);
    assert_eq!(done.code, 13);
    assert_eq!(done.json()["result"], "not_owner");
    assert_eq!(env.run(&["release", "name:x", "--owner", "a"]).code, 0);
    assert!(env.lock_dirs().is_empty());
}
