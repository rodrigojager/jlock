//! A foreground command with process custody and a deadline, without taking any lock.
use crate::cli::{fs_error, parse_duration, unknown_option, usage, Arg, CmdResult, Parser};
use crate::i18n::txt;
use crate::process_tree::Tree;
use crate::wait;
use std::thread::sleep;
use std::time::{Duration, Instant};

pub fn run(args: Vec<String>) -> CmdResult {
    let mut parser = Parser::new(args);
    let mut timeout = None;
    let mut command = Vec::new();
    while let Some(arg) = parser.next() {
        match arg {
            Arg::Flag(f) if f == "timeout" => timeout = Some(parse_duration(&parser.value(&f)?)?),
            Arg::Flag(f) => return Err(unknown_option("exec", &f)),
            Arg::Rest(r) => command = r,
            Arg::Pos(_) => return Err(usage(txt!("error.exec_needs_command"))),
        }
    }
    if command.is_empty() {
        return Err(usage(txt!("error.exec_needs_command")));
    }
    wait::install_interrupt_handler();
    let mut tree = Tree::spawn(&command)
        .map_err(|e| fs_error(txt!("error.cannot_run", cmd = command[0]), &e))?;
    let deadline = timeout.map(|d| Instant::now() + d);
    let result = loop {
        match tree.try_wait() {
            Ok(Some(code)) => break Ok(code),
            Ok(None) => {}
            Err(e) => break Err(fs_error(txt!("error.waiting_for", cmd = command[0]), &e)),
        }
        if wait::interrupted() {
            break Ok(130);
        }
        if deadline.is_some_and(|d| Instant::now() >= d) {
            break Ok(124);
        }
        sleep(Duration::from_millis(20));
    };
    // Never report completion while the command's descendants can still write or hold pipes.
    while let Err(e) = tree.stop() {
        eprintln!("jlock: command cleanup pending: {e}");
        sleep(Duration::from_secs(1));
    }
    result
}
