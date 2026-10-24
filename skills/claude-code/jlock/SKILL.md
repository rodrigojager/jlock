---
name: jlock
description: Take turns with other agents, scripts and people on this machine by locking files, folders or named resources with the jlock command line tool. Use it before editing files that another agent may be editing at the same time, before running a build, test run, migration, deploy or dev server that must not run twice at once, and when asked who holds a file or why something is waiting. Not for coordinating different machines.
---

# jlock: locks for files, folders and names

`jlock` gives one owner at a time a lock on a key: a file, a folder (which covers everything inside it)
or a name such as `name:port-3000`. Others who ask wait in line, in order of arrival. Locks of processes
that died are recovered by themselves. The locks are advisory: they only protect against programs that
also use jlock, so take them every time, even when you think you are alone.

## Before the first use

- `jlock --version` must answer. If the command is missing, tell the user and point to the install
  commands: `curl -fsSL https://raw.githubusercontent.com/rodrigojager/jlock/main/install.sh | sh`
  (Linux, macOS) or `irm https://raw.githubusercontent.com/rodrigojager/jlock/main/install.ps1 | iex`
  (Windows).
- If a jlock command fails with exit code 20, run `jlock doctor`: it says what is wrong and how to fix it.
- Pick your identity once and reuse it in every command of the session:
  - `--owner claude:<id>`, where `<id>` is a short random word you choose now (for example
    `claude:k7f2`). The same owner can take, renew and release; a different one cannot.
  - `--group <task>` names the task you are working on (`--group fix-login`), so everything it holds can
    be given back at once.
  - `--intent "<what you are doing>"` is shown to others who find the file taken.

## Editing files that others may edit

1. Take everything the task will touch, in one call, before the first edit:

   ```
   jlock hold src/auth.rs src/session.rs --owner claude:k7f2 --group fix-login --intent "fix login timeout" --ttl 2h --timeout 2m --detach
   ```

   - It takes all the keys or none (in a fixed order, so it cannot deadlock with another agent asking
     for the same files in another order).
   - `--detach` makes it answer and return while a small background process keeps the lock alive. Do
     not run `hold` without `--detach` from a tool call: it would never return.
   - `--timeout 2m` waits at most two minutes for others to finish; `--no-wait` does not wait at all.
   - `--ttl 2h` is a safety limit: the lock goes away by itself after two hours even if you never
     release it.
   - A folder key covers every file inside it; lock the folder when the task changes many files there.

2. Read the exit code:

   | Code | Meaning | What to do |
   |---|---|---|
   | 0 | granted: the keys are yours | edit |
   | 10 | busy (with `--no-wait`) | see who has it, then wait, do other work, or ask the user |
   | 11 | timeout: still taken after `--timeout` | same as 10 |
   | 12 | deadlock: waiting would close a cycle | release what you hold, then take everything in one call |
   | 2 | wrong usage | fix the command (`jlock help hold`) |
   | 20 | file system error | `jlock doctor` |

3. Edit. If you need one more file later, take it with another `hold ... --detach` with the same
   `--owner` and `--group`.

4. Give everything back when the task is done, and also when it fails or is abandoned:

   ```
   jlock release --group fix-login --owner claude:k7f2
   ```

## When a key is taken

- `jlock status src/auth.rs` shows who holds it, since when, its intent, the queue, and any folder above
  it or file inside it that blocks it.
- `jlock list` shows everything locked on the machine; `jlock list --owner claude:k7f2` shows what you
  hold; `jlock graph` shows who waits for whom.
- Never break someone else's lock on your own. `jlock unlock <key> --force --reason "..."` exists for
  emergencies, and only when the user asks for it.

## After a long pause, before trusting a lock

If the lock was broken (`unlock --force`) or expired, it is no longer yours. Check before you write:

```
jlock heartbeat src/auth.rs --owner claude:k7f2
```

Exit code 0: still yours. Exit code 13 (`lost`): stop changing the file, look at `jlock status`, and take
it again if the task must go on.

## Running a command that must not run twice

`jlock run` takes the keys, runs the command, keeps the lock while it runs, releases it at the end and
exits with the command's own exit code:

```
jlock run name:build --owner claude:k7f2 --timeout 10m -- cargo build --release
jlock run name:port-3000 --owner claude:k7f2 --no-wait -- npm run dev
jlock run name:db-migrations --owner claude:k7f2 -- ./migrate.sh
```

Everything after `--` belongs to the command. Use a `name:` key for anything that is not a file: ports,
databases, deploy targets, a shared test environment. Everyone has to use the same name.

## Keys

| Key | Means | Conflicts with |
|---|---|---|
| a path: `src/auth.rs`, `D:\proj\dist`, `.` | that file or folder; relative to the current folder; it does not have to exist | the same path, everything inside it, every folder above it, and `*` |
| `name:<text>` | anything that is not a file | the same name and `*` |
| `*` | everything | every key |

## Reading the answers

- Add `--json` to any command to get one JSON object with a `result` field (`granted`, `queued`,
  `timeout`, `deadlock`, `released`, `not_owner`, `lost`, `ok`, `error`). The JSON never changes with the
  language; prefer it when you parse answers.
- The text output follows the user's language. `--lang en` forces English.
- `jlock help <command>` explains every option of a command, with examples and exit codes.

## Do not

- Do not use `jlock acquire` for work that lasts more than a few seconds: a lock taken with `acquire` must
  be renewed with `jlock heartbeat` every 30 seconds or it expires. Use `hold --detach` instead.
- Do not change `--owner` between commands of the same session: `release` and `heartbeat` only work for
  the owner that took the lock.
- Do not leave locks behind: release at the end of every task, including failed ones.
