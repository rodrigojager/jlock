//! `jlock doctor`: is this machine ready for locks? It checks and suggests, never fixes: the only
//! files it writes are its own temporary ones in `JLOCK_HOME`, removed even when a check fails, and it
//! never creates `JLOCK_HOME`, runs a sweep or touches `PATH`.

use crate::cli::{print_json, unknown_option, usage, Arg, CmdResult, Parser, EXIT_FS, EXIT_OK};
use crate::fsx;
use crate::i18n::{pad, t, txt, width, Text};
use crate::index;
use crate::oslock::{self, Try};
use crate::store::{dir_is_empty, KeyDir, Store};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Status {
    Ok,
    Warn,
    Fail,
}

impl Status {
    fn word(self) -> &'static str {
        match self {
            Status::Ok => "ok",
            Status::Warn => "warn",
            Status::Fail => "fail",
        }
    }
}

struct Check {
    name: &'static str,
    status: Status,
    detail: Text,
    fix: Option<Text>,
}

fn ok(name: &'static str, detail: Text) -> Check {
    Check {
        name,
        status: Status::Ok,
        detail,
        fix: None,
    }
}

fn warn(name: &'static str, detail: Text, fix: Text) -> Check {
    Check {
        name,
        status: Status::Warn,
        detail,
        fix: Some(fix),
    }
}

fn fail(name: &'static str, detail: Text, fix: Text) -> Check {
    Check {
        name,
        status: Status::Fail,
        detail,
        fix: Some(fix),
    }
}

/// `a; b`, in each language.
fn joined(a: &Text, b: &Text) -> Text {
    Text {
        local: format!("{}; {}", a.local, b.local),
        en: format!("{}; {}", a.en, b.en),
    }
}

/// The doctor's own files, removed when it ends, whatever happened.
struct Scratch(Vec<PathBuf>);

impl Scratch {
    fn file(&mut self, root: &Path, what: &str) -> PathBuf {
        let rand = crate::wait::random_u64() & 0xffff;
        let path = root.join(format!(
            "doctor-{}-{rand:04x}-{what}.tmp",
            std::process::id()
        ));
        self.0.push(path.clone());
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        for p in &self.0 {
            let _ = fs::remove_file(p);
        }
    }
}

/// Every place `program` is found in `PATH`, in order (with `.exe` on Windows).
pub fn find_in_path(program: &str) -> Vec<PathBuf> {
    let name = if cfg!(windows) {
        format!("{program}.exe")
    } else {
        program.to_string()
    };
    let Some(path) = std::env::var_os("PATH") else {
        return Vec::new();
    };
    std::env::split_paths(&path)
        .map(|dir| dir.join(&name))
        .filter(|p| p.is_file())
        .collect()
}

fn same_file(a: &Path, b: &Path) -> bool {
    let canon = |p: &Path| fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let (a, b) = (canon(a), canon(b));
    if cfg!(windows) {
        a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
    } else {
        a == b
    }
}

fn shown(p: &Path) -> String {
    p.display().to_string()
}

/// (1) the version and the executable that is running.
fn check_version() -> Check {
    let exe = std::env::current_exe().map_or("?".into(), |p| shown(&p));
    ok(
        "version",
        txt!(
            "doctor.version",
            version = env!("CARGO_PKG_VERSION"),
            path = exe
        ),
    )
}

/// (2) `JLOCK_HOME`: where, why there, and whether it is a writable folder. `Some(root)` when the
/// next checks can use it.
fn check_home(scratch: &mut Scratch) -> (Check, Option<PathBuf>) {
    let root = match crate::home::root() {
        Ok(root) => root,
        Err(k) => return (fail("home", txt!(k), txt!("doctor.home_set_fix")), None),
    };
    let origin = if std::env::var_os("JLOCK_HOME").is_some_and(|v| !v.is_empty()) {
        txt!("doctor.home_from_env")
    } else {
        txt!("doctor.home_default")
    };
    let path = shown(&root);
    let where_ = |state: Text| Text {
        local: format!("{path} ({}): {}", origin.local, state.local),
        en: format!("{path} ({}): {}", origin.en, state.en),
    };
    if !root.exists() {
        let fix = txt!("doctor.home_missing_fix", path = path);
        return (fail("home", where_(txt!("doctor.home_missing")), fix), None);
    }
    if !root.is_dir() {
        let fix = txt!("doctor.home_not_dir_fix", path = path);
        return (fail("home", where_(txt!("doctor.home_not_dir")), fix), None);
    }
    let probe = scratch.file(&root, "probe");
    match fsx::create_exclusive(&probe, b"jlock doctor") {
        Ok(()) => {
            let _ = fs::remove_file(&probe);
            (ok("home", where_(txt!("doctor.home_ok"))), Some(root))
        }
        Err(e) => {
            let fix = txt!("doctor.home_readonly_fix", path = path);
            let detail = where_(txt!("doctor.home_readonly").with(&e));
            (fail("home", detail, fix), None)
        }
    }
}

/// (3) the three primitives of the protocol: `create_new` (and its refusal), `rename` over an
/// existing file, `remove`.
fn check_primitives(root: &Path, scratch: &mut Scratch) -> Check {
    let a = scratch.file(root, "a");
    let b = scratch.file(root, "b");
    let broken = |step: &str, e: Option<std::io::Error>| {
        let detail = txt!("doctor.primitive_failed", step = step);
        let detail = match e {
            Some(e) => detail.with(e),
            None => detail,
        };
        fail("primitives", detail, txt!("doctor.primitives_fix"))
    };
    if let Err(e) = fsx::create_exclusive(&a, b"a") {
        return broken("create_new", Some(e));
    }
    match fsx::create_exclusive(&a, b"again") {
        Err(e) if fsx::is_exists(&e) => {}
        Err(e) => return broken("create_new (EEXIST)", Some(e)),
        Ok(()) => return broken("create_new (EEXIST)", None),
    }
    if let Err(e) = fsx::create_exclusive(&b, b"b") {
        return broken("create_new", Some(e));
    }
    if let Err(e) = fsx::rename_with_retry(&a, &b) {
        return broken("rename", Some(e));
    }
    if a.exists() || fs::read(&b).ok().as_deref() != Some(b"a".as_slice()) {
        return broken("rename", None);
    }
    match fsx::remove_with_retry(&b) {
        Ok(true) if !b.exists() => ok("primitives", txt!("doctor.primitives_ok")),
        Ok(_) => broken("remove", None),
        Err(e) => broken("remove", Some(e)),
    }
}

/// (4) the operating system's lock, which `hold` and `run` keep on `holder.json`.
fn check_oslock(root: &Path, scratch: &mut Scratch) -> Check {
    let file = scratch.file(root, "lock");
    if let Err(e) = fsx::create_exclusive(&file, b"lock") {
        return fail(
            "oslock",
            txt!("doctor.primitive_failed", step = "create_new").with(e),
            txt!("doctor.primitives_fix"),
        );
    }
    let unsupported = || {
        warn(
            "oslock",
            txt!("doctor.oslock_unsupported"),
            txt!("doctor.oslock_unsupported_fix"),
        )
    };
    let first = match oslock::try_lock(&file) {
        Ok(Try::Locked(lock)) => lock,
        Ok(Try::Busy) => {
            return fail(
                "oslock",
                txt!("doctor.oslock_wrong"),
                txt!("doctor.primitives_fix"),
            )
        }
        Ok(Try::Unsupported) | Err(_) => return unsupported(),
    };
    let second = oslock::try_lock(&file);
    drop(first);
    match second {
        Ok(Try::Busy) => ok("oslock", txt!("doctor.oslock_ok")),
        Ok(Try::Locked(_)) => fail(
            "oslock",
            txt!("doctor.oslock_wrong"),
            txt!("doctor.primitives_fix"),
        ),
        Ok(Try::Unsupported) | Err(_) => unsupported(),
    }
}

/// The file system holding `path`, and whether it is on the network, when the system says.
#[cfg(windows)]
fn file_system(path: &Path) -> Option<(String, bool)> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        GetDriveTypeW, GetVolumeInformationW, GetVolumePathNameW,
    };
    const DRIVE_REMOTE: u32 = 4;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    let mut volume = [0u16; 512];
    let mut name = [0u16; 64];
    // SAFETY: every buffer is passed with its length in UTF-16 units and outlives the calls.
    unsafe {
        if GetVolumePathNameW(wide.as_ptr(), volume.as_mut_ptr(), volume.len() as u32) == 0 {
            return None;
        }
        let remote = GetDriveTypeW(volume.as_ptr()) == DRIVE_REMOTE;
        let got = GetVolumeInformationW(
            volume.as_ptr(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            name.as_mut_ptr(),
            name.len() as u32,
        );
        if got == 0 {
            return None;
        }
        let len = name.iter().position(|c| *c == 0).unwrap_or(name.len());
        Some((String::from_utf16_lossy(&name[..len]), remote))
    }
}

#[cfg(target_os = "macos")]
fn file_system(path: &Path) -> Option<(String, bool)> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: statfs fills the zeroed struct; the path is a valid C string.
    let mut st: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    let name: String = st
        .f_fstypename
        .iter()
        .take_while(|c| **c != 0)
        .map(|c| *c as u8 as char)
        .collect();
    let remote = matches!(name.as_str(), "nfs" | "smbfs" | "afpfs" | "webdav");
    Some((name, remote))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn file_system(path: &Path) -> Option<(String, bool)> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: statfs fills the zeroed struct; the path is a valid C string.
    let mut st: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    #[allow(clippy::unnecessary_cast)]
    let magic = (st.f_type as u64) & 0xFFFF_FFFF;
    let (name, remote) = match magic {
        0xEF53 => ("ext2/3/4", false),
        0x9123_683E => ("btrfs", false),
        0x5846_5342 => ("xfs", false),
        0x2FC1_2FC1 => ("zfs", false),
        0x0102_1994 => ("tmpfs", false),
        0x794C_7630 => ("overlayfs", false),
        0x4D44 => ("vfat", false),
        0x2011_BAB0 => ("exfat", false),
        0x6573_5546 => ("fuse", false),
        0x0102_1997 => ("9p", true),
        0x6969 => ("nfs", true),
        0xFF53_4D42 => ("cifs", true),
        0xFE53_4D42 => ("smb2", true),
        _ => return Some((format!("0x{magic:x}"), false)),
    };
    Some((name.to_string(), remote))
}

/// (5) which file system the registry is on. Informative.
fn check_fs(root: &Path) -> Check {
    match file_system(root) {
        Some((name, false)) => ok("filesystem", txt!("doctor.fs", name = name)),
        Some((name, true)) => ok("filesystem", txt!("doctor.fs_remote", name = name)),
        None => ok("filesystem", txt!("doctor.fs_unknown")),
    }
}

/// A millisecond (or byte) setting: its value in effect, and a problem if it is set but invalid.
fn setting(name: &'static str, default: u64, problems: &mut Vec<Text>) -> u64 {
    match std::env::var(name) {
        Ok(v) => match v.trim().parse::<u64>() {
            Ok(n) if n > 0 => n,
            _ => {
                problems.push(txt!(
                    "doctor.var_invalid",
                    name = name,
                    value = v,
                    default = default
                ));
                default
            }
        },
        Err(_) => default,
    }
}

const KNOWN_VARS: &[&str] = &[
    "JLOCK_HOME",
    "JLOCK_HEARTBEAT_MS",
    "JLOCK_STALE_MS",
    "JLOCK_POLL_MS",
    "JLOCK_SWEEP_MS",
    "JLOCK_LOG_MAX_BYTES",
    "JLOCK_LANG",
    "JLOCK_HINTS",
    "JLOCK_DEBUG",
    "JLOCK_UPDATE_BASE",
];

/// (6) the `JLOCK_*` variables: the values in effect, and the combinations that do not work.
fn check_vars() -> Check {
    let mut problems = Vec::new();
    let heartbeat = setting("JLOCK_HEARTBEAT_MS", 5000, &mut problems);
    let stale = setting("JLOCK_STALE_MS", 30000, &mut problems);
    let poll = setting("JLOCK_POLL_MS", 250, &mut problems);
    let sweep = setting("JLOCK_SWEEP_MS", 60000, &mut problems);
    let log = setting("JLOCK_LOG_MAX_BYTES", 5_242_880, &mut problems);
    if stale <= heartbeat {
        problems.push(txt!(
            "doctor.var_stale_heartbeat",
            stale = stale,
            heartbeat = heartbeat
        ));
    }
    if poll > stale {
        problems.push(txt!("doctor.var_poll_stale", poll = poll, stale = stale));
    }
    if let Ok(lang) = std::env::var("JLOCK_LANG") {
        if !lang.trim().is_empty() && crate::i18n::resolve(&lang).is_none() {
            problems.push(txt!("doctor.var_lang", value = lang));
        }
    }
    for (name, _) in std::env::vars_os() {
        let name = name.to_string_lossy();
        if name.starts_with("JLOCK_")
            && !name.starts_with("JLOCK_TEST_")
            && !KNOWN_VARS.contains(&name.as_ref())
        {
            problems.push(txt!("doctor.var_unknown", name = name));
        }
    }
    let values = txt!(
        "doctor.vars",
        heartbeat = heartbeat,
        stale = stale,
        poll = poll,
        sweep = sweep,
        log = log,
        lang = crate::i18n::lang()
    );
    if problems.is_empty() {
        return ok("variables", values);
    }
    let join = |f: fn(&Text) -> &String| {
        problems
            .iter()
            .map(|p| f(p).clone())
            .collect::<Vec<_>>()
            .join("; ")
    };
    let detail = Text {
        local: format!("{}; {}", values.local, join(|p| &p.local)),
        en: format!("{}; {}", values.en, join(|p| &p.en)),
    };
    warn("variables", detail, txt!("doctor.vars_fix"))
}

/// (7) the `jlock` that a shell would run is this one.
fn check_path() -> Check {
    let me = std::env::current_exe().unwrap_or_default();
    let found = find_in_path("jlock");
    match found.first() {
        None => warn(
            "path",
            txt!("doctor.path_missing"),
            txt!(
                "doctor.path_missing_fix",
                dir = me.parent().map_or("?".into(), shown)
            ),
        ),
        Some(first) if same_file(first, &me) => ok("path", txt!("doctor.path_ok")),
        Some(first) => warn(
            "path",
            txt!("doctor.path_other", other = shown(first), me = shown(&me)),
            txt!("doctor.path_other_fix"),
        ),
    }
}

/// (8) what the registry holds right now. Read only.
fn check_registry(root: &Path) -> Check {
    let store = Store {
        root: root.to_path_buf(),
        locks: root.join("locks"),
    };
    let mut unknown = 0;
    let mut broken = 0;
    if let Ok(entries) = fs::read_dir(&store.locks) {
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            match index::key_of_dir(&path) {
                Some(key) => {
                    let kd = KeyDir { path, key };
                    if !dir_is_empty(&kd.broken_dir()) {
                        broken += 1;
                    }
                }
                None => unknown += 1,
            }
        }
    }
    let states = index::scan(&store).unwrap_or_default();
    let held = states.iter().filter(|s| s.holder.is_some()).count();
    let queued = states.iter().filter(|s| !s.tickets.is_empty()).count();
    let dead = states
        .iter()
        .filter(|s| {
            s.holder
                .as_ref()
                .is_some_and(|h| crate::cmd::status::liveness(&s.dir, h).0)
        })
        .count();
    let sweep = fs::read_to_string(root.join("last-sweep"))
        .ok()
        .and_then(|t| t.trim().parse::<u64>().ok())
        .map_or_else(
            || txt!("doctor.never"),
            |at| {
                let ms = crate::holder::now_micros().saturating_sub(at) / 1000;
                txt!("doctor.ago", time = crate::cmd::status::human(ms))
            },
        );
    let log = crate::sweep::log_bytes(&store);
    let detail = Text::nested(
        "doctor.registry",
        &[
            ("held", &held),
            ("queued", &queued),
            ("dead", &dead),
            ("log", &log),
        ],
        &[("sweep", &sweep)],
    );
    if unknown + broken > 0 {
        let more = txt!("doctor.registry_debris", unknown = unknown, broken = broken);
        return warn(
            "registry",
            joined(&detail, &more),
            txt!("doctor.registry_fix"),
        );
    }
    ok("registry", detail)
}

/// (9) `curl`, which `jlock update` uses (PowerShell is the fallback on Windows). Informative.
fn check_curl() -> Check {
    if let Some(curl) = find_in_path("curl").first() {
        return ok("curl", txt!("doctor.curl_ok", path = shown(curl)));
    }
    if cfg!(windows) && !find_in_path("powershell").is_empty() {
        return ok("curl", txt!("doctor.curl_powershell"));
    }
    warn(
        "curl",
        txt!("doctor.curl_missing"),
        txt!("doctor.curl_missing_fix"),
    )
}

fn run_checks() -> Vec<Check> {
    let mut scratch = Scratch(Vec::new());
    let mut checks = vec![check_version()];
    let (home, root) = check_home(&mut scratch);
    checks.push(home);
    match &root {
        Some(root) => {
            checks.push(check_primitives(root, &mut scratch));
            checks.push(check_oslock(root, &mut scratch));
            checks.push(check_fs(root));
        }
        None => {
            for name in ["primitives", "oslock", "filesystem"] {
                checks.push(warn(
                    name,
                    txt!("doctor.skipped"),
                    txt!("doctor.skipped_fix"),
                ));
            }
        }
    }
    checks.push(check_vars());
    checks.push(check_path());
    match &root {
        Some(root) => checks.push(check_registry(root)),
        None => checks.push(warn(
            "registry",
            txt!("doctor.skipped"),
            txt!("doctor.skipped_fix"),
        )),
    }
    checks.push(check_curl());
    checks
}

pub fn run(args: Vec<String>) -> CmdResult {
    let mut p = Parser::new(args);
    let mut json = false;
    while let Some(arg) = p.next() {
        match arg {
            Arg::Flag(f) if f == "json" => json = true,
            Arg::Flag(f) => return Err(unknown_option("doctor", &f)),
            _ => return Err(usage(txt!("error.takes_no_args", cmd = "doctor"))),
        }
    }
    let checks = run_checks();
    let count = |s: Status| checks.iter().filter(|c| c.status == s).count();
    let (oks, warns, fails) = (count(Status::Ok), count(Status::Warn), count(Status::Fail));
    if json {
        let list: Vec<_> = checks
            .iter()
            .map(|c| {
                json!({ "name": c.name, "status": c.status.word(), "detail": c.detail.en,
                        "fix": c.fix.as_ref().map(|f| &f.en) })
            })
            .collect();
        print_json(&json!({
            "result": if fails == 0 { "ok" } else { "error" },
            "checks": list,
            "summary": { "ok": oks, "warn": warns, "fail": fails },
        }));
    } else {
        let labels: Vec<String> = checks
            .iter()
            .map(|c| t!(&format!("doctor.name.{}", c.name)))
            .collect();
        let col = labels.iter().map(|l| width(l)).max().unwrap_or(0);
        let words: Vec<String> = [Status::Ok, Status::Warn, Status::Fail]
            .iter()
            .map(|s| t!(&format!("doctor.status.{}", s.word())))
            .collect();
        let wcol = words.iter().map(|w| width(w)).max().unwrap_or(0);
        let indent = " ".repeat(wcol + 2 + col + 2);
        for (c, label) in checks.iter().zip(&labels) {
            let word = &words[c.status as usize];
            let first = format!("{word}{}  {label}{}  ", pad(word, wcol), pad(label, col));
            print!(
                "{}",
                crate::help::wrap(&c.detail.local, &first, &indent, crate::help::COLUMNS)
            );
            if let Some(fix) = &c.fix {
                let first = format!("{indent}{}: ", t!("doctor.fix"));
                print!(
                    "{}",
                    crate::help::wrap(&fix.local, &first, &indent, crate::help::COLUMNS)
                );
            }
        }
        println!(
            "\n{}",
            t!("doctor.summary", ok = oks, warn = warns, fail = fails)
        );
    }
    Ok(if fails == 0 { EXIT_OK } else { EXIT_FS })
}
