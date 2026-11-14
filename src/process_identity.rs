//! Process identity: is there a process with this pid, and is it the same incarnation as before?
//! The answer is `present` (with an opaque identity token), `absent` (the system says there is no
//! such process, or it has exited) or `unknown` (anything else: access denied, an error, another
//! platform). Silence is never taken as death. Nothing here ever stops a process.

use serde_json::{json, Value};

/// The answer for one pid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inspection {
    Present { identity: String },
    Absent { reason: Option<String> },
    Unknown { reason: String },
}

impl Inspection {
    pub fn state(&self) -> &'static str {
        match self {
            Inspection::Present { .. } => "present",
            Inspection::Absent { .. } => "absent",
            Inspection::Unknown { .. } => "unknown",
        }
    }

    pub fn identity(&self) -> Option<&str> {
        match self {
            Inspection::Present { identity } => Some(identity),
            _ => None,
        }
    }

    pub fn reason(&self) -> Option<&str> {
        match self {
            Inspection::Absent { reason } => reason.as_deref(),
            Inspection::Unknown { reason } => Some(reason),
            Inspection::Present { .. } => None,
        }
    }

    /// The `--json` shape: `{ "state", "identity"?, "reason"? }`.
    pub fn to_json(&self) -> Value {
        let mut v = json!({ "state": self.state() });
        if let Some(id) = self.identity() {
            v["identity"] = json!(id);
        }
        if let Some(r) = self.reason() {
            v["reason"] = json!(r);
        }
        v
    }
}

/// Inspects `pid`; with `expected`, a present process of another incarnation is `absent` with
/// reason `replaced` (the original has ended; the new one is never touched).
pub fn inspect(pid: u32, expected: Option<&str>) -> Inspection {
    match (os::inspect(pid), expected) {
        (Inspection::Present { identity }, Some(want)) if identity != want => Inspection::Absent {
            reason: Some("replaced".into()),
        },
        (found, _) => found,
    }
}

/// Whether this platform answers `present`/`absent` at all.
pub const SUPPORTED: bool = cfg!(windows);

/// Classifies the error of opening a process: only "no such process" proves absence; access
/// denied and every other error leave the answer unknown.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn classify_open_error(code: u32) -> Inspection {
    const ERROR_ACCESS_DENIED: u32 = 5;
    const ERROR_INVALID_PARAMETER: u32 = 87;
    match code {
        ERROR_INVALID_PARAMETER => Inspection::Absent {
            reason: Some("not found".into()),
        },
        ERROR_ACCESS_DENIED => Inspection::Unknown {
            reason: format!("access denied (os error {code})"),
        },
        _ => Inspection::Unknown {
            reason: format!("cannot open process (os error {code})"),
        },
    }
}

/// The token of one incarnation: the pid and its creation time (a FILETIME, in decimal).
#[cfg_attr(not(windows), allow(dead_code))]
pub fn token(pid: u32, creation: u64) -> String {
    format!("win:{pid}:{creation}")
}

#[cfg(windows)]
mod os {
    use super::{classify_open_error, token, Inspection};
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, FILETIME, STILL_ACTIVE};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    pub fn inspect(pid: u32) -> Inspection {
        // SAFETY: plain Win32 calls; the handle is closed before returning and every out pointer
        // points to a local.
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                return classify_open_error(GetLastError());
            }
            let zero = FILETIME {
                dwLowDateTime: 0,
                dwHighDateTime: 0,
            };
            let (mut creation, mut exit, mut kernel, mut user) = (zero, zero, zero, zero);
            // Times and exit code come from the same open handle, so both describe one incarnation.
            let found = if GetProcessTimes(h, &mut creation, &mut exit, &mut kernel, &mut user) == 0
            {
                Inspection::Unknown {
                    reason: format!("cannot read process times (os error {})", GetLastError()),
                }
            } else {
                let mut code = 0u32;
                if GetExitCodeProcess(h, &mut code) == 0 {
                    Inspection::Unknown {
                        reason: format!("cannot read exit code (os error {})", GetLastError()),
                    }
                } else if code as i32 != STILL_ACTIVE {
                    Inspection::Absent {
                        reason: Some("exited".into()),
                    }
                } else {
                    // A process that exited with code 259 also reads as still active: it stays
                    // `present`, the safe side (never a false `absent`).
                    let ft = (u64::from(creation.dwHighDateTime) << 32)
                        | u64::from(creation.dwLowDateTime);
                    Inspection::Present {
                        identity: token(pid, ft),
                    }
                }
            };
            CloseHandle(h);
            found
        }
    }
}

#[cfg(not(windows))]
mod os {
    use super::Inspection;

    pub fn inspect(_pid: u32) -> Inspection {
        Inspection::Unknown {
            reason: "unsupported".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_no_such_process_is_absent() {
        assert_eq!(classify_open_error(87).state(), "absent");
        let denied = classify_open_error(5);
        assert_eq!(denied.state(), "unknown");
        assert!(denied.reason().unwrap().contains("access denied"));
        for code in [0, 1, 6, 31, 299, 1314] {
            let other = classify_open_error(code);
            assert_eq!(other.state(), "unknown", "os error {code}");
            assert!(other.reason().is_some());
        }
    }

    #[test]
    fn json_shape() {
        let p = Inspection::Present {
            identity: token(42, 7),
        };
        assert_eq!(
            p.to_json(),
            json!({ "state": "present", "identity": "win:42:7" })
        );
        let a = Inspection::Absent { reason: None };
        assert_eq!(a.to_json(), json!({ "state": "absent" }));
        let u = Inspection::Unknown {
            reason: "unsupported".into(),
        };
        assert_eq!(
            u.to_json(),
            json!({ "state": "unknown", "reason": "unsupported" })
        );
    }

    #[test]
    fn this_process_is_present_and_keeps_its_identity() {
        let me = std::process::id();
        let first = inspect(me, None);
        if !SUPPORTED {
            assert_eq!(first.state(), "unknown");
            return;
        }
        let id = first.identity().expect("present").to_string();
        assert_eq!(inspect(me, Some(&id)).state(), "present");
        assert_eq!(
            inspect(me, Some("win:1:1")),
            Inspection::Absent {
                reason: Some("replaced".into())
            }
        );
    }
}
