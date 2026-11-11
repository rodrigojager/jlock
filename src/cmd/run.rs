//! `jlock run <keys> -- <command>`: like `flock(1)`. Takes the keys, runs the command as a child
//! while renewing the heartbeat, releases at the end and passes the command's exit code through.

use crate::cli::{fs_error, usage, CmdResult, Failure, EXIT_LOST, EXIT_USAGE};
use crate::cmd::acquire::{acquire_all, parse, report, wants, SetOutcome};
use crate::cmd::hold::Keeper;
use crate::i18n::txt;
use crate::process_tree::Tree;
use crate::store::Store;
use crate::wait;
use std::thread::sleep;
use std::time::{Duration, Instant};

pub fn run(args: Vec<String>) -> CmdResult {
    let (opts, command) = parse(args, "run")?;
    if command.is_empty() {
        return Err(usage(txt!("error.run_needs_command")));
    }
    let started = Instant::now();
    let total_deadline = opts.total_timeout.map(|t| started + t);
    // CTRL+C reaches the command too; jlock keeps the lock until the command is gone.
    wait::install_interrupt_handler();
    let wants = wants(&opts)?;
    let store = Store::open()?;
    let deadline = [opts.timeout.map(|t| started + t), total_deadline]
        .into_iter()
        .flatten()
        .min();
    let outcome = acquire_all(&store, &wants, &opts.ident, opts.wait, deadline)
        .map_err(|e| fs_error(txt!("error.cannot_lock"), &e))?;
    if !matches!(outcome, SetOutcome::Granted { .. }) {
        let code = report(&opts, &wants, &outcome);
        return Ok(
            if code == crate::cli::EXIT_TIMEOUT
                && total_deadline.is_some_and(|d| Instant::now() >= d)
            {
                124
            } else {
                code
            },
        );
    }
    let _ = crate::index::rewrite(&store);
    let mut keeper = Keeper::new(&store, outcome.keys(), &opts.ident.owner, opts.json);
    // A grant can race with the deadline: release it without ever starting the payload.
    if total_deadline.is_some_and(|d| Instant::now() >= d) {
        keeper.release_all();
        return Ok(124);
    }
    let mut child = match Tree::spawn(&command) {
        Ok(child) => child,
        Err(e) => {
            keeper.release_all();
            return Err(Failure {
                code: EXIT_USAGE,
                msg: txt!("error.cannot_run", cmd = command[0]).with(e),
            });
        }
    };
    let command_deadline = [
        opts.command_timeout.map(|t| Instant::now() + t),
        total_deadline,
    ]
    .into_iter()
    .flatten()
    .min();
    let mut lost = None;
    let result = loop {
        match child.try_wait() {
            Ok(Some(code)) => break Ok(code),
            Ok(None) => {}
            Err(e) => {
                break Err(fs_error(txt!("error.waiting_for", cmd = command[0]), &e));
            }
        }
        if let Some(trouble) = keeper.tick() {
            // The lock is gone: the command must not go on unprotected.
            lost = Some(trouble);
            break Ok(EXIT_LOST);
        }
        if wait::interrupted() {
            break Ok(130);
        }
        if command_deadline.is_some_and(|d| Instant::now() >= d) {
            break Ok(124);
        }
        sleep(Duration::from_millis(20));
    };
    while let Err(e) = child.stop() {
        eprintln!("jlock: command cleanup pending; retaining locks: {e}");
        if lost.is_none() {
            lost = keeper.tick();
        }
        sleep(Duration::from_secs(1));
    }
    if let Some(trouble) = lost {
        keeper.lose(&trouble, true);
        return Ok(EXIT_LOST);
    }
    keeper.release_all();
    result
}
