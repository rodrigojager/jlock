# jlock roadmap

What is done, what comes next, and what is deliberately out of scope. Dates are intentions, not promises.
The design behind every item lives in [`docs/PLAN.md`](docs/PLAN.md) (Portuguese); the Linux/macOS work has
its own plan in [`docs/PLAN-LINUX-MACOS.md`](docs/PLAN-LINUX-MACOS.md).

## Done: 0.1.0 (October 2026)

The whole core protocol, developed and tested on Windows 10 / NTFS:

- one owner per key, decided by an atomic exclusive create;
- strict arrival-order queue (one ticket per waiter, ordered by arrival time);
- heartbeat for holders and waiters; locks and tickets of dead processes recovered by anyone who passes by;
- path hierarchy (a folder covers what is inside it), `name:` keys, `*`;
- sets of keys taken all-or-nothing in canonical order;
- deadlock refused before queueing, with the chain; simultaneous cycles broken deterministically;
- `hold`, `run` (like `flock(1)`), `heartbeat`, `release`, `status`, `list --watch`, `graph`,
  `unlock --force`, `gc`;
- an operating-system lock under `hold`/`run`, so the kernel frees it the instant a process dies and a late
  heartbeat is not mistaken for death;
- `log.jsonl` with rotation, derived `index.json`, local and global sweeps, and the zero-leftovers invariant
  under stress (8 processes × 1000 cycles with 20 induced deaths);
- CI matrix for Windows, Linux and macOS (the Linux/macOS legs are defined but have not run yet; see below).

## Next: 0.2, "usable by anyone"

Tracked as work items 11–17 in the maintainers' ticket folder.

1. **Internationalization.** Catalogs embedded in the binary; `--lang`, `JLOCK_LANG`, or the system language;
   English as reference and fallback. Commands and options stay English; everything explained to a human is
   translated. Languages: English, Português (Brasil), Español, Français, Deutsch, Italiano, 简体中文, 日本語,
   Русский, العربية, हिन्दी. JSON output, the log and exit codes never change with the language.
2. **Didactic help.** `jlock help <command>` explains every command and **every option** (what it does, default,
   example), with real examples and the exit codes that command can return; hints after `busy`, `timeout`,
   `not_owner`, `lost`; a legend in `list`; `jlock about` with author and links.
3. **`jlock doctor`.** Checks the registry folder, the three file-system primitives the protocol relies on, the
   OS lock on that file system, the `JLOCK_*` settings, the `PATH`, and the registry's current state; says
   what to fix.
4. **Releases and one-command install.** Binaries for Windows x64, Linux x64/arm64 (static musl), macOS
   Intel/Apple Silicon on every `v*` tag, with `SHA256SUMS`; `install.sh` and `install.ps1` one-liners;
   `cargo binstall jlock`.
5. **`jlock update`.** Fetches the latest release through the system's `curl` (PowerShell as fallback on
   Windows), verifies the checksum and replaces itself. Only when asked: jlock never touches the network on its
   own.
6. **Translated catalogs** for the nine remaining languages, once every message exists. Five of them
   (简体中文, 日本語, Русский, العربية, हिन्दी) ship marked "awaiting native review"; corrections to any
   language are welcome, see `CONTRIBUTING.md`.
7. **READMEs** in all eleven languages: why jlock exists, the problem it solves, why existing tools were not
   enough, install, five-minute tour, agent/script/human paths, how it works, guarantees and limits. Examples
   in the READMEs are re-executed by a test so they never lie.

## Then: Linux and macOS hardening

The code compiles for Unix and the design is portable, but it has only ever run on Windows. Plan:
[`docs/PLAN-LINUX-MACOS.md`](docs/PLAN-LINUX-MACOS.md).

- Run the full suite on real Linux and macOS in CI and fix what breaks.
- Signals and pipes: `SIGPIPE`/`EPIPE` when output is piped (`jlock list | head`), `sigaction` instead of
  `signal`, `SIGHUP`, signal pass-through in `run`.
- Shared `JLOCK_HOME` between Unix users: directory permissions (setgid group, no sticky bit), `doctor` checks,
  documentation.
- Network and special file systems: what `create_new`, `rename` and `flock` guarantee on NFS, SMB, 9p (WSL),
  FUSE; `doctor` detects the file-system type and warns.
- Path normalization on Unix: symlinks (`/tmp` → `/private/tmp` on macOS), case-sensitive APFS volumes, an
  opt-in canonicalization.
- Static musl builds, arm64 for Linux and macOS, size and startup checks.
- Packaging: Homebrew tap, `.deb`/`.rpm`, AUR; shell completions for bash/zsh/fish/PowerShell; a man page
  generated from the same help table.

## Later / ideas

- Publish on crates.io (`cargo install jlock`).
- winget and scoop manifests for Windows; Windows on ARM builds.
- Filter `list` by metadata (`--meta task=T04`), `graph --dot`.
- `jlock events`: follow `log.jsonl` as a stream, for dashboards.
- Panels: `jlock list --watch` inside herdr; a locks section in the Claude Code / OpenCode HUD showing the
  current repository's locks (the first consumer is goal-tasks).
- Opt-in default `--ttl` policy per `JLOCK_HOME`.

## Not planned

- Coordination across machines (jlock is one machine, one file system by design; put a real lock service there).
- Mandatory locking: jlock is advisory; only programs that call it respect it. Agent frameworks enforce it with
  their own guards.
- A daemon. Everything is on disk so that nothing has to stay alive.

## Proposing something

Open an issue describing the situation that needs it (who is waiting for whom, on which system). Items move
from "ideas" to "next" when a real user needs them.
