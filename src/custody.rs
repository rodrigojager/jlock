//! Custody of a detached command: its durable records in `<custody>/<id>/`, the named job that
//! holds its process tree, and the state `custody status` reads from both.
//!
//! Records (see `jlock help custody`): `intent.json` (before any process, created exclusively),
//! `receipt.json` (with the command suspended and already in its job, before it runs),
//! `beat.json` (the keeper's sign of life, replaced every `JLOCK_CUSTODY_BEAT_MS`),
//! `final.json` (once, by whoever confirmed the end), `output.log` (the command's output and
//! errors). Two private files help tell `launching` from `launch-failed`: `launch.lock`, which the
//! first process holds an OS lock on from before the intent until it answers, and
//! `custodian.json`, the pid and identity of the keeper before its receipt exists. A third,
//! `stop.json`, is written by `custody stop` before it ends the job.
//!
//! Windows only: elsewhere every custody command refuses with "unsupported on this platform".
#![cfg_attr(not(windows), allow(dead_code))]

use serde_json::{json, Value};
use std::io;
use std::path::{Path, PathBuf};

pub const INTENT: &str = "intent.json";
pub const RECEIPT: &str = "receipt.json";
pub const BEAT: &str = "beat.json";
pub const FINAL: &str = "final.json";
pub const OUTPUT: &str = "output.log";
pub const CUSTODIAN: &str = "custodian.json";
pub const LAUNCH_LOCK: &str = "launch.lock";
/// Private: written by `custody stop` before it ends the job, so the keeper, seeing its command
/// end, leaves `final.json` to the stop.
pub const STOP_REQUEST: &str = "stop.json";

/// Set in the environment of the keeper (the detached `jlock` that holds the job); never passed on
/// to the command.
pub const KEEPER_ENV: &str = "JLOCK_CUSTODY_KEEPER";

/// `^[A-Za-z0-9._-]{1,64}$`, without `.` and `..` (they would name the custody folder itself or
/// its parent).
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id != "."
        && id != ".."
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// The absolute form of `dir`, without trailing separators: what the job name is made from.
pub fn absolute(dir: &Path) -> io::Result<PathBuf> {
    let abs = std::path::absolute(dir)?;
    let text = abs.to_string_lossy();
    let trimmed = text.trim_end_matches(['\\', '/']);
    // Keep the separator of a root (`C:\`, `/`).
    if trimmed.is_empty() || trimmed.ends_with(':') {
        return Ok(abs);
    }
    Ok(PathBuf::from(trimmed))
}

/// `Local\jlock-custody-<12 hex of the sha1 of the absolute custody folder>-<id>`.
pub fn job_name(dir: &Path, id: &str) -> String {
    use sha1::{Digest, Sha1};
    let hex: String = Sha1::digest(dir.to_string_lossy().as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("Local\\jlock-custody-{}-{id}", &hex[..12])
}

pub fn now() -> String {
    crate::holder::iso(crate::holder::now_micros())
}

/// A record, if it exists and is whole JSON (records are published whole, but a reader never
/// trusts that).
pub fn read_json(path: &Path) -> Option<Value> {
    let text = crate::fsx::retry(path, || std::fs::read_to_string(path)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Publishes a record whole and only once: written to a private temporary file, then linked under
/// its name, which fails if the name exists (`AlreadyExists`). A file system without hard links
/// falls back to an exclusive create.
pub fn publish_once(path: &Path, value: &Value) -> io::Result<()> {
    let bytes = format!("{value}\n");
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    crate::fsx::retry(&tmp, || std::fs::write(&tmp, bytes.as_bytes()))?;
    let linked = crate::fsx::retry(path, || std::fs::hard_link(&tmp, path));
    let result = match linked {
        Ok(()) => Ok(()),
        Err(e) if crate::fsx::is_exists(&e) => Err(e),
        Err(_) => crate::fsx::create_exclusive(path, bytes.as_bytes()),
    };
    let _ = crate::fsx::remove_with_retry(&tmp);
    result
}

/// A duration in milliseconds from the environment, or `default_ms` when it is unset or not a
/// whole number.
pub fn setting_ms(name: &str, default_ms: u64) -> std::time::Duration {
    let ms = std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default_ms);
    std::time::Duration::from_millis(ms)
}

/// How often the keeper writes `beat.json` (`JLOCK_CUSTODY_BEAT_MS`, 10 s).
pub fn beat_every() -> std::time::Duration {
    setting_ms("JLOCK_CUSTODY_BEAT_MS", 10_000)
}

/// How long the keeper goes on without a beat written before it ends its own job
/// (`JLOCK_CUSTODY_SELF_FENCE_MS`, 30 s).
pub fn self_fence_after() -> std::time::Duration {
    setting_ms("JLOCK_CUSTODY_SELF_FENCE_MS", 30_000)
}

/// How long ending a job waits for it to be empty (`JLOCK_CUSTODY_STOP_MS`, 15 s).
pub fn stop_wait() -> std::time::Duration {
    setting_ms("JLOCK_CUSTODY_STOP_MS", 15_000)
}

/// Replaces `beat.json` in `rec`: written whole to `beat.json.tmp`, then renamed over the old one.
pub fn write_beat(rec: &Path, beat: &Value) -> io::Result<()> {
    let path = rec.join(BEAT);
    let tmp = rec.join(format!("{BEAT}.tmp"));
    std::fs::write(&tmp, format!("{beat}\n").as_bytes())?;
    std::fs::rename(&tmp, &path)
}

/// Whether the keeper named by a record (`custodianPid` + `custodianIdentity`, or `pid` +
/// `identity` in `custodian.json`) is proven gone. Unknown is never taken as gone.
fn keeper_gone(pid: &Value, identity: &Value) -> bool {
    let (Some(pid), Some(identity)) = (pid.as_u64(), identity.as_str()) else {
        return false;
    };
    matches!(
        crate::process_identity::inspect(pid as u32, Some(identity)),
        crate::process_identity::Inspection::Absent { .. }
    )
}

/// `{ "open": bool, "activeProcesses"?: n }`, opening the job by its name and closing it at once.
pub fn job_info(name: &str) -> Value {
    match Job::open_query(name) {
        Ok(Some(job)) => match job.active() {
            Ok(n) => json!({ "open": true, "activeProcesses": n }),
            Err(_) => json!({ "open": true }),
        },
        _ => json!({ "open": false }),
    }
}

/// The answer of `custody status`: `{ state, intent?, receipt?, beat?, final?, job }`. Read only.
pub fn status(dir: &Path, id: &str) -> Value {
    let rec = dir.join(id);
    let intent = read_json(&rec.join(INTENT));
    let receipt = read_json(&rec.join(RECEIPT));
    let beat = read_json(&rec.join(BEAT));
    let fin = read_json(&rec.join(FINAL));
    let name = receipt
        .as_ref()
        .and_then(|r| r["job"].as_str().map(str::to_string))
        .unwrap_or_else(|| job_name(dir, id));
    let state = if !rec.join(INTENT).exists() {
        "absent"
    } else if fin.is_some() {
        "ended"
    } else if let Some(r) = &receipt {
        if keeper_gone(&r["custodianPid"], &r["custodianIdentity"]) {
            "lost"
        } else {
            "running"
        }
    } else if crate::oslock::is_held(&rec.join(LAUNCH_LOCK)) {
        // The first process is still between the intent and its answer.
        "launching"
    } else {
        match read_json(&rec.join(CUSTODIAN)) {
            Some(c) if !keeper_gone(&c["pid"], &c["identity"]) => "launching",
            _ => "launch-failed",
        }
    };
    let mut out = json!({ "state": state });
    for (field, value) in [
        ("intent", intent),
        ("receipt", receipt),
        ("beat", beat),
        ("final", fin),
    ] {
        if let Some(v) = value {
            out[field] = v;
        }
    }
    out["job"] = job_info(&name);
    out
}

/// What `custody stop` found: the answer to print and whether the end is confirmed.
pub struct Stopped {
    pub confirmed: bool,
    pub answer: Value,
}

/// `custody stop`: ends the command's job by its name and confirms that nothing is left, by one of
/// two proofs only: (1) the job opened by name, ended, and zero processes active within `timeout`
/// (with the root of the receipt not alive outside it); (2) no job by that name, and the root's
/// identity checked as absent through an open handle. Nothing is ever ended by pid, and a job that
/// is gone is never created again. A confirmed end is recorded once in `final.json` (`by: "stop"`);
/// an existing confirmed `final.json` is answered as is, without touching anything.
///
/// `skip_kill` leaves the job alone (the test-only fault that makes the wait run out).
pub fn stop(
    dir: &Path,
    id: &str,
    detail: &str,
    timeout: std::time::Duration,
    skip_kill: bool,
) -> io::Result<Stopped> {
    use crate::process_identity::{inspect, Inspection};

    let rec = dir.join(id);
    let fin_path = rec.join(FINAL);
    let recorded = read_json(&fin_path);
    if let Some(f) = &recorded {
        if f["cleanupConfirmed"] == true {
            return Ok(Stopped {
                confirmed: true,
                answer: f.clone(),
            });
        }
    }
    let not_confirmed = |why: String, active: Option<u32>| {
        let mut v = json!({
            "v": 1,
            "id": id,
            "cleanupConfirmed": false,
            "scope": "job",
            "error": why,
        });
        if let Some(n) = active {
            v["activeProcesses"] = json!(n);
        }
        if let Some(f) = &recorded {
            v["final"] = f.clone();
        }
        Stopped {
            confirmed: false,
            answer: v,
        }
    };
    if !rec.join(INTENT).exists() {
        return Ok(not_confirmed(
            "no intent.json: nothing was started with this id".into(),
            None,
        ));
    }
    let Some(receipt) = read_json(&rec.join(RECEIPT)) else {
        let why = match status(dir, id)["state"].as_str() {
            Some("launching") => "the launch is still in progress (no receipt yet)",
            _ => "no receipt.json: there is no command to confirm",
        };
        return Ok(not_confirmed(why.into(), None));
    };
    let (Some(pid), Some(identity)) = (receipt["pid"].as_u64(), receipt["identity"].as_str())
    else {
        return Ok(not_confirmed(
            "receipt.json has no pid and identity".into(),
            None,
        ));
    };
    let name = receipt["job"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| job_name(dir, id));
    let marker = rec.join(STOP_REQUEST);
    // Decided before the job is touched: once ended, a keeper that leaves on its own looks lost.
    let reason = if !marker.exists()
        && keeper_gone(&receipt["custodianPid"], &receipt["custodianIdentity"])
    {
        "custodian-lost"
    } else {
        "stopped"
    };
    // Tells the keeper that this end is ours to record, before the command can end.
    let _ = publish_once(
        &marker,
        &json!({ "v": 1, "id": id, "detail": detail, "at": now() }),
    );
    let failed = |why: String, active: Option<u32>| {
        // A stop that did not confirm leaves the end to the keeper again.
        let _ = crate::fsx::remove_with_retry(&marker);
        Ok(not_confirmed(why, active))
    };
    let root = |pid: u64| inspect(pid as u32, Some(identity));
    match Job::open_control(&name) {
        Ok(Some(job)) => {
            if !skip_kill {
                if let Err(e) = job.kill() {
                    return failed(format!("cannot end the job {name}: {e}"), None);
                }
            }
            let left = match job.wait_empty(timeout) {
                Ok(n) => n,
                Err(e) => return failed(format!("cannot query the job {name}: {e}"), None),
            };
            if left > 0 {
                return failed(
                    format!(
                        "timed out after {} ms: {left} process(es) still active in the job",
                        timeout.as_millis()
                    ),
                    Some(left),
                );
            }
            match root(pid) {
                Inspection::Absent { .. } => {}
                Inspection::Present { .. } => {
                    return failed(
                        "the job opened by name is empty but the command's root is alive \
                         outside it"
                            .into(),
                        Some(0),
                    )
                }
                Inspection::Unknown { reason } => {
                    return failed(
                        format!("the job is empty but the command's root is unknown: {reason}"),
                        Some(0),
                    )
                }
            }
        }
        Ok(None) => match root(pid) {
            Inspection::Absent { .. } => {}
            Inspection::Present { .. } => {
                return failed(
                    "the job does not exist and the command's root is still running \
                     (nothing is ended by pid)"
                        .into(),
                    None,
                )
            }
            Inspection::Unknown { reason } => {
                return failed(
                    format!("the job does not exist and the command's root is unknown: {reason}"),
                    None,
                )
            }
        },
        Err(e) => return failed(format!("cannot open the job {name}: {e}"), None),
    }
    let fin = json!({
        "v": 1,
        "id": id,
        "reason": reason,
        "detail": detail,
        "cleanupConfirmed": true,
        "scope": "job",
        "activeProcesses": 0,
        "endedAt": now(),
        "by": "stop",
    });
    match publish_once(&fin_path, &fin) {
        Ok(()) => Ok(Stopped {
            confirmed: true,
            answer: fin,
        }),
        Err(e) if crate::fsx::is_exists(&e) => match read_json(&fin_path) {
            // Another stop, or the keeper, recorded a confirmed end first: that is the answer.
            Some(f) if f["cleanupConfirmed"] == true => Ok(Stopped {
                confirmed: true,
                answer: f,
            }),
            // An unconfirmed end was recorded first; it stays, and this stop's proof is the
            // answer.
            other => {
                let mut answer = fin;
                if let Some(f) = other {
                    answer["final"] = f;
                }
                Ok(Stopped {
                    confirmed: true,
                    answer,
                })
            }
        },
        Err(e) => Err(e),
    }
}

/// Whether a `custody stop` has asked for the end: the keeper then leaves `final.json` to it.
pub fn stop_requested(rec: &Path) -> bool {
    rec.join(STOP_REQUEST).exists()
}

#[cfg(windows)]
mod os {
    use std::io;
    use std::mem::{size_of, zeroed};
    use std::os::windows::io::AsRawHandle;
    use std::process::Child;
    use std::ptr::{null, null_mut};
    use std::time::{Duration, Instant};
    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, ERROR_FILE_NOT_FOUND, HANDLE,
    };
    use windows_sys::Win32::System::JobObjects::*;

    /// `JOB_OBJECT_QUERY` (winnt.h).
    const JOB_QUERY: u32 = 0x0004;
    /// `JOB_OBJECT_TERMINATE` (winnt.h).
    const JOB_TERMINATE: u32 = 0x0008;

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// A job handle of our own, closed on drop. Never inheritable.
    pub struct Job(HANDLE);

    impl Drop for Job {
        fn drop(&mut self) {
            // SAFETY: the handle is ours and closed exactly once.
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    impl Job {
        /// Creates the named job, kill-on-close and without breakaway. A name that already exists
        /// is refused (`AlreadyExists`): a job is never shared with an earlier one.
        pub fn create(name: &str) -> io::Result<Job> {
            let name = wide(name);
            // SAFETY: null security attributes (a handle that is not inheritable) and a
            // NUL-terminated name that outlives the call.
            unsafe {
                let h = CreateJobObjectW(null(), name.as_ptr());
                if h.is_null() {
                    return Err(io::Error::last_os_error());
                }
                let job = Job(h);
                if GetLastError() == ERROR_ALREADY_EXISTS {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        "custody job already exists",
                    ));
                }
                let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = zeroed();
                limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                if SetInformationJobObject(
                    job.0,
                    JobObjectExtendedLimitInformation,
                    &limits as *const _ as _,
                    size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                ) == 0
                {
                    return Err(io::Error::last_os_error());
                }
                Ok(job)
            }
        }

        /// Opens the named job to query it; `None` when no such job exists.
        pub fn open_query(name: &str) -> io::Result<Option<Job>> {
            Self::open(name, JOB_QUERY)
        }

        /// Opens the named job to end and query it; `None` when no such job exists. Never
        /// creates one: a job that is gone stays gone.
        pub fn open_control(name: &str) -> io::Result<Option<Job>> {
            Self::open(name, JOB_QUERY | JOB_TERMINATE)
        }

        fn open(name: &str, access: u32) -> io::Result<Option<Job>> {
            let name = wide(name);
            // SAFETY: a NUL-terminated name that outlives the call; the handle is not inheritable.
            unsafe {
                let h = OpenJobObjectW(access, 0, name.as_ptr());
                if h.is_null() {
                    let code = GetLastError();
                    if code == ERROR_FILE_NOT_FOUND {
                        return Ok(None);
                    }
                    return Err(io::Error::from_raw_os_error(code as i32));
                }
                Ok(Some(Job(h)))
            }
        }

        /// Processes alive in the job right now.
        pub fn active(&self) -> io::Result<u32> {
            // SAFETY: a query into a local of the documented size.
            unsafe {
                let mut info: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = zeroed();
                if QueryInformationJobObject(
                    self.0,
                    JobObjectBasicAccountingInformation,
                    &mut info as *mut _ as _,
                    size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                    null_mut(),
                ) == 0
                {
                    return Err(io::Error::last_os_error());
                }
                Ok(info.ActiveProcesses)
            }
        }

        pub fn assign(&self, child: &Child) -> io::Result<()> {
            // SAFETY: both handles are open for the duration of the call.
            if unsafe { AssignProcessToJobObject(self.0, child.as_raw_handle() as HANDLE) } == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }

        /// Ends every process in the job and waits, up to `timeout`, for none to be left. Returns
        /// how many are still active (0: cleanup confirmed).
        pub fn terminate(&self, timeout: Duration) -> io::Result<u32> {
            // A refused end shows up as processes still active when the wait runs out.
            let _ = self.kill();
            self.wait_empty(timeout)
        }

        /// Ends every process in the job (`TerminateJobObject`), without waiting.
        pub fn kill(&self) -> io::Result<()> {
            // SAFETY: the handle is open; an empty job is simply left empty.
            if unsafe { TerminateJobObject(self.0, 1) } == 0 {
                let e = io::Error::last_os_error();
                // An empty job may refuse; what counts is the count of processes left.
                if self.active().is_ok_and(|n| n == 0) {
                    return Ok(());
                }
                return Err(e);
            }
            Ok(())
        }

        /// Waits, up to `timeout`, for no process to be left in the job. Returns how many are
        /// still active (0: empty).
        pub fn wait_empty(&self, timeout: Duration) -> io::Result<u32> {
            let deadline = Instant::now() + timeout;
            loop {
                let n = self.active()?;
                if n == 0 || Instant::now() >= deadline {
                    return Ok(n);
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

#[cfg(not(windows))]
mod os {
    use std::io;
    use std::process::Child;
    use std::time::Duration;

    fn unsupported() -> io::Error {
        io::Error::new(io::ErrorKind::Unsupported, "unsupported on this platform")
    }

    pub struct Job;

    impl Job {
        pub fn create(_name: &str) -> io::Result<Job> {
            Err(unsupported())
        }
        pub fn open_query(_name: &str) -> io::Result<Option<Job>> {
            Err(unsupported())
        }
        pub fn open_control(_name: &str) -> io::Result<Option<Job>> {
            Err(unsupported())
        }
        pub fn kill(&self) -> io::Result<()> {
            Err(unsupported())
        }
        pub fn wait_empty(&self, _timeout: Duration) -> io::Result<u32> {
            Err(unsupported())
        }
        pub fn active(&self) -> io::Result<u32> {
            Err(unsupported())
        }
        pub fn assign(&self, _child: &Child) -> io::Result<()> {
            Err(unsupported())
        }
        pub fn terminate(&self, _timeout: Duration) -> io::Result<u32> {
            Err(unsupported())
        }
    }
}

pub use os::Job;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids() {
        for ok in ["a1", "T04.exec-2", "A_b", &"x".repeat(64)] {
            assert!(valid_id(ok), "{ok}");
        }
        for bad in ["", ".", "..", "a/b", "a\\b", "a b", "á", &"x".repeat(65)] {
            assert!(!valid_id(bad), "{bad}");
        }
    }

    #[test]
    fn job_name_has_twelve_hex_and_the_id() {
        let name = job_name(Path::new("D:\\runs\\r1\\custody"), "a1");
        let rest = name.strip_prefix("Local\\jlock-custody-").unwrap();
        let (hex, id) = rest.split_at(12);
        assert!(hex.bytes().all(|b| b.is_ascii_hexdigit()), "{name}");
        assert_eq!(id, "-a1");
    }

    #[test]
    fn absolute_drops_trailing_separators() {
        let here = std::env::current_dir().unwrap();
        let a = absolute(&here.join("x").join("")).unwrap();
        assert_eq!(a, absolute(&here.join("x")).unwrap());
        assert!(a.is_absolute());
    }
}
