# jlock command reference

Every command, option, exit code, variable and file of jlock. Why jlock exists, how to install it and a
first tour are in the [README](../README.md) (in eleven languages); `jlock help <command>` prints the
same reference in the terminal, in your language. The design is in [`PLAN.md`](PLAN.md) (Portuguese);
the decisions taken while building it are in [`DEVIATIONS.md`](DEVIATIONS.md).

## Keys

| Key | Meaning | Conflicts with |
|---|---|---|
| a path: `D:\proj\src\a.ts`, `src/a.ts`, `~/notes`, `.` | the file or folder, resolved to an absolute path; `\` becomes `/`; lower case on Windows and macOS; no trailing `/`. The path does not have to exist. | the same path, every path inside it (if it is a folder), every folder above it, and `*` |
| `name:<text>` | any resource that is not a path: `name:port-3000`, `name:deploy-prod` | the same name, and `*` |
| `*` | everything | every key |

`D:\Proj\SRC\`, `d:/proj/src` and `src` (run from `D:\proj`) are the same key.

## Commands

Every command accepts `--json` and then prints one JSON object with a `result` field:
`granted`, `queued`, `timeout`, `deadlock`, `released`, `not_owner`, `lost`, `ok` or `error`.
Without `--json` it prints one short line per key.

### `acquire` — take keys and keep them

```
jlock acquire <key...> [--owner id] [--group g] [--intent "..."] [--meta k=v]... [--ttl 2h]
              [--wait | --no-wait] [--timeout 30s] [--json]
```

```
jlock acquire D:\proj\src\a.ts --owner agent-7 --group run-42 --intent "refactor parser"
granted d:/proj/src/a.ts
```

- Waits its turn by default (no limit). `--timeout 30s` gives up after 30 s (exit 11);
  `--no-wait` never waits (exit 10, with the holder and the position it would have had).
- Several keys are taken all or none, in a fixed (sorted) order, so two callers asking for
  `{a,b}` and `{b,a}` never block each other.
- `--owner` defaults to `pid:<pid>`; whoever presents the same owner can release. Asking again for a
  key you already hold is granted at once.
- `--ttl` is a hard limit on how long the lock may be held, even with a heartbeat.
- A lock taken by `acquire` stays after the command exits. Keep it alive with `jlock heartbeat`
  (at least every `JLOCK_STALE_MS`), or use `hold`/`run`, which do that for you.

### `run` — hold keys while a command runs (like `flock(1)`)

```
jlock run D:\proj\dist -- cargo build --release
```

Takes the keys (same options as `acquire`), runs the command, renews the heartbeat while it runs,
releases when it ends and exits with the command's exit code. If the lock is lost (forced, expired) the
command is stopped and `run` exits 14. If `run` itself is killed, the lock expires.

### `hold` — hold keys until told to stop

```
jlock hold D:\proj\src --owner agent-7 --group run-42
granted d:/proj/src
```

Prints `granted`, then stays alive renewing the heartbeat. It releases and exits 0 on a `release` line
on standard input, CTRL+C or SIGTERM. When the owner gives a key back with `jlock release`, `hold` prints
`released <key>`, keeps the others, and exits 0 when none is left (the log shows a `release`, not a loss).
If the lock is taken away (`unlock --force`, expiration) it prints `lost <key>` and exits 14. While held,
`holder.json` also carries an operating-system lock (`LockFileEx` / `flock`), so the kernel frees it the
instant the process dies.

```
jlock hold src/a.ts src/b.ts --owner agent-7 --group task-42 --ttl 2h --detach
granted d:/proj/src/a.ts
granted d:/proj/src/b.ts
jlock release --group task-42 --owner agent-7
```

`--detach` makes it one call: `hold` waits its turn (or answers `busy` with `--no-wait`, `timeout` with
`--timeout`), prints the same answer with the same exit code, and returns; a second `jlock hold`,
started detached from the terminal (its own session on Unix; `DETACHED_PROCESS` and, when the caller's
job allows it, out of that job on Windows), keeps the lock alive until `jlock release`, its `--ttl` or
`unlock --force`. It writes nothing anywhere afterwards; the log keeps what happened. This is the way for
an agent to hold files for a whole task: a lock taken with plain `acquire` expires after
`JLOCK_STALE_MS` (30 s) without a `jlock heartbeat`, and an agent spends minutes between two commands.
A `--ttl` is a good idea with `--detach`, as a limit if the agent never releases.

### `release` — give keys back

```
jlock release D:\proj\src\a.ts --owner agent-7
jlock release --owner agent-7                 # everything agent-7 holds
jlock release --group run-42 --owner agent-7  # agent-7's locks in the group
jlock release --group run-42 --any-owner      # the whole group
```

Only the owner can release a key (exit 13 otherwise). The key's folder disappears once nobody holds
or waits for it.

### `heartbeat` — renew a lock taken with `acquire`

```
jlock heartbeat D:\proj\src\a.ts --owner agent-7
renewed d:/proj/src/a.ts
```

Exit 13 with `lost` if the lock is not yours any more.

### `status` — one key

```
jlock status D:\proj\src\a.ts
free d:/proj/src/a.ts
conflicts:
  d:/proj/src held by agent-3, 1 waiting
  why: a lock on a folder covers everything inside it, so a held folder above this key blocks it
```

Shows the holder (owner, since, heartbeat, group, intent), the queue with positions, and the
conflicting keys that would block it: a folder above, files inside, or `*`.

### `list` — everything locked on this machine

```
jlock list [--owner x] [--group g] [--under D:\proj] [--stale] [--json] [--watch]
KEY                  OWNER    GROUP   INTENT    HELD   HEARTBEAT  QUEUE
d:/proj/src          agent-3  run-42  refactor  4m10s  2s         1
name:port-3000       dev      -       -         15s    1s         0

2 locks, 1 waiting
```

Read straight from the folder, so it is always the truth. `--under` keeps paths at or below a folder;
`--stale` only dead locks (heartbeat older than `JLOCK_STALE_MS` with nobody holding the OS lock, or past
their ttl); `--watch` redraws every second until CTRL+C. A `hold`/`run` whose heartbeat is late but still
has the OS lock is alive: it is shown as `late`, not `dead`, and the expiration leaves it alone. The
footer counts locks and waiters, and explains `(dead)` and `(late)` when they appear;
`jlock help list` explains every column.

### `graph` — who waits for whom

```
jlock graph
agent-7 -> agent-3  (d:/proj/src held by agent-3)
no cycles
```

A request that would close a cycle is refused before it queues, with the chain (exit 12):
`B → name:x (held by A) → A → name:y (held by B) → B`. If two requests close a cycle at the same
instant, the newer one gives up.

### `unlock --force` — break someone else's lock

```
jlock unlock D:\proj\src --force --reason "agent hung"
jlock unlock --owner agent-3 --force
jlock unlock --group run-42 --force --by ci
```

Writes `forced` to the log with `by` (default `user:<login>`) and `reason`. The owner's `hold`/`run`
notices on its next heartbeat and stops with 14. Without `--force` it refuses (exit 2).

### `gc` — sweep now

```
jlock gc
expired locks:   1
expired tickets: 0
broken consumed: 0
folders removed: 1
log bytes:       48211
```

Any command already does this at most once per `JLOCK_SWEEP_MS`; `gc` does it now and reports.

### `doctor` — is this machine ready for locks?

```
jlock doctor [--json]
ok    version          jlock 0.1.0, running from C:\Users\me\.cargo\bin\jlock.exe
ok    registry folder  C:\Users\me\AppData\Local\jlock (the default for this system): a writable folder
ok    primitives       create_new refuses an existing file, rename replaces one, remove deletes
ok    OS lock          taken, refused to a second handle, then released
ok    file system      NTFS
ok    variables        heartbeat 5000 ms, stale after 30000 ms, poll 250 ms, sweep every 60000 ms,
                       log up to 5242880 bytes, language en
ok    PATH             this jlock is the first one in PATH
ok    registry         0 held, 0 with a queue, 0 dead; last sweep: 12s ago; log: 4096 bytes
ok    curl             found: C:\Windows\system32\curl.exe (jlock update uses it)

ok: 9, warnings: 0, failures: 0
```

Nine checks, in this order: the version and the executable running; the registry folder (`JLOCK_HOME`
or the system default), whether it exists, is a folder and is writable; the three file operations of
the protocol in that folder (`create_new` and its refusal of an existing file, `rename` over a file,
`remove`); the OS lock used by `hold` and `run` (`warn` when the file system has none: network or
FUSE); the file system; the `JLOCK_*` variables (`warn` for invalid values, `JLOCK_STALE_MS` not above
`JLOCK_HEARTBEAT_MS`, `JLOCK_POLL_MS` above `JLOCK_STALE_MS`, unknown names); whether another `jlock`
comes first in `PATH`; what the registry holds (`warn` with a suggestion of `jlock gc` for folders
without a readable key or leftovers in `broken/`); and `curl`, which `update` uses.

Each `warn` and `fail` comes with what to do. Exit 0 when nothing failed (warnings do not change the
code), 20 when something did. `--json` prints `{ "result": "ok" | "error", "checks": [{name, status,
detail, fix}], "summary": {ok, warn, fail} }`. The doctor only suggests: it never creates `JLOCK_HOME`
(a missing one is a `fail` with the command that creates it), never runs `gc`, never touches `PATH`,
and the temporary files it uses in the registry folder are removed even when a check fails.

### `update` — install the latest release

```
jlock update --check          # this version, the latest release, and whether to update
jlock update                  # install the latest release over this jlock
jlock update --version v0.1.0 # install one exact version (also an older one)
```

Asks GitHub for the latest release (or takes `--version`), downloads the archive for this system,
checks it against the release's `SHA256SUMS`, puts the new executable in place of the running one, and
runs it once (`--version`) before saying it worked; if that fails the old one is put back. On Windows
the old file is renamed to `jlock.exe.old` (a running executable cannot be deleted) and the next jlock
that starts deletes it. Without write permission in the executable's folder it stops with exit 20 and
says so; it never asks for more privileges by itself.

Downloads go through the system's `curl` (Windows 10 and later, macOS and Linux have it), or PowerShell
on Windows when there is no curl; without either it stops with exit 20. **jlock only uses the network
in `update`, and only when you run it**: no other command checks for new versions. `--json` prints
`{ "result": "ok", "current", "latest", "update_available" }` with `--check`, `"updated": true|false`
otherwise. Until a first release is published, `update` answers exit 20, "no release found".
`JLOCK_UPDATE_BASE` points it somewhere else (`<base>/releases/latest` and
`<base>/releases/download/<tag>/`; `file://` works).

### `help` — explain jlock or one command

```
jlock help                 # what jlock is, the commands grouped by what they are for
jlock help acquire         # one command: every option with its default and an example,
jlock acquire --help       #   complete examples, its exit codes, related commands
```

Every option a command accepts has a line in its help page: the parser and the help are made from the
same table, and an option that is not in it is refused with exit 2 (with the closest name, if it looks
like a typo). An unknown command does the same. Help pages are wrapped at 100 columns.

### `about` — who made it

```
jlock about [--json]
```

The version, the author (Rodrigo Jäger, <https://github.com/rodrigojager>,
<https://www.linkedin.com/in/rodrigojager>), the repository, the license and the links to this file,
the README and the roadmap. It does not use the network.

### `--version`, `--help`, `--lang`

`jlock --version` prints one line, `jlock 0.1.0`, stable for scripts. `--help` (or `-h`) goes with any
command and shows its help page. `--lang <tag>` can go with any command and sets the language of its
text output (see below).

### Tips

After some results a short tip on the next step goes to standard error (standard output keeps one line
per key): after `busy` and `timeout` how to see who holds the key, after `not_owner` whose lock it is,
after `lost` what to do. They never appear with `--json`; `JLOCK_HINTS=0` turns them off.

## Language

Text output (help, result lines, errors, table headers) comes in the language chosen by, in order:
the `--lang <tag>` option (anywhere on the line, also as `--lang=<tag>`; never after the `--` of `run`),
the `JLOCK_LANG` variable, the system's language (`LC_ALL`, `LC_MESSAGES`, `LANG` on Unix;
the user's locale on Windows), and finally English.

```
jlock --lang pt-BR status name:x
livre name:x
```

| Tag | Language | | Tag | Language |
|---|---|---|---|---|
| `en` | English | | `zh-CN` | 简体中文 |
| `pt-BR` | Português (Brasil) | | `ja` | 日本語 |
| `es` | Español | | `ru` | Русский |
| `fr` | Français | | `ar` | العربية |
| `de` | Deutsch | | `hi` | हिन्दी |
| `it` | Italiano | | | |

A tag is matched exactly first, then by its main language (`pt`, `pt_PT.UTF-8` → `pt-BR`; `es-MX` →
`es`; `zh-TW` → `zh-CN`); an unknown tag prints a warning and the run goes on in English. To make a
language the default for your shell, set `JLOCK_LANG` (for example `export JLOCK_LANG=ja`, or
`$env:JLOCK_LANG = "ja"` in PowerShell). The Chinese, Japanese, Russian, Arabic and Hindi texts still
await a review by a native speaker; corrections are welcome, see [`CONTRIBUTING.md`](../CONTRIBUTING.md).

What never changes with the language: command and option names, every `--json` output, `log.jsonl`,
`index.json` and the exit codes. Scripts should read `--json`; the text is for people. The OS message
inside a file system error (exit 20) comes from the system, in the system's language.

## Exit codes

| Code | Meaning |
|---|---|
| 0 | granted / ok |
| 10 | busy: not taken with `--no-wait` |
| 11 | timeout waiting |
| 12 | deadlock |
| 13 | not the owner (`release`, `heartbeat`) |
| 14 | lock lost (`hold`, `run`) |
| 2 | wrong usage |
| 20 | file system error (permission, disk full, file held by another program); the OS message is shown |
| 130 | interrupted (CTRL+C) while waiting; the ticket was removed |

`run` passes the command's own exit code through once the lock was granted.

## Environment

| Variable | Default | Use |
|---|---|---|
| `JLOCK_HOME` | `%LOCALAPPDATA%\jlock`, `$XDG_STATE_HOME/jlock` or `~/.local/state/jlock`, `~/Library/Application Support/jlock` | where the registry lives |
| `JLOCK_HEARTBEAT_MS` | 5000 | how often `hold`/`run` and waiters renew |
| `JLOCK_STALE_MS` | 30000 | no renewal for this long = dead |
| `JLOCK_POLL_MS` | 250 | how often a waiter looks (±40% jitter) |
| `JLOCK_SWEEP_MS` | 60000 | minimum time between global sweeps |
| `JLOCK_LOG_MAX_BYTES` | 5242880 | log rotation size |
| `JLOCK_LANG` | system language, else `en` | language of the text output (see [Language](#language)) |
| `JLOCK_HINTS` | `1` | `0` turns off the tips printed after some results |
| `JLOCK_UPDATE_BASE` | the project on GitHub | where `update` looks for releases (`file://` works; the tests use a folder) |
| `JLOCK_DEBUG` | unset | any value: trace to stderr |

## On disk

```
<JLOCK_HOME>/
├─ locks/<sha1 of the key>/
│  ├─ key            the key, as text
│  ├─ holder.json    exists = taken; content = who; mtime = heartbeat
│  ├─ queue/<µs>-<rand>.json   one ticket per waiter, in arrival order
│  └─ broken/        only a rename target while breaking a dead lock; normally absent
├─ index.json        derived summary, rewritten after changes; never trusted
├─ last-sweep        time of the last global sweep
└─ log.jsonl         acquire, release, queued, timeout, expired, forced, deadlock, lost, gc
                     (rotated to log.1.jsonl at JLOCK_LOG_MAX_BYTES)
```

With nothing locked, `locks/` is empty. The locks are advisory: only programs that use `jlock` respect
them.

## From a script: `run`

```
jlock run D:\proj\dist -- npm run build           # one build at a time on this machine
jlock run name:port-3000 --timeout 2m -- ./serve  # give up after 2 minutes (exit 11)
```

## From an agent: `acquire` + `hold` + `release`

1. Before editing, take the files or folders, with an identity that survives across commands:
   `jlock acquire src/parser.rs src/lexer.rs --owner agent-7 --group task-42 --intent "T4 parser" --json`
   (`queued`/`timeout` mean someone else has them: wait, or pick other work).
2. Keep them alive while the task runs: start `jlock hold src/parser.rs src/lexer.rs --owner agent-7`
   in the background (it is granted at once, since you already hold them), or call
   `jlock heartbeat <key> --owner agent-7` more often than `JLOCK_STALE_MS`.
3. When done: send `release` to the `hold` process's standard input, or
   `jlock release --group task-42 --owner agent-7`.
4. If `hold` exits 14 the lock was taken away (forced or expired): stop editing.
