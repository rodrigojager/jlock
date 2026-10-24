//! The installers, against a release laid out in a temporary folder and served by `file://`: the
//! freshly built binary is packed as the release workflow packs it, with a correct `SHA256SUMS`;
//! the script installs it into a temporary folder and the installed binary answers `--version`. A
//! wrong checksum makes the script fail without installing anything. The real `PATH` is never
//! touched (`-NoPath` / `JLOCK_NO_PATH=1`).

use std::path::{Path, PathBuf};
use std::process::Command;

const JLOCK: &str = env!("CARGO_BIN_EXE_jlock");
const VERSION: &str = env!("CARGO_PKG_VERSION");

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("installers-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn script(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(name)
}

/// `file:///D:/x/y` or `file:///tmp/x`.
fn file_url(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    if text.starts_with('/') {
        format!("file://{text}")
    } else {
        format!("file:///{text}")
    }
}

fn version_line() -> String {
    format!("jlock {VERSION}")
}

#[cfg(windows)]
mod windows {
    use super::*;

    const TARGET: &str = "x86_64-pc-windows-msvc";

    fn ps(shell: &str, command: &str) -> std::process::Output {
        Command::new(shell)
            .args(["-NoProfile", "-NonInteractive", "-Command", command])
            .env_remove("PSModulePath")
            .output()
            .expect("run PowerShell")
    }

    /// `<root>/v<version>/jlock-<version>-<target>.zip` and `SHA256SUMS` next to it.
    fn release(root: &Path, good_sum: bool) {
        let dir = root.join(format!("v{VERSION}"));
        let stage = root.join("stage");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(&stage).unwrap();
        std::fs::copy(JLOCK, stage.join("jlock.exe")).unwrap();
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        for f in ["LICENSE", "README.md"] {
            std::fs::copy(manifest.join(f), stage.join(f)).unwrap();
        }
        let asset = format!("jlock-{VERSION}-{TARGET}.zip");
        let zip = dir.join(&asset);
        let out = ps(
            "powershell",
            &format!(
                "Compress-Archive -Path '{}\\*' -DestinationPath '{}' -Force; \
                 (Get-FileHash -Algorithm SHA256 -LiteralPath '{}').Hash.ToLower()",
                stage.display(),
                zip.display(),
                zip.display()
            ),
        );
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let mut sum = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !good_sum {
            sum = "0".repeat(64);
        }
        std::fs::write(dir.join("SHA256SUMS"), format!("{sum}  {asset}\n")).unwrap();
    }

    fn install(shell: &str, base: &Path, into: &Path) -> std::process::Output {
        Command::new(shell)
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ])
            .env_remove("PSModulePath")
            .arg(script("install.ps1"))
            .args(["-BaseUrl", &file_url(base)])
            .args(["-Version", &format!("v{VERSION}")])
            .arg("-InstallDir")
            .arg(into)
            .arg("-NoPath")
            .output()
            .expect("run install.ps1")
    }

    fn shells() -> Vec<&'static str> {
        let mut out = vec!["powershell"];
        if Command::new("pwsh").arg("-Version").output().is_ok() {
            out.push("pwsh");
        }
        out
    }

    #[test]
    fn install_ps1_installs_a_working_binary() {
        for shell in shells() {
            let root = scratch(&format!("ps-ok-{shell}"));
            release(&root, true);
            let into = root.join("bin");
            let out = install(shell, &root, &into);
            assert!(
                out.status.success(),
                "{shell}: {}\n{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
            let exe = into.join("jlock.exe");
            let v = Command::new(&exe).arg("--version").output().unwrap();
            assert_eq!(String::from_utf8_lossy(&v.stdout).trim(), version_line());

            // Again, over the installed one (the old file is renamed out of the way first).
            let out = install(shell, &root, &into);
            assert!(out.status.success(), "{shell} reinstall");
            assert!(exe.exists());
            let _ = std::fs::remove_dir_all(&root);
        }
    }

    #[test]
    fn install_ps1_refuses_a_wrong_checksum() {
        for shell in shells() {
            let root = scratch(&format!("ps-bad-{shell}"));
            release(&root, false);
            let into = root.join("bin");
            let out = install(shell, &root, &into);
            assert!(!out.status.success(), "{shell} installed despite a bad sum");
            let err = String::from_utf8_lossy(&out.stderr).into_owned()
                + &String::from_utf8_lossy(&out.stdout);
            assert!(err.contains("checksum mismatch"), "{shell}: {err}");
            assert!(!into.join("jlock.exe").exists());
            let _ = std::fs::remove_dir_all(&root);
        }
    }
}

#[cfg(unix)]
mod unix {
    use super::*;

    fn target() -> String {
        let arch = std::env::consts::ARCH;
        match std::env::consts::OS {
            "macos" => format!("{arch}-apple-darwin"),
            _ => format!("{arch}-unknown-linux-musl"),
        }
    }

    fn sha256(path: &Path) -> String {
        let out = Command::new("sh")
            .arg("-c")
            .arg("if command -v sha256sum >/dev/null; then sha256sum \"$1\"; else shasum -a 256 \"$1\"; fi")
            .arg("sh")
            .arg(path)
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout)
            .split_whitespace()
            .next()
            .unwrap()
            .to_string()
    }

    fn release(root: &Path, good_sum: bool) {
        let dir = root.join(format!("v{VERSION}"));
        let stage = root.join("stage");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(&stage).unwrap();
        std::fs::copy(JLOCK, stage.join("jlock")).unwrap();
        let asset = format!("jlock-{VERSION}-{}.tar.gz", target());
        let tgz = dir.join(&asset);
        let ok = Command::new("tar")
            .arg("-czf")
            .arg(&tgz)
            .arg("-C")
            .arg(&stage)
            .arg("jlock")
            .status()
            .unwrap();
        assert!(ok.success());
        let sum = if good_sum {
            sha256(&tgz)
        } else {
            "0".repeat(64)
        };
        std::fs::write(dir.join("SHA256SUMS"), format!("{sum}  {asset}\n")).unwrap();
    }

    fn install(root: &Path, into: &Path) -> std::process::Output {
        Command::new("sh")
            .arg(script("install.sh"))
            .env("JLOCK_BASE_URL", file_url(root))
            .env("JLOCK_VERSION", format!("v{VERSION}"))
            .env("JLOCK_INSTALL_DIR", into)
            .env("JLOCK_NO_PATH", "1")
            .output()
            .expect("run install.sh")
    }

    #[test]
    fn install_sh_installs_a_working_binary() {
        let root = scratch("sh-ok");
        release(&root, true);
        let into = root.join("bin");
        let out = install(&root, &into);
        assert!(
            out.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        let v = Command::new(into.join("jlock"))
            .arg("--version")
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&v.stdout).trim(), version_line());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn install_sh_refuses_a_wrong_checksum() {
        let root = scratch("sh-bad");
        release(&root, false);
        let into = root.join("bin");
        let out = install(&root, &into);
        assert!(!out.status.success());
        assert!(String::from_utf8_lossy(&out.stderr).contains("checksum mismatch"));
        assert!(!into.join("jlock").exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
