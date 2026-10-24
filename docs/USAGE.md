# jlock usage

`jlock` gives out machine-wide locks on keys: files, folders, or plain names. One owner per key, a fair
queue in order of arrival, locks of dead processes recovered by heartbeat, deadlocks refused, a list of
everything locked and by whom, and `unlock --force` for emergencies. There is no server: the state is a
folder on disk, so nothing has to stay running and nothing is lost if any process crashes.

The design is in [`PLAN.md`](PLAN.md) (Portuguese); departures from it are in
[`DEVIATIONS.md`](DEVIATIONS.md).

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
on standard input, CTRL+C or SIGTERM. If the lock is taken away it prints `lost <key>` and exits 14.
While held, `holder.json` also carries an operating-system lock (`LockFileEx` / `flock`), so the
kernel frees it the instant the process dies.

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
```

Shows the holder (owner, since, heartbeat, group, intent), the queue with positions, and the
conflicting keys that would block it: a folder above, files inside, or `*`.

### `list` — everything locked on this machine

```
jlock list [--owner x] [--group g] [--under D:\proj] [--stale] [--json] [--watch]
KEY                  OWNER    GROUP   INTENT    HELD   HEARTBEAT  QUEUE
d:/proj/src          agent-3  run-42  refactor  4m10s  2s         1
name:port-3000       dev      -       -         15s    1s         0
```

Read straight from the folder, so it is always the truth. `--under` keeps paths at or below a folder;
`--stale` only dead locks (heartbeat older than `JLOCK_STALE_MS` with nobody holding the OS lock, or past
their ttl); `--watch` redraws every second until CTRL+C. A `hold`/`run` whose heartbeat is late but still
has the OS lock is alive: it is shown as `late`, not `dead`, and the expiration leaves it alone.

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

### `--version`, `--help`, `--lang`

`--lang <tag>` can go with any command and sets the language of its text output (see below).

## Language

Text output (help, result lines, errors, table headers) comes in the language chosen by, in order:
the `--lang <tag>` option (anywhere on the line, also as `--lang=<tag>`; never after the `--` of `run`),
the `JLOCK_LANG` variable, the system's language (`LC_ALL`, `LC_MESSAGES`, `LANG` on Unix;
the user's locale on Windows), and finally English.

```
jlock --lang pt-BR status name:x
livre name:x
```

Accepted tags: `en`, `pt-BR`. A tag is matched exactly first, then by its main language
(`pt`, `pt_PT.UTF-8` → `pt-BR`); an unknown tag prints a warning and the run goes on in English.

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
