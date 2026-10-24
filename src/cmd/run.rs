//! `jlock run <keys> -- <command>`: like `flock(1)`. Takes the keys, runs the command as a child
//! while renewing the heartbeat, releases at the end and passes the command's exit code through.

use crate::cli::{fs_error, usage, CmdResult, Failure, EXIT_LOST, EXIT_USAGE};
use crate::cmd::acquire::{acquire_all, normalize_keys, parse, report, SetOutcome};
use crate::cmd::hold::Keeper;
use crate::i18n::txt;
use crate::store::Store;
use crate::wait;
use std::process::{Command, ExitStatus};
use std::thread::sleep;
use std::time::{Duration, Instant};

fn exit_code(status: ExitStatus) -> i32 {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(sig) = status.signal() {
            return 128 + sig;
        }
    }
    status.code().unwrap_or(1)
}

pub fn run(args: Vec<String>) -> CmdResult {
    let (opts, command) = parse(args, "run")?;
    if command.is_empty() {
        return Err(usage(txt!("error.run_needs_command")));
    }
    // CTRL+C reaches the command too; jlock keeps the lock until the command is gone.
    wait::install_interrupt_handler();
    let keys = normalize_keys(&opts.keys)?;
    let store = Store::open()?;
    let deadline = opts.timeout.map(|t| Instant::now() + t);
    let outcome = acquire_all(&store, &keys, &opts.ident, opts.wait, deadline)
        .map_err(|e| fs_error(txt!("error.cannot_lock"), &e))?;
    let SetOutcome::Granted { keys, .. } = outcome else {
        return Ok(report(&opts, &outcome));
    };
    let _ = crate::index::rewrite(&store);
    let mut keeper = Keeper::new(&store, keys, &opts.ident.owner, opts.json);
    let mut child = match Command::new(&command[0]).args(&command[1..]).spawn() {
        Ok(child) => child,
        Err(e) => {
            keeper.release_all();
            return Err(Failure {
                code: EXIT_USAGE,
                msg: txt!("error.cannot_run", cmd = command[0]).with(e),
            });
        }
    };
    let code = loop {
        match child.try_wait() {
            Ok(Some(status)) => break exit_code(status),
            Ok(None) => {}
            Err(e) => {
                keeper.release_all();
                return Err(fs_error(txt!("error.waiting_for", cmd = command[0]), &e));
            }
        }
        if let Some(trouble) = keeper.tick() {
            // The lock is gone: the command must not go on unprotected.
            let _ = child.kill();
            let _ = child.wait();
            keeper.lose(&trouble, true);
            return Ok(EXIT_LOST);
        }
        sleep(Duration::from_millis(20));
    };
    keeper.release_all();
    Ok(code)
}
