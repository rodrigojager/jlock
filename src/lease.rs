//! A renewed deadline (`jlock renew`, plan 7.4): `lease.json` next to `holder.json`, written by
//! `.tmp` + rename. `holder.json` itself is never rewritten, so the OS lock a `hold`/`run` keeps on
//! it and its heartbeat stay as they are, and the key is never free while it is renewed. A lease
//! counts only for the grant it names (`owner` and `since`); one left from an earlier grant is
//! ignored (and overwritten by the next renew), and removed with the key folder.

use crate::fsx;
use crate::holder::{parse_iso, Holder};
use crate::store::KeyDir;
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
pub fn read(kd: &KeyDir) -> Option<Lease> {
    let text = fsx::retry(&kd.lease_path(), || fs::read_to_string(kd.lease_path())).ok()?;
    serde_json::from_str(&text).ok()
}

/// Replaces the lease in one rename: a reader sees the old one or the new one, never half.
pub fn write(kd: &KeyDir, lease: &Lease) -> io::Result<()> {
    let tmp = kd.path.join(format!(
        "lease.{}-{:04x}.tmp",
        std::process::id(),
        random_u64() & 0xffff
    ));
    let bytes = serde_json::to_vec(lease).expect("lease serializes");
    fsx::create_exclusive(&tmp, &bytes)?;
    let result = fsx::rename_with_retry(&tmp, &kd.lease_path());
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// The moment (micros) the lock of `h` expires: the lease's `until` when the lease is for this
/// very grant, else `since + ttl`; `None` when it never does.
pub fn deadline(kd: &KeyDir, h: &Holder) -> Option<u64> {
    if kd.lease_path().exists() {
        if let Some(until) = read(kd)
            .filter(|l| l.is_for(h))
            .and_then(|l| parse_iso(&l.until))
        {
            return Some(until);
        }
    }
    h.ttl_deadline()
}

/// The deadline has passed.
pub fn expired(kd: &KeyDir, h: &Holder, now: u64) -> bool {
    deadline(kd, h).is_some_and(|d| now > d)
}
