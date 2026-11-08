//! A renewed deadline (`jlock renew`, plan 7.4): `holds/<grant>.lease` next to the hold it extends,
//! written by `.tmp` + rename. The hold file itself is never rewritten, so the OS lock a `hold`/`run`
//! keeps on it and its heartbeat stay as they are, and the key is never free while it is renewed. A
//! lease counts only for the grant it names (`owner` and `since`); one left from an earlier grant is
//! ignored, and removed with the hold or the key folder.

use crate::fsx;
use crate::holder::{parse_iso, Holder};
use crate::store::{Hold, KeyDir};
use crate::wait::random_u64;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Lease {
    pub owner: String,
    /// The `since` of the grant it extends.
    pub since: String,
    pub until: String,
}

impl Lease {
    fn is_for(&self, h: &Holder) -> bool {
        self.owner == h.owner && self.since == h.since
    }
}

/// The lease on disk, `None` if missing or unreadable (a torn file never wins over the ttl).
pub fn read(kd: &KeyDir, grant: &str) -> Option<Lease> {
    let path = kd.lease_path(grant);
    let text = fsx::retry(&path, || fs::read_to_string(&path)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Replaces the lease in one rename: a reader sees the old one or the new one, never half.
pub fn write(kd: &KeyDir, grant: &str, lease: &Lease) -> io::Result<()> {
    let tmp = kd.holds_dir().join(format!(
        "{grant}.lease.{}-{:04x}.tmp",
        std::process::id(),
        random_u64() & 0xffff
    ));
    let bytes = serde_json::to_vec(lease).expect("lease serializes");
    fsx::create_exclusive(&tmp, &bytes)?;
    let result = fsx::rename_with_retry(&tmp, &kd.lease_path(grant));
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// The moment (micros) a hold expires: the lease's `until` when the lease is for this very grant,
/// else `since + ttl`; `None` when it never does.
pub fn deadline_of(kd: &KeyDir, grant: &str, h: &Holder) -> Option<u64> {
    if kd.lease_path(grant).exists() {
        if let Some(until) = read(kd, grant)
            .filter(|l| l.is_for(h))
            .and_then(|l| parse_iso(&l.until))
        {
            return Some(until);
        }
    }
    h.ttl_deadline()
}

pub fn deadline(kd: &KeyDir, hold: &Hold) -> Option<u64> {
    deadline_of(kd, &hold.grant, &hold.body)
}

/// The deadline has passed.
pub fn expired(kd: &KeyDir, hold: &Hold, now: u64) -> bool {
    deadline(kd, hold).is_some_and(|d| now > d)
}
