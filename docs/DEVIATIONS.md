# Deviations from `docs/PLAN.md`

**Open deviations: 8 and later** (phase 2, to be written into the plan as revision 5). Entries 1–7 were written into the plan on 24/10/2026 (revision 4, the
paragraphs marked **[rev. 4]**), so the plan is the source of truth again. The entries stay as the record of
what was decided during implementation and why.

## 1. `busy` vs `queued` in the `--no-wait` answer — now in plan §6, `acquire` row

- **What:** the tickets call the `--no-wait` refusal `busy` (exit 10); the plan's `--json` result list has
  `queued` and no `busy`.
- **Done instead:** the plan wins for the machine-readable output: `--json` prints `"result": "queued"` with
  `holder` and `position`. The human text line says `busy <key> held by <owner> (position N)`. Exit code 10.

## 2. Keys that do not look like paths — now in plan §5

- **What:** section 5 defines path keys as inputs containing `/`, `\`, or starting with a drive letter, `.`
  or `~`, and `name:` keys; it does not say what a bare word such as `README.md` is.
- **Done instead:** anything that is not `*` and does not start with `name:` is a path, resolved from the
  current directory (`jlock acquire a.ts` locks `<cwd>/a.ts`).

## 3. The same owner asking again — now in plan §7.1 step 0

- **What:** the plan does not say what happens when an owner asks for a key it already holds.
- **Done instead:** it is granted at once and `holder.json` is left as is (no counting). Holders and
  tickets of the same owner never block that owner, including through the path hierarchy.

## 4. Exit code for an interrupted wait — now in plan §6, exit codes

- **What:** the plan says CTRL+C during the wait removes the ticket, but its exit-code table has no entry
  for "interrupted".
- **Done instead:** the ticket is removed and the command exits with 130 (the shell convention for SIGINT),
  `--json` result `error` with `"error": "interrupted"`.

## 5. Waiting on conflicting keys: one ticket, ordered across queues — now in plan §7.1 step 1

- **What:** section 7.1 step 1 says a path request waits "with a ticket in each conflicting key".
- **Why not:** tickets in several queues have to be created, renewed and removed together, and a crash
  in between leaves partial state in keys the caller never asked for.
- **Done instead:** the request has one ticket, in its own key. On each poll it also reads the queues and
  holders of every conflicting key (folder above, files inside, `*`) and only goes ahead when no other
  owner holds a conflicting key and no other owner has an older ticket in any conflicting queue. Ticket
  names are global arrival times, so this is the same arrival order. Two conflicting keys taken at the
  same instant by requests that could not see each other are caught right after the `create_new`: each
  side re-reads the conflicting holders, and one that finds a clash withdraws its `holder.json` and keeps
  waiting.

## 6. `index.json` rename: short retries only — now in plan §7, "Escritas"

- **What:** section 7.5 asks for retries of 50 → 1600 ms on `EBUSY`/`EPERM`/`EACCES` for renames.
- **Why not for the index:** every command that changes something rewrites `index.json`; under load the
  rename over it collides with the other writers, and waiting up to 3 s for a file that is only derived
  would slow every acquire and release.
- **Done instead:** the index rename is tried 4 times 10 ms apart, then given up (the `.tmp` is removed);
  the next change or the global sweep writes it again. It is skipped when its content would not change.
  Lock files, tickets, `broken/` and the log rotation keep the full retry ladder.

## 7. A late heartbeat no longer breaks a live `hold`/`run` — now in plan §7 "Trava do SO", §9, §10

- **What:** plan sections 9 and 10 say an owner frozen for more than `STALE_MS` while alive has its lock
  broken and gets `lost`. Ticket 09 adds the OS lock and asks that a live `hold`/`run` (OS lock still
  taken) is **not** broken when its heartbeat is late.
- **Done instead:** as ticket 09 says. Before breaking a holder for a stale heartbeat, the expirer tries the
  OS lock on `holder.json`; if it is taken, the owner is alive and the lock stays. A `hold`/`run` that is
  alive but stuck therefore keeps its keys until its `--ttl` runs out or someone uses `unlock --force`.
  Locks taken with plain `acquire` (no OS lock) expire by heartbeat exactly as in the plan.

## 8. Tips go to standard error (ticket 12)

- **What:** ticket 12 asks for tips "in the text outputs" without saying which stream.
- **Why:** scripts that read the text (`out=$(jlock acquire ...)`) expect one line per key on standard
  output; an extra `tip:` line there would break them unless every script set `JLOCK_HINTS=0`.
- **Done instead:** tips are printed to standard error, only in text mode (never with `--json`), and
  `JLOCK_HINTS=0` turns them off. Standard output keeps exactly the lines it had before.

## 9. The command table lives in `src/commands.rs` (ticket 12)

- **What:** ticket 12 places the single table of commands and options in `src/help.rs`.
- **Why:** the test that checks "every option the parser takes has a line of help" has to read that table,
  and integration tests can only include a source file that does not depend on the rest of the crate;
  `help.rs` uses the catalogs, the CLI types and `about.rs`.
- **Done instead:** the table is pure data in `src/commands.rs` (included by `tests/help.rs` with
  `#[path]`); `src/help.rs` renders it and checks the options against it. Still one table, used by both
  the help and the dispatcher.

## 10. `x86_64-apple-darwin` is built on an arm64 macOS runner (ticket 14)

- **What:** ticket 14 builds `x86_64-apple-darwin` on `macos-13`, the Intel runner.
- **Why not:** GitHub retired the `macos-13` runners; a workflow that names them waits for a runner that
  never comes.
- **Done instead:** the Intel binary is cross-built on `macos-14` (Apple's toolchain targets both
  architectures). The test suite still runs on that runner, for arm64; the Intel binary is not tested
  natively before it is published.

## 11. SHA-256 written in `src/sha256.rs`, not the `sha2` crate (ticket 15)

- **What:** ticket 15 lets `update` check the download with its own SHA-256 or with `sha2`, and prefers
  `sha2`.
- **Why not `sha2`:** the phase-2 rules allow no new dependency besides `windows-sys` features; `sha2` is
  not already in the tree (`sha1` does not pull it in).
- **Done instead:** a FIPS 180-4 implementation of about 80 lines, checked against the standard test
  vectors (empty, `abc`, the 448-bit message, one million `a`).

## 12. `update` unpacks with the system's `tar` (ticket 15)

- **What:** the archives are `.zip` (Windows) and `.tar.gz` (the rest), and the ticket does not say how
  `update` takes the binary out of them; jlock has no archive code.
- **Done instead:** the system's `tar` does it: `%SystemRoot%\System32\tar.exe` on Windows 10 and later
  (bsdtar, which reads zip files), `tar` from `PATH` on Unix. On a Windows without it, PowerShell's .NET
  `ZipFile`. No new dependency, as for `curl`.

## 13. `JLOCK_FAKE_VERSION` is read by debug builds (ticket 15)

- **What:** the update test needs the installed "new" binary to answer `--version` with the fake
  release's version.
- **Done instead:** `--version` prints `JLOCK_FAKE_VERSION` when it is set **and** the binary is a debug
  build (the one `cargo test` runs). Release builds ignore it, so the version a user sees is always the
  real one.
