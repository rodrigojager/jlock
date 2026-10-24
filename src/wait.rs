//! Waiting: poll interval with jitter (`JLOCK_POLL_MS`), randomness without extra crates, and the
//! CTRL+C / SIGINT / SIGTERM flag that lets a waiter remove its ticket before leaving.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::sleep;
use std::time::{Duration, Instant};

static INTERRUPTED: AtomicBool = AtomicBool::new(false);
static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Reads a millisecond setting from the environment; missing or invalid falls back to `default`.
pub fn env_ms(name: &str, default: u64) -> u64 {
    match std::env::var(name) {
        Ok(v) => match v.trim().parse::<u64>() {
            Ok(n) if n > 0 => n,
            _ => {
                let value = format!("{v:?}");
                eprintln!(
                    "jlock: {}",
                    crate::i18n::t!("warn.bad_ms", name = name, value = value)
                );
                default
            }
        },
        Err(_) => default,
    }
}

pub fn poll_ms() -> u64 {
    env_ms("JLOCK_POLL_MS", 250)
}

pub fn debug(msg: impl std::fmt::Display) {
    if std::env::var_os("JLOCK_DEBUG").is_some() {
        eprintln!("jlock[{}]: {msg}", std::process::id());
    }
}

pub fn random_u64() -> u64 {
    let mut h = RandomState::new().build_hasher();
    h.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
    h.write_u128(crate::holder::now_micros() as u128);
    h.write_u32(std::process::id());
    h.finish()
}

/// The poll interval with ±40% jitter.
pub fn jittered(base_ms: u64) -> Duration {
    let spread = base_ms * 4 / 10;
    let offset = if spread == 0 {
        0
    } else {
        random_u64() % (2 * spread + 1)
    };
    Duration::from_millis(base_ms - spread + offset)
}

/// Sleeps one poll interval in short slices; stops early on interruption or at `deadline`.
pub fn pause(base_ms: u64, deadline: Option<Instant>) {
    let until = Instant::now() + jittered(base_ms);
    let until = match deadline {
        Some(d) if d < until => d,
        _ => until,
    };
    while !interrupted() {
        let now = Instant::now();
        if now >= until {
            break;
        }
        sleep((until - now).min(Duration::from_millis(20)));
    }
}

pub fn interrupted() -> bool {
    INTERRUPTED.load(Ordering::SeqCst)
}

pub fn interrupt() {
    INTERRUPTED.store(true, Ordering::SeqCst);
}

#[cfg(windows)]
unsafe extern "system" fn on_ctrl(kind: u32) -> windows_sys::Win32::Foundation::BOOL {
    use windows_sys::Win32::System::Console::{CTRL_BREAK_EVENT, CTRL_C_EVENT};
    interrupt();
    if kind != CTRL_C_EVENT && kind != CTRL_BREAK_EVENT {
        // Close, logoff, shutdown: the process ends when this returns, so give the main thread
        // a moment to clean up.
        sleep(Duration::from_millis(1500));
    }
    1
}

#[cfg(unix)]
extern "C" fn on_signal(_: libc::c_int) {
    interrupt();
}

/// Turns CTRL+C, CTRL+BREAK, console close, SIGINT, SIGTERM and SIGHUP into [`interrupted`].
pub fn install_interrupt_handler() {
    #[cfg(windows)]
    // SAFETY: registers a handler with the signature the console API expects.
    unsafe {
        windows_sys::Win32::System::Console::SetConsoleCtrlHandler(Some(on_ctrl), 1);
    }
    #[cfg(unix)]
    // SAFETY: the handler only stores into an atomic, which is async-signal-safe.
    unsafe {
        let handler = on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
        libc::signal(libc::SIGINT, handler);
        libc::signal(libc::SIGTERM, handler);
        libc::signal(libc::SIGHUP, handler);
    }
}
