//! Heartbeat (plan decision 6): the mtime of a hold file or of a ticket, renewed every
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
                "jlock: {}",
                crate::i18n::t!(
                    "warn.stale_vs_heartbeat",
                    stale = stale,
                    heartbeat = heartbeat
                )
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
    /// Ours, but past its deadline (`since + ttl`, or the one `renew` set).
    TtlExpired,
}

/// Renews every hold of `owner` on a key. No hold of the owner is `Lost` (with the owner of the
/// oldest other hold, if any).
pub fn beat(kd: &KeyDir, owner: &str) -> io::Result<Beat> {
    let holds = kd.read_holds()?;
    let mine: Vec<_> = holds.iter().filter(|h| h.body.owner == owner).collect();
    if mine.is_empty() {
        return Ok(Beat::Lost(holds.first().map(|h| h.body.owner.clone())));
    }
    let now = now_micros();
    if mine.iter().any(|h| crate::lease::expired(kd, h, now)) {
        return Ok(Beat::TtlExpired);
    }
    let mut renewed = false;
    for h in mine {
        match fsx::touch(&h.path) {
            Ok(()) => renewed = true,
            Err(e) if fsx::is_not_found(&e) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(if renewed {
        Beat::Renewed
    } else {
        Beat::Lost(None)
    })
}
