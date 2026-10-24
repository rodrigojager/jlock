//! `jlock hold --detach`: one call that waits its turn, answers like `hold` (granted, busy, timeout,
//! deadlock, with the same exit codes) and returns, while a second `jlock hold`, started detached
//! from the terminal, keeps the lock alive until `jlock release`, its `--ttl`, or `unlock --force`.
//! The second process tells the first that the lock is held with a private line on its standard
//! output, then points its standard output and error at the null device so the first one sees the
//! end of its output and can leave.

use crate::cli::{CmdResult, EXIT_FS, EXIT_OK};
use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::thread::sleep;
use std::time::Duration;

/// Set in the environment of the second process.
pub const CHILD: &str = "JLOCK_DETACHED_CHILD";
/// What the second process prints once the keys are held (never shown to the user).
const READY: &str = "\u{1e}jlock-ready";
const EXIT_INTERRUPTED: i32 = 130;

pub fn is_child() -> bool {
    std::env::var_os(CHILD).is_some()
}

#[cfg(windows)]
fn spawn(cmd: &mut Command) -> std::io::Result<Child> {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
    use windows_sys::Win32::Foundation::{SetHandleInformation, HANDLE_FLAG_INHERIT};
    use windows_sys::Win32::System::Console::{
        GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
    };
    // Windows hands every inheritable handle to a new process. Our own standard handles must not
    // go to the keeper: whoever reads our output (a terminal pipe, an agent's tool) would wait for
    // its end until the keeper exits. This process ends right after, so nothing else needs them.
    for id in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
        // SAFETY: changes a flag of our own standard handle; an invalid one is simply refused.
        unsafe {
            SetHandleInformation(GetStdHandle(id), HANDLE_FLAG_INHERIT, 0);
        }
    }
    // Out of the caller's job when the job allows it, so that a tool which closes its job when the
    // command returns does not take the lock keeper with it.
    cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_BREAKAWAY_FROM_JOB);
    match cmd.spawn() {
        Ok(child) => Ok(child),
        Err(_) => {
            cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
            cmd.spawn()
        }
    }
}

#[cfg(unix)]
fn spawn(cmd: &mut Command) -> std::io::Result<Child> {
    use std::os::unix::process::CommandExt;
    // SAFETY: setsid is async-signal-safe; it only gives the child its own session, so that the
    // terminal's CTRL+C and hang-up do not reach it.
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    cmd.spawn()
}

/// In the first process: starts the second one with the same arguments, passes its answer along
/// and returns its exit code, or 0 once it holds the keys.
pub fn start(args: Vec<String>) -> CmdResult {
    let exe = std::env::current_exe()
        .map_err(|e| crate::cli::fs_error(crate::i18n::txt!("update.no_exe"), &e))?;
    let args: Vec<String> = args.into_iter().filter(|a| a != "--detach").collect();
    let mut cmd = Command::new(exe);
    cmd.arg("hold")
        .args(&args)
        .env(CHILD, "1")
        .env("JLOCK_LANG", crate::i18n::lang())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = spawn(&mut cmd)
        .map_err(|e| crate::cli::fs_error(crate::i18n::txt!("error.cannot_detach"), &e))?;
    let read = |pipe: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut text = String::new();
            if let Some(mut p) = pipe {
                let _ = p.read_to_string(&mut text);
            }
            text
        })
    };
    let out = read(
        child
            .stdout
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let err = read(
        child
            .stderr
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    crate::wait::install_interrupt_handler();
    while !out.is_finished() {
        if crate::wait::interrupted() {
            // Our own child, still waiting its turn: it goes, and its ticket expires.
            let _ = child.kill();
            let _ = child.wait();
            return Ok(EXIT_INTERRUPTED);
        }
        sleep(Duration::from_millis(20));
    }
    let out = out.join().unwrap_or_default();
    let held = out.lines().any(|l| l == READY);
    let code = if held {
        EXIT_OK
    } else {
        child.wait().ok().and_then(|s| s.code()).unwrap_or(EXIT_FS)
    };
    let err = err.join().unwrap_or_default();
    for line in out.lines().filter(|l| *l != READY) {
        println!("{line}");
    }
    eprint!("{err}");
    Ok(code)
}

/// In the second process, once the keys are held: tells the first one, then lets go of the pipes
/// it was given (the first process ends; nothing would read them).
pub fn ready() {
    println!("{READY}");
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
    to_null();
}

#[cfg(windows)]
fn to_null() {
    use std::os::windows::io::IntoRawHandle;
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Console::{
        GetStdHandle, SetStdHandle, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE,
    };
    let Ok(null) = std::fs::OpenOptions::new().write(true).open("NUL") else {
        return;
    };
    let null = null.into_raw_handle();
    // SAFETY: swaps the process's standard handles for one we own, then closes the pipe ends we
    // were started with; the standard library looks the handles up on every write.
    unsafe {
        let out = GetStdHandle(STD_OUTPUT_HANDLE);
        let err = GetStdHandle(STD_ERROR_HANDLE);
        SetStdHandle(STD_OUTPUT_HANDLE, null as _);
        SetStdHandle(STD_ERROR_HANDLE, null as _);
        CloseHandle(out);
        if err != out {
            CloseHandle(err);
        }
    }
}

#[cfg(unix)]
fn to_null() {
    // SAFETY: plain descriptor calls; /dev/null replaces descriptors 1 and 2, which closes the pipes.
    unsafe {
        let fd = libc::open(c"/dev/null".as_ptr(), libc::O_RDWR);
        if fd >= 0 {
            libc::dup2(fd, 1);
            libc::dup2(fd, 2);
            if fd > 2 {
                libc::close(fd);
            }
        }
    }
}
