//! Putting a new jlock in place of the one that is running (`jlock update`). Unix lets a running file
//! be renamed over; Windows does not let it be deleted, but lets it be renamed: the old one becomes
//! `jlock.exe.old`, the new one takes its name, and the `.old` is deleted now if possible, or by the
//! next jlock that starts.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn old_path(exe: &Path) -> PathBuf {
    let mut name = exe.as_os_str().to_owned();
    name.push(".old");
    PathBuf::from(name)
}

/// Deletes the `.old` an update left next to this executable because it was still running (only
/// Windows keeps one: a running file there cannot be deleted).
#[cfg(windows)]
pub fn cleanup_old() {
    if let Ok(exe) = std::env::current_exe() {
        let old = old_path(&exe);
        if old.exists() {
            let _ = fs::remove_file(old);
        }
    }
}

#[cfg(unix)]
pub fn make_executable(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))
}

#[cfg(windows)]
pub fn make_executable(_: &Path) -> io::Result<()> {
    Ok(())
}

/// Takes the binary `name` out of `archive` (`.zip` on Windows, `.tar.gz` elsewhere) into `dir`, with
/// the system's `tar` (Windows 10 and later ship one that reads zip files), or PowerShell's .NET on a
/// Windows without it.
pub fn extract(archive: &Path, name: &str, dir: &Path) -> io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let failed = |what: String| io::Error::other(what);
    #[cfg(windows)]
    let out = {
        let system = std::env::var_os("SystemRoot").map(PathBuf::from);
        let tar = system.map(|s| s.join("System32").join("tar.exe"));
        match tar.filter(|t| t.is_file()) {
            Some(tar) => Command::new(tar)
                .arg("-xf")
                .arg(archive)
                .arg("-C")
                .arg(dir)
                .arg(name)
                .output()?,
            None => {
                let script = format!(
                    "Add-Type -AssemblyName System.IO.Compression.FileSystem; \
                     [IO.Compression.ZipFile]::ExtractToDirectory('{}', '{}')",
                    archive.display().to_string().replace('\'', "''"),
                    dir.display().to_string().replace('\'', "''")
                );
                Command::new("powershell")
                    .args(["-NoProfile", "-NonInteractive", "-Command", &script])
                    .env_remove("PSModulePath")
                    .output()?
            }
        }
    };
    #[cfg(unix)]
    let out = {
        let tar = crate::cmd::doctor::find_in_path("tar")
            .into_iter()
            .next()
            .unwrap_or_else(|| PathBuf::from("/usr/bin/tar"));
        Command::new(tar)
            .arg("-xzf")
            .arg(archive)
            .arg("-C")
            .arg(dir)
            .arg(name)
            .output()?
    };
    let path = dir.join(name);
    if !out.status.success() || !path.is_file() {
        return Err(failed(format!(
            "cannot unpack {}: {}",
            archive.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(path)
}

/// `new` takes the place of `exe`; the old file stays as `<exe>.old` until [`finish`] or [`restore`].
pub fn swap(exe: &Path, new: &Path) -> io::Result<()> {
    let old = old_path(exe);
    if old.exists() {
        fs::remove_file(&old)?;
    }
    fs::rename(exe, &old)?;
    if let Err(e) = fs::rename(new, exe) {
        let _ = fs::rename(&old, exe);
        return Err(e);
    }
    Ok(())
}

/// Puts the old executable back after a failed update.
pub fn restore(exe: &Path) -> io::Result<()> {
    let old = old_path(exe);
    let _ = fs::remove_file(exe);
    fs::rename(old, exe)
}

/// The update worked: the old file goes now if the system allows (Windows keeps a running file).
pub fn finish(exe: &Path) {
    let _ = fs::remove_file(old_path(exe));
}

/// The first line `exe --version` prints.
pub fn version_of(exe: &Path) -> Option<String> {
    let out = Command::new(exe).arg("--version").output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    out.status
        .success()
        .then(|| text.lines().next().unwrap_or("").trim().to_string())
}
