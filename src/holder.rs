//! Body of `holder.json` and of the queue tickets (plan section 4), and the UTC clock helpers.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

/// Who asks: what the caller passes in `--owner`, `--group`, `--intent`, `--meta`, `--ttl`.
#[derive(Clone, Debug, Default)]
pub struct Ident {
    pub owner: String,
    pub group: Option<String>,
    pub intent: Option<String>,
    pub meta: BTreeMap<String, String>,
    pub ttl_secs: Option<u64>,
}

impl Ident {
    pub fn default_owner() -> String {
        format!("pid:{}", std::process::id())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Holder {
    pub key: String,
    pub owner: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub pid: u32,
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub since: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl_secs: Option<u64>,
}

impl Holder {
    pub fn new(key: &str, ident: &Ident) -> Holder {
        Holder {
            key: key.to_string(),
            owner: ident.owner.clone(),
            group: ident.group.clone(),
            intent: ident.intent.clone(),
            meta: if ident.meta.is_empty() {
                None
            } else {
                Some(ident.meta.clone())
            },
            pid: std::process::id(),
            host: hostname(),
            cwd: cwd_text(),
            since: iso(now_micros()),
            ttl_secs: ident.ttl_secs,
        }
    }

    /// Placeholder for a file that exists but cannot be parsed (still being written, or damaged).
    pub fn unreadable(key: &str) -> Holder {
        Holder {
            key: key.to_string(),
            owner: "?".into(),
            group: None,
            intent: None,
            meta: None,
            pid: 0,
            host: String::new(),
            cwd: String::new(),
            since: String::new(),
            ttl_secs: None,
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("holder serializes")
    }

    pub fn parse(text: &str) -> Option<Holder> {
        serde_json::from_str(text).ok()
    }

    pub fn since_micros(&self) -> Option<u64> {
        parse_iso(&self.since)
    }

    /// `since + ttl` has passed.
    pub fn ttl_expired(&self, now: u64) -> bool {
        match (self.ttl_secs, self.since_micros()) {
            (Some(ttl), Some(since)) => now > since + ttl * 1_000_000,
            _ => false,
        }
    }
}

pub fn hostname() -> String {
    #[cfg(windows)]
    {
        std::env::var("COMPUTERNAME").unwrap_or_default()
    }
    #[cfg(unix)]
    {
        let mut buf = [0u8; 256];
        // SAFETY: the buffer is valid for its whole length and gethostname writes at most that.
        let rc = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) };
        if rc != 0 {
            return String::new();
        }
        let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        String::from_utf8_lossy(&buf[..end]).into_owned()
    }
}

fn cwd_text() -> String {
    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let cwd = cwd.strip_prefix(r"\\?\").unwrap_or(&cwd);
    if cfg!(windows) {
        cwd.replace('\\', "/")
    } else {
        cwd.to_string()
    }
}

pub fn now_micros() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(0)
}

/// `2026-10-23T23:06:50.123Z`
pub fn iso(micros: u64) -> String {
    let secs = (micros / 1_000_000) as i64;
    let millis = (micros / 1000) % 1000;
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    let rem = secs.rem_euclid(86_400);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        rem / 3600,
        rem / 60 % 60,
        rem % 60
    )
}

pub fn parse_iso(text: &str) -> Option<u64> {
    let b = text.as_bytes();
    if b.len() < 20 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' || b[13] != b':' {
        return None;
    }
    let num = |s: &str| s.parse::<i64>().ok();
    let y = num(&text[0..4])?;
    let mo = num(&text[5..7])?;
    let d = num(&text[8..10])?;
    let h = num(&text[11..13])?;
    let mi = num(&text[14..16])?;
    let s = num(&text[17..19])?;
    let frac = text[19..].trim_end_matches('Z');
    let micros_frac = match frac.strip_prefix('.') {
        Some(f) if !f.is_empty() => {
            let digits: String = f.chars().take(6).collect();
            let n = num(&digits)?;
            n * 10i64.pow(6 - digits.len() as u32)
        }
        _ => 0,
    };
    let days = days_from_civil(y, mo, d);
    let secs = days * 86_400 + h * 3600 + mi * 60 + s;
    u64::try_from(secs * 1_000_000 + micros_frac).ok()
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_round_trip() {
        let t = 1_792_794_410_123_456;
        let s = iso(t);
        assert_eq!(s.len(), 24);
        assert_eq!(parse_iso(&s), Some(t / 1000 * 1000));
        assert_eq!(iso(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(
            parse_iso("2000-03-01T00:00:00.000Z"),
            Some(951_868_800_000_000)
        );
    }
}
