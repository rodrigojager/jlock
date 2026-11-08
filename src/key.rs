//! Key normalization (plan section 5), the per-key mode suffix and the owner path check. Pure
//! string work: nothing here touches the disk.

use sha1::{Digest, Sha1};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Path,
    Name,
    /// `resource:<name>[:<arg>]`, a resource of the catalogue (`resources.rs`).
    Resource,
    All,
}

/// The per-key mode suffix: `src/a.ts@shared`, `resource:desktop@exclusive`. Returns the key without
/// it and the mode word. Only these two exact words count, so any other `@` stays in the key.
pub fn split_mode_suffix(input: &str) -> (&str, Option<&'static str>) {
    let trimmed = input.trim_end();
    for word in ["shared", "exclusive"] {
        if let Some(rest) = trimmed.strip_suffix(word) {
            if let Some(key) = rest.strip_suffix('@') {
                return (key, Some(word));
            }
        }
    }
    (input, None)
}

/// An owner is a path of segments separated by `/` (`gt:run/T18/exec-1`): no empty segment, no
/// leading or trailing `/`. An error is a catalog key.
pub fn check_owner(owner: &str) -> Result<(), String> {
    if owner.split('/').any(|seg| seg.trim().is_empty()) {
        return Err("error.owner_path".into());
    }
    Ok(())
}

/// Normalizes `resource:<name>[:<arg>]` (the part after `resource:`): the name in lower case, both
/// parts trimmed. Whether the name exists and takes an argument is checked against the catalogue
/// by the caller.
fn normalize_resource(rest: &str) -> Result<String, String> {
    let (name, arg) = match rest.split_once(':') {
        Some((n, a)) => (n.trim().to_ascii_lowercase(), Some(a.trim())),
        None => (rest.trim().to_ascii_lowercase(), None),
    };
    if name.is_empty() {
        return Err("error.empty_resource".into());
    }
    match arg {
        Some("") => Err("error.empty_resource_arg".into()),
        Some(a) => Ok(format!("resource:{name}:{a}")),
        None => Ok(format!("resource:{name}")),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Windows,
    Linux,
    Mac,
}

impl Platform {
    pub fn current() -> Platform {
        if cfg!(windows) {
            Platform::Windows
        } else if cfg!(target_os = "macos") {
            Platform::Mac
        } else {
            Platform::Linux
        }
    }

    fn case_insensitive(self) -> bool {
        self != Platform::Linux
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Key {
    pub text: String,
    pub hash: String,
    pub kind: Kind,
}

impl Key {
    /// Rebuilds a key from its already normalized text (as stored in the `key` file).
    pub fn from_text(text: &str) -> Key {
        let kind = if text == "*" {
            Kind::All
        } else if text.starts_with("name:") {
            Kind::Name
        } else if text.starts_with("resource:") {
            Kind::Resource
        } else {
            Kind::Path
        };
        Key {
            text: text.to_string(),
            hash: sha1_hex(text),
            kind,
        }
    }
}

pub fn sha1_hex(text: &str) -> String {
    let digest = Sha1::digest(text.as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Normalizes a key given on the command line, relative to `cwd`, for the running system. An error
/// is a catalog key (`error.empty_key`, `error.empty_name`, `error.empty_resource`...), turned into
/// text by the caller.
pub fn normalize(input: &str, cwd: &std::path::Path) -> Result<Key, String> {
    let home =
        std::env::var(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).unwrap_or_default();
    normalize_for(input, &cwd.to_string_lossy(), &home, Platform::current())
}

/// Same as [`normalize`], with the working directory, home and platform made explicit.
pub fn normalize_for(
    input: &str,
    cwd: &str,
    home: &str,
    platform: Platform,
) -> Result<Key, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err("error.empty_key".into());
    }
    if input == "*" {
        return Ok(Key::from_text("*"));
    }
    if let Some(name) = input.strip_prefix("name:") {
        let name = name.trim();
        if name.is_empty() {
            return Err("error.empty_name".into());
        }
        return Ok(Key::from_text(&format!("name:{name}")));
    }
    if let Some(rest) = input.strip_prefix("resource:") {
        return normalize_resource(rest).map(|text| Key::from_text(&text));
    }
    let path = normalize_path(input, cwd, home, platform);
    Ok(Key::from_text(&path))
}

fn strip_verbatim(s: &str) -> String {
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = s.strip_prefix(r"\\?\").or_else(|| s.strip_prefix("//?/")) {
        rest.to_string()
    } else {
        s.to_string()
    }
}

fn clean(s: &str, platform: Platform) -> String {
    let s = strip_verbatim(s);
    if platform == Platform::Windows {
        s.replace('\\', "/")
    } else {
        s
    }
}

fn has_drive(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':'
}

fn normalize_path(input: &str, cwd: &str, home: &str, platform: Platform) -> String {
    let mut p = clean(input, platform);
    let cwd = clean(cwd, platform);
    if p == "~" || p.starts_with("~/") {
        p = format!("{}{}", clean(home, platform), &p[1..]);
    }

    let joined = if platform == Platform::Windows {
        if has_drive(&p) {
            if p[2..].starts_with('/') {
                p
            } else if has_drive(&cwd) && cwd[..1].eq_ignore_ascii_case(&p[..1]) {
                format!("{cwd}/{}", &p[2..])
            } else {
                format!("{}/{}", &p[..2], &p[2..])
            }
        } else if p.starts_with("//") {
            p
        } else if p.starts_with('/') {
            let drive = if has_drive(&cwd) { &cwd[..2] } else { "" };
            format!("{drive}{p}")
        } else {
            format!("{cwd}/{p}")
        }
    } else if p.starts_with('/') {
        p
    } else {
        format!("{cwd}/{p}")
    };

    // Split off the root: "x:" (drive), "//server/share" (UNC) or "" (Unix "/").
    let (prefix, rest) = if platform == Platform::Windows && has_drive(&joined) {
        (joined[..2].to_string(), joined[2..].to_string())
    } else if platform == Platform::Windows && joined.starts_with("//") {
        let mut parts = joined[2..].splitn(3, '/');
        let server = parts.next().unwrap_or("");
        let share = parts.next().unwrap_or("");
        let rest = parts.next().unwrap_or("");
        (format!("//{server}/{share}"), format!("/{rest}"))
    } else {
        (String::new(), joined)
    };

    let mut parts: Vec<&str> = Vec::new();
    for part in rest.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }

    let mut out = if parts.is_empty() {
        if prefix.starts_with("//") {
            prefix
        } else {
            format!("{prefix}/")
        }
    } else {
        format!("{prefix}/{}", parts.join("/"))
    };
    if platform.case_insensitive() {
        out = out.to_lowercase();
    }
    out
}
