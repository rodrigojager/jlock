//! `jlock update`: the latest GitHub Release (or the version asked for), checked against its
//! `SHA256SUMS`, put in place of the running executable, and run once before success is declared.
//! It runs only when asked: no other command looks at the network.

use crate::cli::{print_json, unknown_option, usage, Arg, CmdResult, Failure, Parser, EXIT_FS};
use crate::cmd::about::REPOSITORY;
use crate::fetch::{self, FetchError};
use crate::i18n::{t, txt, Text};
use crate::selfreplace;
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};

pub const TARGET: &str = env!("BUILD_TARGET_TRIPLE");
const CURRENT: &str = env!("CARGO_PKG_VERSION");

fn failure(msg: Text) -> Failure {
    Failure { code: EXIT_FS, msg }
}

/// Where the archives are and where to ask for the latest release. `JLOCK_UPDATE_BASE` replaces
/// both with `<base>/releases/download` and `<base>/releases/latest` (`file://` works: the tests
/// use a folder).
fn urls() -> (String, String) {
    match std::env::var("JLOCK_UPDATE_BASE") {
        Ok(base) if !base.trim().is_empty() => {
            let base = base.trim().trim_end_matches('/');
            (
                format!("{base}/releases/download"),
                format!("{base}/releases/latest"),
            )
        }
        _ => (
            format!("{REPOSITORY}/releases/download"),
            "https://api.github.com/repos/rodrigojager/jlock/releases/latest".to_string(),
        ),
    }
}

fn fetch_failure(e: FetchError, url: &str) -> Failure {
    failure(match e {
        FetchError::NoTool => txt!("update.no_tool"),
        FetchError::NotFound => txt!("update.not_found", url = url),
        FetchError::Failed(why) => txt!("update.fetch_failed", url = url).with(why),
    })
}

/// `1.2.3` as numbers, for comparing; anything after `-` or `+` is ignored.
fn numbers(version: &str) -> Vec<u64> {
    version
        .trim_start_matches('v')
        .split(['-', '+'])
        .next()
        .unwrap_or("")
        .split('.')
        .map(|p| p.parse().unwrap_or(0))
        .collect()
}

fn newer(a: &str, b: &str) -> bool {
    numbers(a) > numbers(b)
}

/// The tag of the latest release, from the GitHub API answer (or a file shaped like it).
fn latest_tag(latest_url: &str) -> Result<String, Failure> {
    let body = fetch::get(latest_url).map_err(|e| match e {
        FetchError::NotFound => failure(txt!("update.no_release", url = latest_url)),
        e => fetch_failure(e, latest_url),
    })?;
    let value: serde_json::Value = serde_json::from_slice(&body)
        .map_err(|_| failure(txt!("update.bad_answer", url = latest_url)))?;
    value["tag_name"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| failure(txt!("update.bad_answer", url = latest_url)))
}

pub fn asset_name(number: &str, target: &str) -> String {
    let ext = if target.contains("windows") {
        "zip"
    } else {
        "tar.gz"
    };
    format!("jlock-{number}-{target}.{ext}")
}

/// The files of one update attempt, removed whatever happens.
struct Scratch(Vec<PathBuf>);

impl Drop for Scratch {
    fn drop(&mut self) {
        for p in &self.0 {
            if p.is_dir() {
                let _ = fs::remove_dir_all(p);
            } else {
                let _ = fs::remove_file(p);
            }
        }
    }
}

/// A folder the executable lives in, that we cannot write: say so, and what to do.
fn cannot_write(dir: &Path, e: &std::io::Error) -> Failure {
    failure(txt!("update.cannot_write", dir = dir.display()).with(e))
}

/// Downloads, checks and installs `tag` over the running executable.
fn install(tag: &str, number: &str, download_base: &str) -> Result<(), Failure> {
    let exe = std::env::current_exe().map_err(|e| failure(txt!("update.no_exe").with(e)))?;
    let dir = exe.parent().unwrap_or(Path::new(".")).to_path_buf();
    let asset = asset_name(number, TARGET);
    let mut scratch = Scratch(Vec::new());

    let archive = dir.join(format!("jlock-{number}.download.tmp"));
    scratch.0.push(archive.clone());
    // Writing here first tells at once whether the folder is ours to change.
    fs::write(&archive, b"").map_err(|e| cannot_write(&dir, &e))?;
    let url = format!("{download_base}/{tag}/{asset}");
    fetch::download(&url, &archive).map_err(|e| fetch_failure(e, &url))?;
    let sums_url = format!("{download_base}/{tag}/SHA256SUMS");
    let sums = fetch::get(&sums_url).map_err(|e| fetch_failure(e, &sums_url))?;
    let sums = String::from_utf8_lossy(&sums);
    let want = sums
        .lines()
        .filter_map(|l| l.split_once(char::is_whitespace))
        .find(|(_, file)| file.trim().trim_start_matches('*') == asset)
        .map(|(sum, _)| sum.trim().to_ascii_lowercase())
        .ok_or_else(|| failure(txt!("update.not_in_sums", asset = asset)))?;
    let bytes = fs::read(&archive).map_err(|e| cannot_write(&dir, &e))?;
    let got = crate::sha256::hex(&bytes);
    if got != want {
        return Err(failure(txt!(
            "update.bad_sum",
            asset = asset,
            want = want,
            got = got
        )));
    }

    let unpack = dir.join(format!(".jlock-update-{}", std::process::id()));
    scratch.0.push(unpack.clone());
    let name = if cfg!(windows) { "jlock.exe" } else { "jlock" };
    let unpacked = selfreplace::extract(&archive, name, &unpack)
        .map_err(|e| failure(txt!("update.cannot_unpack", asset = asset).with(e)))?;
    let new = dir.join(format!("jlock-{number}.tmp"));
    scratch.0.push(new.clone());
    fs::rename(&unpacked, &new).map_err(|e| cannot_write(&dir, &e))?;
    selfreplace::make_executable(&new).map_err(|e| cannot_write(&dir, &e))?;

    selfreplace::swap(&exe, &new).map_err(|e| cannot_write(&dir, &e))?;
    let answer = selfreplace::version_of(&exe);
    if answer.as_deref() != Some(format!("jlock {number}").as_str()) {
        let restored = selfreplace::restore(&exe);
        let answer = answer.unwrap_or_else(|| "-".into());
        return Err(failure(match restored {
            Ok(()) => txt!("update.check_failed", answer = answer),
            Err(e) => txt!("update.restore_failed", path = exe.display()).with(e),
        }));
    }
    selfreplace::finish(&exe);
    Ok(())
}

pub fn run(args: Vec<String>) -> CmdResult {
    let mut p = Parser::new(args);
    let (mut check, mut json) = (false, false);
    let mut asked: Option<String> = None;
    while let Some(arg) = p.next() {
        match arg {
            Arg::Flag(f) if f == "check" => check = true,
            Arg::Flag(f) if f == "json" => json = true,
            Arg::Flag(f) if f == "version" => asked = Some(p.value(&f)?),
            Arg::Flag(f) => return Err(unknown_option("update", &f)),
            _ => return Err(usage(txt!("error.takes_no_args", cmd = "update"))),
        }
    }
    let (download_base, latest_url) = urls();
    let tag = match &asked {
        Some(v) => {
            let v = v.trim();
            if numbers(v).iter().all(|n| *n == 0) {
                return Err(usage(txt!("update.bad_version", version = v)));
            }
            if v.starts_with('v') {
                v.to_string()
            } else {
                format!("v{v}")
            }
        }
        None => latest_tag(&latest_url)?,
    };
    let number = tag.trim_start_matches('v').to_string();
    let available = newer(&number, CURRENT);

    if check {
        if json {
            print_json(
                &json!({ "result": "ok", "current": CURRENT, "latest": number,
                                "update_available": available }),
            );
        } else {
            println!("{}", t!("update.current", version = CURRENT));
            println!("{}", t!("update.latest", version = number));
            println!(
                "{}",
                if available {
                    t!("update.available")
                } else {
                    t!("update.up_to_date")
                }
            );
        }
        return Ok(0);
    }

    // Without --version, only a newer release is installed; with it, that exact version.
    let wanted = if asked.is_some() {
        number != CURRENT
    } else {
        available
    };
    if !wanted {
        if json {
            print_json(
                &json!({ "result": "ok", "current": CURRENT, "latest": number,
                                "updated": false }),
            );
        } else {
            println!("{}", t!("update.already", version = CURRENT));
        }
        return Ok(0);
    }
    install(&tag, &number, &download_base)?;
    if json {
        print_json(
            &json!({ "result": "ok", "current": CURRENT, "latest": number,
                            "updated": true }),
        );
    } else {
        println!("{}", t!("update.done", from = CURRENT, to = number));
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions() {
        assert!(newer("0.2.0", "0.1.9"));
        assert!(newer("v1.0.0", "0.10.0"));
        assert!(!newer("0.1.0", "0.1.0"));
        assert!(!newer("0.1.0-beta", "0.1.0"));
        assert_eq!(
            asset_name("0.2.0", "x86_64-pc-windows-msvc"),
            "jlock-0.2.0-x86_64-pc-windows-msvc.zip"
        );
        assert_eq!(
            asset_name("0.2.0", "aarch64-apple-darwin"),
            "jlock-0.2.0-aarch64-apple-darwin.tar.gz"
        );
    }
}
