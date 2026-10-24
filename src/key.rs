//! Key normalization (plan section 5). Pure string work: nothing here touches the disk.

use sha1::{Digest, Sha1};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Path,
    Name,
    All,
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
/// is a catalog key (`error.empty_key`, `error.empty_name`), turned into text by the caller.
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
