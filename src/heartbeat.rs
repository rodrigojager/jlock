//! Heartbeat (plan decision 6): the mtime of `holder.json` or of a ticket, renewed every
//! `JLOCK_HEARTBEAT_MS`; without renewal for `JLOCK_STALE_MS` the owner is taken for dead.

use crate::fsx;
use crate::holder::now_micros;
use crate::store::KeyDir;
use crate::wait::env_ms;
use std::io;
use std::sync::OnceLock;
use std::time::Duration;

struct Times {
    heartbeat: u64,
    stale: u64,
}

fn times() -> &'static Times {
    static TIMES: OnceLock<Times> = OnceLock::new();
    TIMES.get_or_init(|| {
        let heartbeat = env_ms("JLOCK_HEARTBEAT_MS", 5000);
        let stale = env_ms("JLOCK_STALE_MS", 30000);
        if stale <= heartbeat {
            eprintln!(
                "jlock: JLOCK_STALE_MS ({stale}) should be well above JLOCK_HEARTBEAT_MS ({heartbeat})"
            );
        }
        Times { heartbeat, stale }
    })
}

pub fn heartbeat_interval() -> Duration {
    Duration::from_millis(times().heartbeat)
}

pub fn stale_after() -> Duration {
    Duration::from_millis(times().stale)
}

pub enum Beat {
    Renewed,
    /// The lock is not ours any more; the current owner if there is one.
    Lost(Option<String>),
    /// Ours, but past `since + ttl`.
    TtlExpired,
}

/// Renews `owner`'s lock on a key: reads the holder first; a different owner or no holder is `Lost`.
pub fn beat(kd: &KeyDir, owner: &str) -> io::Result<Beat> {
    match kd.read_holder()? {
        None => Ok(Beat::Lost(None)),
        Some(h) if h.owner != owner => Ok(Beat::Lost(Some(h.owner))),
        Some(h) if h.ttl_expired(now_micros()) => Ok(Beat::TtlExpired),
        Some(_) => match fsx::touch(&kd.holder_path()) {
            Ok(()) => Ok(Beat::Renewed),
            Err(e) if fsx::is_not_found(&e) => Ok(Beat::Lost(None)),
            Err(e) => Err(e),
        },
    }
}
