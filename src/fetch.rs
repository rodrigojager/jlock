//! Downloads for `jlock update`, through the system's `curl` (on Windows, PowerShell when there is no
//! curl): jlock has no HTTP client of its own. This is the only place jlock touches the network, and
//! only `update` calls it.

use crate::cmd::doctor::find_in_path;
use std::path::{Path, PathBuf};
use std::process::Command;

pub enum FetchError {
    /// Neither curl nor (on Windows) PowerShell is in `PATH`.
    NoTool,
    /// The server (or the folder, for `file://`) has nothing at this address.
    NotFound,
    Failed(String),
}

enum Tool {
    Curl(PathBuf),
    PowerShell(PathBuf),
}

/// The tool, looked up in `PATH` only (so that a `PATH` without curl really means no curl).
fn tool() -> Option<Tool> {
    if let Some(curl) = find_in_path("curl").into_iter().next() {
        return Some(Tool::Curl(curl));
    }
    if cfg!(windows) {
        let ps = find_in_path("powershell")
            .into_iter()
            .chain(find_in_path("pwsh"))
            .next();
        return ps.map(Tool::PowerShell);
    }
    None
}

/// Text for a single-quoted PowerShell string.
fn quoted(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

/// Saves what `url` points at into `dest` (`https://` or `file://`), giving up after 60 seconds.
pub fn download(url: &str, dest: &Path) -> Result<(), FetchError> {
    let out = match tool().ok_or(FetchError::NoTool)? {
        Tool::Curl(curl) => Command::new(curl)
            .args(["-fsSL", "--retry", "2", "--max-time", "60"])
            .args(["-A", concat!("jlock/", env!("CARGO_PKG_VERSION"))])
            .arg("-o")
            .arg(dest)
            .arg(url)
            .output(),
        Tool::PowerShell(ps) => {
            let script = format!(
                "$ErrorActionPreference = 'Stop'; $ProgressPreference = 'SilentlyContinue'; \
                 $u = {}; $d = {}; \
                 if ($u -like 'file://*') {{ Copy-Item -LiteralPath ([Uri]$u).LocalPath -Destination $d }} \
                 else {{ [Net.ServicePointManager]::SecurityProtocol = \
                 [Net.ServicePointManager]::SecurityProtocol -bor 3072; \
                 Invoke-WebRequest -UseBasicParsing -TimeoutSec 60 -Uri $u -OutFile $d }}",
                quoted(url),
                quoted(&dest.to_string_lossy())
            );
            Command::new(ps)
                .args(["-NoProfile", "-NonInteractive", "-Command", &script])
                .env_remove("PSModulePath")
                .output()
        }
    }
    .map_err(|e| FetchError::Failed(e.to_string()))?;
    if out.status.success() {
        return Ok(());
    }
    let _ = std::fs::remove_file(dest);
    let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
    // curl: 22 = HTTP error such as 404, 37 = a file:// that does not exist.
    let missing = matches!(out.status.code(), Some(22) | Some(37))
        || err.contains("404")
        || err.contains("Cannot find path");
    Err(if missing {
        FetchError::NotFound
    } else {
        FetchError::Failed(err)
    })
}

/// What `url` points at, in memory (small files: the release JSON, `SHA256SUMS`).
pub fn get(url: &str) -> Result<Vec<u8>, FetchError> {
    let tmp = std::env::temp_dir().join(format!(
        "jlock-fetch-{}-{:04x}.tmp",
        std::process::id(),
        crate::wait::random_u64() & 0xffff
    ));
    download(url, &tmp)?;
    let bytes = std::fs::read(&tmp).map_err(|e| FetchError::Failed(e.to_string()));
    let _ = std::fs::remove_file(&tmp);
    bytes
}
