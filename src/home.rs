//! Where the registry lives (plan section 3.3): `JLOCK_HOME`, or the per-system default.

use std::path::{Path, PathBuf};

fn env_path(name: &str) -> Option<PathBuf> {
    match std::env::var_os(name) {
        Some(v) if !v.is_empty() => Some(PathBuf::from(v)),
        _ => None,
    }
}

/// The registry root. Not created here.
pub fn root() -> Result<PathBuf, String> {
    if let Some(p) = env_path("JLOCK_HOME") {
        return Ok(p);
    }
    if cfg!(windows) {
        return env_path("LOCALAPPDATA")
            .map(|p| p.join("jlock"))
            .ok_or_else(|| "LOCALAPPDATA is not set; set JLOCK_HOME".to_string());
    }
    let home = env_path("HOME").ok_or_else(|| "HOME is not set; set JLOCK_HOME".to_string());
    if cfg!(target_os = "macos") {
        return Ok(home?
            .join("Library")
            .join("Application Support")
            .join("jlock"));
    }
    if let Some(state) = env_path("XDG_STATE_HOME").filter(|p| p.is_absolute()) {
        return Ok(state.join("jlock"));
    }
    Ok(home?.join(".local").join("state").join("jlock"))
}

/// `locks/` under the root, created on demand.
pub fn locks_dir(root: &Path) -> std::io::Result<PathBuf> {
    let dir = root.join("locks");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}
