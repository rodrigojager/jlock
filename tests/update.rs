//! `jlock update` against a fake release in a folder, served by `file://` (`JLOCK_UPDATE_BASE`). The
//! binary under test is a copy in a temporary folder: the `jlock` installed in `~/.cargo/bin` is
//! never touched. The "new" release is this same binary with a marker appended (executables run
//! fine with trailing bytes), so the test can tell the file was replaced; `JLOCK_FAKE_VERSION` (read
//! by debug builds only) makes it answer `--version` as the release's version.

mod common;

#[path = "../src/sha256.rs"]
mod sha256;

use common::Env;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const MARKER: &[u8] = b"\nJLOCK-UPDATE-TEST-MARKER\n";
const NAME: &str = if cfg!(windows) { "jlock.exe" } else { "jlock" };

struct Fake {
    env: Env,
    exe: PathBuf,
    base: String,
    original: Vec<u8>,
}

fn file_url(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    if text.starts_with('/') {
        format!("file://{text}")
    } else {
        format!("file:///{text}")
    }
}

fn target(env: &Env) -> String {
    env.run(&["about", "--json"]).json()["target"]
        .as_str()
        .unwrap()
        .to_string()
}

/// Packs `stage/<NAME>` the way the release workflow does.
fn pack(stage: &Path, archive: &Path) {
    let ok = if cfg!(windows) {
        Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command"])
            .arg(format!(
                "Compress-Archive -Path '{}' -DestinationPath '{}' -Force",
                stage.join(NAME).display(),
                archive.display()
            ))
            .env_remove("PSModulePath")
            .status()
    } else {
        Command::new("tar")
            .arg("-czf")
            .arg(archive)
            .arg("-C")
            .arg(stage)
            .arg(NAME)
            .status()
    };
    assert!(ok.unwrap().success(), "packing {}", archive.display());
}

/// A copy of jlock in `<home>/bin`, and a release `latest` → `tag` in `<home>/rel`.
fn setup(name: &str, tag: &str, good_sum: bool) -> Fake {
    let env = Env::new(&format!("update-{name}"));
    let root = env.home.join("rel");
    let bin = env.home.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let exe = bin.join(NAME);
    std::fs::copy(common::JLOCK, &exe).unwrap();
    let original = std::fs::read(&exe).unwrap();

    let number = tag.trim_start_matches('v');
    let target = target(&env);
    let ext = if target.contains("windows") {
        "zip"
    } else {
        "tar.gz"
    };
    let asset = format!("jlock-{number}-{target}.{ext}");
    let dir = root.join("releases").join("download").join(tag);
    let stage = env.home.join("stage");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::create_dir_all(&stage).unwrap();
    let mut new = original.clone();
    new.extend_from_slice(MARKER);
    std::fs::write(stage.join(NAME), &new).unwrap();
    pack(&stage, &dir.join(&asset));
    let sum = if good_sum {
        sha256::hex(&std::fs::read(dir.join(&asset)).unwrap())
    } else {
        "0".repeat(64)
    };
    std::fs::write(dir.join("SHA256SUMS"), format!("{sum}  {asset}\n")).unwrap();
    std::fs::write(
        root.join("releases").join("latest"),
        format!("{{ \"tag_name\": \"{tag}\" }}"),
    )
    .unwrap();
    Fake {
        env,
        exe,
        base: file_url(&root),
        original,
    }
}

impl Fake {
    fn run(&self, args: &[&str], fake_version: Option<&str>, path: Option<&str>) -> Output {
        let mut cmd = Command::new(&self.exe);
        cmd.args(args);
        self.env.apply(&mut cmd);
        cmd.env("JLOCK_UPDATE_BASE", &self.base);
        if let Some(v) = fake_version {
            cmd.env("JLOCK_FAKE_VERSION", v);
        }
        if let Some(p) = path {
            cmd.env("PATH", p);
        }
        cmd.output().unwrap()
    }

    fn bin_files(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.exe.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }
}

fn text(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr)
}

#[test]
fn update_replaces_the_executable() {
    let fake = setup("ok", "v9.9.9", true);
    let out = fake.run(&["update"], Some("9.9.9"), None);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out));
    assert!(text(&out).contains("updated jlock"), "{}", text(&out));
    let now = std::fs::read(&fake.exe).unwrap();
    assert!(now.ends_with(MARKER), "the file was not replaced");

    // The new binary runs; on Windows its first start deletes the `.old` the update left.
    let v = fake.run(&["--version"], None, None);
    assert_eq!(
        String::from_utf8_lossy(&v.stdout).trim(),
        format!("jlock {}", env!("CARGO_PKG_VERSION"))
    );
    assert_eq!(fake.bin_files(), [NAME], "leftovers next to the executable");
}

#[test]
fn check_compares_and_writes_nothing() {
    let fake = setup("check", "v9.9.9", true);
    let before = fake.bin_files();
    let out = fake.run(&["update", "--check", "--json"], None, None);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["current"], env!("CARGO_PKG_VERSION"));
    assert_eq!(v["latest"], "9.9.9");
    assert_eq!(v["update_available"], true);
    assert_eq!(fake.bin_files(), before);
    assert_eq!(std::fs::read(&fake.exe).unwrap(), fake.original);

    let out = fake.run(&["update", "--check"], None, None);
    assert!(
        text(&out).contains("an update is available"),
        "{}",
        text(&out)
    );
}

#[test]
fn already_at_the_latest_changes_nothing() {
    let tag = format!("v{}", env!("CARGO_PKG_VERSION"));
    let fake = setup("same", &tag, true);
    let out = fake.run(&["update"], None, None);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out));
    assert!(text(&out).contains("already at"), "{}", text(&out));
    assert_eq!(std::fs::read(&fake.exe).unwrap(), fake.original);
    assert_eq!(fake.bin_files(), [NAME]);
}

#[test]
fn wrong_checksum_keeps_the_old_binary() {
    let fake = setup("badsum", "v9.9.9", false);
    let out = fake.run(&["update"], Some("9.9.9"), None);
    assert_eq!(out.status.code(), Some(20), "{}", text(&out));
    assert!(text(&out).contains("checksum mismatch"), "{}", text(&out));
    assert_eq!(std::fs::read(&fake.exe).unwrap(), fake.original);
    assert_eq!(fake.bin_files(), [NAME], "leftovers next to the executable");
}

#[test]
fn without_curl_it_says_so() {
    let fake = setup("nocurl", "v9.9.9", true);
    let out = fake.run(&["update"], Some("9.9.9"), Some(""));
    assert_eq!(out.status.code(), Some(20), "{}", text(&out));
    assert!(text(&out).contains("update needs curl"), "{}", text(&out));
    assert_eq!(std::fs::read(&fake.exe).unwrap(), fake.original);
}

#[test]
fn a_missing_release_is_a_clear_error() {
    let fake = setup("missing", "v9.9.9", true);
    std::fs::remove_file(fake.env.home.join("rel").join("releases").join("latest")).unwrap();
    let out = fake.run(&["update", "--check"], None, None);
    assert_eq!(out.status.code(), Some(20), "{}", text(&out));
    assert!(text(&out).contains("no release found"), "{}", text(&out));
}
