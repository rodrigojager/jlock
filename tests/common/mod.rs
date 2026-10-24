//! Test helpers: a temporary `JLOCK_HOME` per test (under `target/tmp`, never the real one), short
//! times by environment, and children that are only ever our own processes, stopped by their PID.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

pub const JLOCK: &str = env!("CARGO_BIN_EXE_jlock");

pub struct Env {
    pub home: PathBuf,
    vars: Vec<(String, String)>,
}

pub struct Done {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Done {
    pub fn json(&self) -> serde_json::Value {
        let line = self.stdout.lines().last().unwrap_or("");
        serde_json::from_str(line)
            .unwrap_or_else(|e| panic!("not JSON ({e}): {:?} / {:?}", self.stdout, self.stderr))
    }
}

impl Env {
    pub fn new(name: &str) -> Env {
        let home = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("home-{name}"));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        let vars = [
            ("JLOCK_POLL_MS", "10"),
            ("JLOCK_HEARTBEAT_MS", "100"),
            ("JLOCK_STALE_MS", "1500"),
        ];
        Env {
            home,
            vars: vars
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    /// Overrides (or adds) an environment variable for every child of this environment.
    pub fn set(&mut self, name: &str, value: &str) {
        self.vars.retain(|(k, _)| k != name);
        self.vars.push((name.to_string(), value.to_string()));
    }

    pub fn cmd(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(JLOCK);
        cmd.args(args)
            .env("JLOCK_HOME", &self.home)
            .env_remove("JLOCK_DEBUG")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (k, v) in &self.vars {
            cmd.env(k, v);
        }
        cmd
    }

    pub fn run(&self, args: &[&str]) -> Done {
        let out = self.cmd(args).output().expect("run jlock");
        Done {
            code: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        }
    }

    pub fn spawn(&self, args: &[&str]) -> Kid {
        let mut cmd = self.cmd(args);
        cmd.stdin(Stdio::piped());
        Kid::start(cmd)
    }

    pub fn locks(&self) -> PathBuf {
        self.home.join("locks")
    }

    /// Names of the key folders under `locks/`.
    pub fn lock_dirs(&self) -> Vec<String> {
        match std::fs::read_dir(self.locks()) {
            Ok(entries) => entries
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    /// A path inside the test home, usable as a path key on any system.
    pub fn path(&self, rel: &str) -> String {
        self.home
            .join("keys")
            .join(rel)
            .to_string_lossy()
            .into_owned()
    }

    /// Waits until `cond` holds, up to `timeout`.
    pub fn wait_for(&self, timeout: Duration, mut cond: impl FnMut() -> bool) -> bool {
        let start = Instant::now();
        while start.elapsed() < timeout {
            if cond() {
                return true;
            }
            sleep(Duration::from_millis(5));
        }
        cond()
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            let _ = std::fs::remove_dir_all(&self.home);
        }
    }
}

/// A child process of this test. Dropping it stops it (by its own handle/PID) if still running.
pub struct Kid {
    child: Child,
    pub pid: u32,
}

impl Kid {
    pub fn start(mut cmd: Command) -> Kid {
        let child = cmd.spawn().expect("spawn child");
        let pid = child.id();
        Kid { child, pid }
    }

    pub fn stdin(&mut self) -> &mut ChildStdin {
        self.child.stdin.as_mut().expect("stdin piped")
    }

    pub fn send(&mut self, line: &str) {
        use std::io::Write;
        let stdin = self.stdin();
        let _ = writeln!(stdin, "{line}");
        let _ = stdin.flush();
    }

    /// Exit code if it ended within `timeout`.
    pub fn wait(&mut self, timeout: Duration) -> Option<i32> {
        let start = Instant::now();
        loop {
            if let Some(status) = self.child.try_wait().expect("try_wait") {
                return Some(status.code().unwrap_or(-1));
            }
            if start.elapsed() >= timeout {
                return None;
            }
            sleep(Duration::from_millis(5));
        }
    }

    pub fn running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Stops this child (only this one: it is our own process).
    pub fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    /// Waits for the end (panics after `timeout`) and collects the output.
    pub fn finish(mut self, timeout: Duration) -> Done {
        let code = match self.wait(timeout) {
            Some(code) => code,
            None => {
                self.kill();
                panic!("child {} did not finish in {timeout:?}", self.pid);
            }
        };
        use std::io::Read;
        let mut stdout = String::new();
        let mut stderr = String::new();
        if let Some(mut out) = self.child.stdout.take() {
            let _ = out.read_to_string(&mut stdout);
        }
        if let Some(mut err) = self.child.stderr.take() {
            let _ = err.read_to_string(&mut stderr);
        }
        Done {
            code,
            stdout,
            stderr,
        }
    }

    /// Reads one line of stdout (blocking).
    pub fn read_line(&mut self) -> String {
        use std::io::Read;
        let out = self.child.stdout.as_mut().expect("stdout piped");
        let mut line = Vec::new();
        let mut byte = [0u8; 1];
        while out.read(&mut byte).unwrap_or(0) == 1 {
            if byte[0] == b'\n' {
                break;
            }
            line.push(byte[0]);
        }
        String::from_utf8_lossy(&line).trim_end().to_string()
    }
}

impl Drop for Kid {
    fn drop(&mut self) {
        if self.running() {
            self.kill();
        }
    }
}
