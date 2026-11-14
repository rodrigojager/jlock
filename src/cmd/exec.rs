//! A foreground command with process custody and a deadline, without taking any lock. With
//! `--id` and `--custody` it also leaves the records of `custody` (`mode: "foreground"`) and runs
//! in a named job, so another process can read its state and end it with `custody stop`.
use crate::cli::{fs_error, parse_duration, unknown_option, usage, Arg, CmdResult, Parser};
use crate::custody::{self, Supervised};
use crate::i18n::txt;
use crate::process_tree::Tree;
use crate::wait;
use std::thread::sleep;
use std::time::{Duration, Instant};

pub fn run(args: Vec<String>) -> CmdResult {
    let mut parser = Parser::new(args);
    let mut timeout = None;
    let (mut id, mut dir) = (None, None);
    let mut command = Vec::new();
    while let Some(arg) = parser.next() {
        match arg {
            Arg::Flag(f) if f == "timeout" => timeout = Some(parse_duration(&parser.value(&f)?)?),
            Arg::Flag(f) if f == "id" => id = Some(parser.value(&f)?),
            Arg::Flag(f) if f == "custody" => dir = Some(parser.value(&f)?),
            Arg::Flag(f) => return Err(unknown_option("exec", &f)),
            Arg::Rest(r) => command = r,
            Arg::Pos(_) => return Err(usage(txt!("error.exec_needs_command"))),
        }
    }
    let kept = custody::foreground_opts("exec", id, dir)?;
    if command.is_empty() {
        return Err(usage(txt!("error.exec_needs_command")));
    }
    // The intent comes before any process: an id that already has one never starts a command.
    let intent = match &kept {
        Some((dir, id)) => Some(intent(dir, id, &command, timeout, "exec")?),
        None => None,
    };
    wait::install_interrupt_handler();
    let spawned = match intent {
        Some(intent) => intent.launch().map(Supervised::Kept),
        None => Tree::spawn(&command).map(Supervised::Tree),
    };
    let mut tree = spawned.map_err(|e| fs_error(txt!("error.cannot_run", cmd = command[0]), &e))?;
    let deadline = timeout.map(|d| Instant::now() + d);
    let mut reason = "exited";
    let result = loop {
        match tree.try_wait() {
            Ok(Some(code)) => break Ok(code),
            Ok(None) => {}
            Err(e) => {
                reason = "interrupted";
                break Err(fs_error(txt!("error.waiting_for", cmd = command[0]), &e));
            }
        }
        if wait::interrupted() {
            reason = "interrupted";
            break Ok(130);
        }
        if deadline.is_some_and(|d| Instant::now() >= d) {
            reason = "limit";
            break Ok(124);
        }
        sleep(Duration::from_millis(20));
    };
    // Never report completion while the command's descendants can still write or hold pipes.
    while let Err(e) = tree.stop(reason) {
        eprintln!("jlock: command cleanup pending: {e}");
        sleep(Duration::from_secs(1));
    }
    result
}

/// Records the intent of a foreground command in custody; a duplicate id is a usage error.
pub fn intent(
    dir: &std::path::Path,
    id: &str,
    command: &[String],
    limit: Option<Duration>,
    cmd: &str,
) -> Result<custody::Intent, crate::cli::Failure> {
    custody::Intent::record(dir, id, command, limit).map_err(|r| match r {
        custody::Refusal::Duplicate => usage(txt!("error.custody_duplicate", cmd = cmd, id = id)),
        custody::Refusal::Records(e) => fs_error(txt!("error.custody_records"), &e),
    })
}
