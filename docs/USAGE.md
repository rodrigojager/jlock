# jlock command reference

Every command, option, exit code, variable and file of jlock. Why jlock exists, how to install it and a
first tour are in the [README](../README.md) (in eleven languages); `jlock help <command>` prints the
same reference in the terminal, in your language. The design is in [`PLAN.md`](PLAN.md) (Portuguese);
the decisions taken while building it are in [`DEVIATIONS.md`](DEVIATIONS.md).

## Keys

| Key | Meaning | Conflicts with |
|---|---|---|
| a path: `D:\proj\src\a.ts`, `src/a.ts`, `~/notes`, `.` | the file or folder, resolved to an absolute path; `\` becomes `/`; lower case on Windows and macOS; no trailing `/`. The path does not have to exist. | the same path, every path inside it (if it is a folder), every folder above it, and `*` |
| `name:<text>` | anything else you name yourself: `name:deploy-prod`, `name:nightly`; free text, no check | the same name, and `*` |
| `resource:<name>[:<arg>]` | a registered resource (see [Resources](#resources)): `resource:desktop`, `resource:port:8080`. The name is folded to lower case; an unknown name is refused | the same resource with the same argument, and `*` |
| `*` | everything; kept for emergencies (to ask for a quiet machine, take `resource:machine` exclusive) | every key |

`D:\Proj\SRC\`, `d:/proj/src` and `src` (run from `D:\proj`) are the same key. A `resource:` key never
conflicts with a path or a `name:` key, even one with the same text (`name:desktop`, `./desktop`).

### Modes

Every key of `acquire`, `hold` and `run` is taken in a mode: `exclusive` (the default) or `shared`.
`--mode shared|exclusive` sets it for every key of the request; a key can carry its own with a suffix,
`src/a.ts@shared` or `resource:desktop@exclusive` (only these two exact words count, so any other `@`
in a path stays part of it). Order of precedence: the key's suffix, then `--mode`, then the resource's
own default (see [Resources](#resources)), then `exclusive`. A key named twice in one request is taken
once, in the stronger mode.

- Shared holds of conflicting keys coexist; an exclusive hold conflicts with any other hold of a
  conflicting key. The path hierarchy applies across modes: a shared hold on `src` and an exclusive one
  on `src/a.ts` conflict; two shared ones do not.
- The queue stays fair: a request waits behind any older ticket that conflicts with it, so a shared
  request arriving after an exclusive waiter queues behind it (no writer starvation), while shared
  waiters behind the same holder are granted together.
- Asking again for a key you hold in the same mode, or holding it exclusive and asking shared, is
  granted at once. Holding it shared and asking exclusive is an upgrade: it waits like any exclusive
  request (your own shared hold does not block it) and adds an exclusive hold; `release` gives back both.

### Owners

An owner (`--owner`) is a path of names separated by `/`: `agent-7`, `gt:run-1/T04`,
`gt:run-1/T04/exec-2`. `gt:run-1/T04` is an ancestor of `gt:run-1/T04/exec-2`; `gt:run-1/T05` is a
sibling, unrelated. Empty parts and a `/` at either end are refused (exit 2).

- **One family never blocks itself.** An owner, its ancestors and its descendants never conflict, in
  any mode, and never wait behind each other's tickets.
- **Nested holds.** A request covered by a hold of an ancestor (that hold's key is the same key, a
  folder above it, or `*`) is granted at once, without queueing, as a *nested* hold: it records the
  ancestor's hold (`parent`: owner, key, grant) and is shown as nested by `status` and `list`. The
  nearest ancestor is the parent. A covered request still waits for an unrelated hold that conflicts
  with it (the ancestor holds a folder shared, a stranger holds it shared too, the child asks for a file
  inside exclusive).
- **A nested hold lives no longer than its parent.** When the parent hold ends (release, expiry, death
  of its process, `unlock --force`), its nested holds end too, and theirs in turn (logged as `expired`
  with reason `parent`; a `hold` keeping one prints `lost` and exits 14). A child's release never touches
  the parent's hold.
- **An ancestor with only descendants in the way** is granted: a descendant holds `src/a.ts`, the
  ancestor asks for `src` and gets it, unless an unrelated owner also holds something inside.
- **Deadlocks.** An owner waits whenever one of its descendants waits (a task waits for the commands it
  started). The waits-for graph uses that: walking from an owner also follows its descendants' edges,
  and a chain that comes back to an ancestor of the waiter is a cycle. Example: P holds `x`, its child
  `P/exec` waits for `y` held by S; S asking for `x` is refused with
  `S → x (held by P) → P ⊃ P/exec → y (held by S) → S`.

### Resources

A `resource:` key names something that is not a file. Its name must be in the catalogue: the built-in
entries, plus your own in `<JLOCK_HOME>/resources.json`. An unknown name is refused (exit 2) with the
closest one as a suggestion (`unknown resource 'desktp'; did you mean resource:desktop?`), so a typo
never silently becomes another lock. `acquire`, `hold`, `run` and `status` check the name; `release`,
`unlock`, `renew` and `heartbeat` do not (a lock must always be releasable). `jlock resources` lists the
catalogue.

| Resource | What it stands for | Default mode |
|---|---|---|
| `resource:audio` | the speakers and the microphone | exclusive |
| `resource:browser` | the user's own browser profile and its logged-in sessions | exclusive |
| `resource:clipboard` | the system clipboard | exclusive |
| `resource:desktop` | the real mouse, keyboard and windows of the desktop session | exclusive |
| `resource:gpu` | the graphics card | exclusive |
| `resource:machine` | the whole machine: exclusive asks for it quiet (benchmarks, timing tests); shared says a job loads it | exclusive |
| `resource:network` | the network link (big transfers, network-sensitive tests) | exclusive |
| `resource:port:<n>` | a TCP or UDP port of this machine, 1 to 65535 (`resource:port:08080` is `resource:port:8080`) | exclusive |

`resource:machine` is the way to ask for a quiet machine: jobs that load it (builds, test suites) take it
shared, a benchmark takes it exclusive and waits for them; locks of other keys are not affected. `*`
also stops every lock of every project, so it is kept for emergencies.

Your own resources go in `resources.json` in the registry folder (`JLOCK_HOME`):

```json
{ "resources": {
    "db":      { "description": "the local test database", "mode": "shared" },
    "printer": { "description": { "en": "the office printer", "pt-BR": "a impressora do escritório" } },
    "vm":      { "description": "a virtual machine, by name", "arg": "name" } } }
```

- The name: lower-case letters, digits, `-`, `_`, `.`; it cannot redefine a built-in one.
- `description`: one text, or one per language tag (the run's language, then its main language, then
  `en`, then any).
- `mode`: `shared` or `exclusive` (default), taken when the request names none.
- `arg`: present when the resource takes an argument (`resource:vm:web-1`); its value names the argument
  in the listing (`vm:<name>`). A resource with an argument refuses a key without one, and the other way
  round.

A file that does not parse, or a bad entry, is reported on standard error and left out; the rest still
works.

## Commands

Every command accepts `--json` and then prints one JSON object with a `result` field:
`granted`, `queued`, `timeout`, `deadlock`, `released`, `not_owner`, `lost`, `ok` or `error`.
Without `--json` it prints one short line per key.

### `acquire` — take keys and keep them

```
jlock acquire <key[@shared|@exclusive]...> [--owner id] [--group g] [--intent "..."] [--meta k=v]...
              [--ttl 2h] [--mode shared|exclusive] [--wait | --no-wait] [--timeout 30s] [--json]
```

```
jlock acquire D:\proj\src\a.ts --owner agent-7 --group run-42 --intent "refactor parser"
granted d:/proj/src/a.ts
jlock acquire name:db --mode shared --owner reader-1
granted name:db (shared)
jlock acquire D:\proj\src\a.ts --owner agent-7/build
granted d:/proj/src/a.ts (nested in agent-7)
```

- `--mode` and the `@` suffix: see [Modes](#modes). An owner path and nested holds: see [Owners](#owners).

- Waits its turn by default (no limit). `--timeout 30s` gives up after 30 s (exit 11);
  `--no-wait` never waits (exit 10, with the holder and the position it would have had).
- Several keys are taken all or none, in a fixed (sorted) order, so two callers asking for
  `{a,b}` and `{b,a}` never block each other.
- `--owner` defaults to `pid:<pid>`; whoever presents the same owner can release. Asking again for a
  key you already hold is granted at once.
- `--ttl` is a hard limit on how long the lock may be held, even with a heartbeat. `jlock renew`
  pushes it back for work that takes longer.
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
If the lock is taken away (`unlock --force`, expiration, the end of the hold it was nested in) it prints
`lost <key>` and exits 14. While held, its hold files also carry an operating-system lock
(`LockFileEx` / `flock`), so the kernel frees them the instant the process dies.

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

`--release-on-eof` is for the other kind of caller: a program that stays open (an editor plugin, a
server) and runs `hold` in the foreground, holding its standard input. The end of that input counts as the
line `release`: if the program dies, the pipe closes and the lock is released at once, even on Windows,
where a dead parent does not end its children. It does not go with `--detach` (the background process has
no standard input): short calls use `--detach`, a host that stays open uses `--release-on-eof`.

### `release` — give keys back

```
jlock release D:\proj\src\a.ts --owner agent-7
jlock release --owner agent-7                 # everything agent-7 holds
jlock release --group run-42 --owner agent-7  # agent-7's locks in the group
jlock release --group run-42 --any-owner      # the whole group
```

Only the owner can release a key (exit 13 otherwise); every hold the owner has on it goes (an upgraded
key has two). The holds nested in the ones released end with them. The key's folder disappears once
nobody holds or waits for it.

### `heartbeat` — renew a lock taken with `acquire`

```
jlock heartbeat D:\proj\src\a.ts --owner agent-7
renewed d:/proj/src/a.ts
```

Exit 13 with `lost` if the lock is not yours any more.

### `renew` — push back the deadline of locks you hold

```
jlock renew [<key...>] [--owner id] [--group g] [--ttl 2h] [--json]
jlock renew --owner agent-7 --ttl 2h
renewed d:/proj/src/a.ts until 2026-10-25T23:40:00.000Z
```

Moves the deadline (`--ttl`) of the owner's locks to now plus the ttl and renews their heartbeat.
Without `--ttl` it reuses the ttl the lock was taken with; without keys it renews every lock of the owner
(only those of the group, with `--group`). The lock is never let go while it is renewed: the hold file is
not rewritten (the new deadline goes to `holds/<grant>.lease`), so a `hold`/`run` keeping it, and its OS
lock, go on as before. A key that is not yours is
exit 13; a lock already past its deadline expires right there (exit 14, `lost`) and has to be taken again.
A lock without a ttl only gets its heartbeat. `status` shows how long is left (`expires in 1h59m`) and
`status --json`/`list --json` carry `expires`.

### `status` — one key

```
jlock status D:\proj\src\a.ts
free d:/proj/src/a.ts
conflicts:
  d:/proj/src held by agent-3 (exclusive), 1 waiting
  why: a lock on a folder covers everything inside it, so a held folder above this key blocks it
jlock status name:db
held name:db
  by reader-1 (shared) since 2026-11-08T10:00:00.000Z (2m10s ago), heartbeat 1s ago
  by reader-1/job (exclusive) since 2026-11-08T10:01:00.000Z (1m10s ago), nested in the hold of reader-1 on name:db
queue:
  1. writer (exclusive, waiting 40s)
```

Shows every hold of the key (owner, mode, the hold it is nested in, since, heartbeat, deadline, group,
intent), the queue with positions and the mode each waiter asks for, and the conflicting keys that would
block it: a folder above, files inside, or `*`. A `resource:` key is checked against the catalogue.

### `list` — everything locked on this machine

```
jlock list [--owner x] [--group g] [--under D:\proj] [--stale] [--json] [--watch]
KEY             MODE       OWNER                GROUP   INTENT    HELD   HEARTBEAT  QUEUE
d:/proj/src     exclusive  agent-3              run-42  refactor  4m10s  2s         1
d:/proj/src     exclusive  agent-3/b (nested)   run-42  build     12s    1s
name:db         shared     reader-1             -       -         15s    1s         0
name:db         shared     reader-2             -       -         9s     1s

4 locks, 1 waiting
(nested) a hold inside a hold of the owner's ancestor: it ends when that hold ends
```

One line per hold: a key held by several owners (shared holds, a family) has several lines, and its
queue length is on the first.

Read straight from the folder, so it is always the truth. `--under` keeps paths at or below a folder;
`--stale` only dead locks (heartbeat older than `JLOCK_STALE_MS` with nobody holding the OS lock, or past
their ttl); `--watch` redraws every second until CTRL+C. A `hold`/`run` whose heartbeat is late but still
has the OS lock is alive: it is shown as `late`, not `dead`, and the expiration leaves it alone. The
footer counts locks and waiters, and explains `(dead)` and `(late)` when they appear;
`jlock help list` explains every column.

### `graph` — who waits for whom

```
jlock graph
agent-7 -> agent-3  (d:/proj/src held exclusive by agent-3; wants exclusive)
writer -> reader-1  (name:db held shared by reader-1; wants exclusive)
no cycles
```

The edges come from the same rule that grants: shared holds that do not exclude the waiter, and the
waiter's own family, make no edge. A request that would close a cycle is refused before it queues, with
the chain (exit 12): `B → name:x (held by A) → A → name:y (held by B) → B`. A step taken through a
descendant's wait shows as `P ⊃ P/exec` (see [Owners](#owners)). If two requests close a cycle at the
same instant, the newer one gives up.

### `resources` — the catalogue of `resource:` keys

```
jlock resources [--json]
Built-in resources (take them as resource:<name>):
  audio      exclusive  the speakers and the microphone (playing or recording sound)
  ...
  port:<n>   exclusive  a TCP or UDP port of this machine, by number (resource:port:8080)

No resources of your own; define them in C:\Users\me\AppData\Local\jlock\resources.json.
```

`--json` prints `{ "result": "ok", "file": "<path of resources.json>", "resources": [{ "name", "key",
"arg", "mode", "source": "builtin" | "user", "description" }], "warnings": [...] }`.

### `unlock --force` — break someone else's lock

```
jlock unlock D:\proj\src --force --reason "agent hung"
jlock unlock --owner agent-3 --force
jlock unlock --group run-42 --force --by ci
```

Writes `forced` to the log with `by` (default `user:<login>`) and `reason`. A key named loses every
hold on it; `--owner`/`--group` break those holds wherever they are. The holds nested in a broken one
end with it. The owner's `hold`/`run` notices on its next heartbeat and stops with 14. Without `--force`
it refuses (exit 2).

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
ok    version          jlock 0.3.0, running from C:\Users\me\.cargo\bin\jlock.exe
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

`jlock --version` prints one line, `jlock 0.3.0`, stable for scripts. `--help` (or `-h`) goes with any
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
| 13 | not the owner (`release`, `heartbeat`, `renew`) |
| 14 | lock lost (`hold`, `run`), or past its deadline (`renew`) |
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
│  ├─ gate           exists for an instant while one request for this key is decided (create_new);
│  │                 one left by a dead process is broken after JLOCK_STALE_MS
│  ├─ holds/<grant>.json    one per hold: who, mode, parent (nested); mtime = heartbeat;
│  │                        <grant> = <µs>-<5 digits>, in grant order
│  ├─ holds/<grant>.lease   the deadline set by `renew` (owner, since of the grant, until)
│  ├─ queue/<µs>-<5 digits>.json   one ticket per waiter (with the mode asked for), in arrival order
│  └─ broken/        only a rename target while breaking a dead lock; normally absent
├─ resources.json    your own resource: names (optional, see Resources)
├─ index.json        derived summary, rewritten after changes; never trusted
├─ last-sweep        time of the last global sweep
└─ log.jsonl         acquire, release, renew, queued, timeout, expired, forced, deadlock, lost, gc
                     (rotated to log.1.jsonl at JLOCK_LOG_MAX_BYTES)
```

The layout changed in 0.3 (0.2 had one `holder.json` per key); a 0.3 jlock does not read locks taken by
0.2, so let every 0.2 lock go before upgrading.

With nothing locked, `locks/` is empty. The locks are advisory: only programs that use `jlock` respect
them.

## From a script: `run`

```
jlock run D:\proj\dist -- npm run build           # one build at a time on this machine
jlock run name:port-3000 --timeout 2m -- ./serve  # give up after 2 minutes (exit 11)
```

## From an agent: `hold --detach` + `release`

1. Before editing, take every file or folder of the task in one call, with an identity that survives
   across commands:
   `jlock hold src/parser.rs src/lexer.rs --owner agent-7 --group task-42 --intent "T4 parser" --ttl 2h --timeout 2m --detach`.
   It answers (`granted`, or `timeout`/exit 11 if someone else kept them past `--timeout`; with
   `--no-wait`, `busy`/exit 10 at once) and returns; a background `jlock hold` keeps the lock alive.
2. After a long pause, `jlock heartbeat <key> --owner agent-7` says whether the lock is still yours
   (exit 13 `lost` if it was broken or expired). A task that may outlast its `--ttl` runs
   `jlock renew --owner agent-7` before the deadline: same length again, the lock never let go.
3. When done (or when the task fails): `jlock release --group task-42 --owner agent-7`. The background
   process notices and ends.

Plain `acquire` is not enough for an agent: its lock expires after `JLOCK_STALE_MS` (30 s) without a
`heartbeat`, and an agent spends minutes between two commands. A command the agent starts on the task's
files runs under a child owner (`jlock run src/parser.rs --owner agent-7/task-42/test -- cargo test`):
it is granted at once, nested in the task's hold, while other agents keep waiting. The skills in
[`skills/`](../skills/README.md) teach Claude Code, OpenCode and Codex this workflow.

## Foreground process custody (0.4)

`jlock exec [--timeout <duration>] -- <argv...>` starts a supervised foreground command without a lock.
`jlock run <keys...> [--command-timeout <duration>] -- <argv...>` retains ownership until cleanup.
`run --timeout` still bounds lock acquisition only. Command timeout returns 124; interruption returns 130;
loss of ownership returns 14. Normal exit preserves the root command's exit status after cleanup.

Windows uses a private kill-on-close Job Object assigned before the child resumes, including batch launchers.
Unix uses a private process group and signals its ordinary descendants before reaping the leader. Deliberate
detachment, hard supervisor death on Unix, remote jobs and Docker daemon workloads require their own lifecycle.
Persistent background services must not be launched under these foreground commands. Cleanup failure reports
`cleanup pending`; `run` retains and renews its locks while retrying. Do not force-release those locks by age.

There is no registry migration from 0.3. Install alongside any active binary and select it for future sessions.
