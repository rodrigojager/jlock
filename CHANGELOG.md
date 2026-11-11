# Changelog

## 0.4.3

- Finish normal commands on macOS when the private group contains only zombies: Darwin returns EPERM for that group. Verify complete group snapshots and zombie states before accepting it; live or uninspectable members still retain ownership.
- Keep the unreaped leader pinned until cleanup, and recheck group membership after state inspection to reject a concurrent fork.
- Add a bounded normal-exit regression alongside descendant, timeout, lock-loss and unrelated-process coverage.
- Publish immutable tags and use supported macOS 15 runners for every native check.

## 0.4.2

- Add `run --total-timeout` to bound queue plus payload with one custodian and a single budget.
- Avoid nesting supervisors for serialized deadlines: on Unix an inner private process group would escape an outer hard timeout.

## 0.4.1

- Use supported macOS 15 runners for CI and release archives; macOS 14 jobs are no longer accepted.
- Includes the foreground custody protections from 0.4.0, whose archive publication was blocked by the runner configuration.

## 0.4.0 — 2026-11-11

- Add `exec --timeout` for process custody without registry locks; add `run --command-timeout`.
- Windows uses suspended launch and a private kill-on-close Job Object. Stop descendants before unlocking,
  including on lock loss, deadline and successful wrapper exit. Refuse unsafe containment fallback.
- Unix commands use private process groups; document detachment and hard-parent-death limits.
- Preserve the 0.3 registry and owner/resource semantics; this release also ships the previously unreleased 0.3 redesign.
- Deterministic regressions cover inherited pipes, lock loss, timeout, supervisor death and unrelated siblings.

## 0.3.0 — unreleased

- **Owners are paths** (`gt:run/T18/exec-1`). An owner, its ancestors and its descendants never block
  each other. A key covered by an ancestor's hold (the key itself, a folder above it, or `*`) is granted
  at once as a *nested* hold, without queueing, and ends when that hold ends (release, ttl, death,
  `unlock --force`); an ancestor is granted a key whose only other holders are its descendants. A
  descendant's wait counts as its ancestors' wait in deadlock detection. Fixes a task deadlocking with
  the commands it starts. Owners with an empty part or a `/` at either end are refused.
- **Shared and exclusive modes**: `--mode shared|exclusive` on `acquire`, `hold`, `run`, or per key with
  `@shared`/`@exclusive`. Shared holds coexist; path hierarchy conflicts apply across modes; an
  exclusive waiter is never starved by later shared requests.
- **`resource:<name>[:<arg>]` keys** checked against a catalogue: built-in `audio`, `browser`,
  `clipboard`, `desktop`, `gpu`, `machine`, `network`, `port:<n>`, plus your own in
  `<JLOCK_HOME>/resources.json`. Unknown names are refused with a "did you mean". New command
  `jlock resources`. `resource:machine` exclusive is the way to ask for a quiet machine; `*` stays for
  emergencies.
- `status`, `list`, `graph`, `index.json`, the log and `--json` show modes, every hold of a key
  (`holders`) and nesting (`nested`, `parent`); `acquire --json` adds `locks`; busy/timeout add
  `holders` and `mode`. `list` has a MODE column and one line per hold. See `docs/API-0.3.md`.
- **New on-disk layout, not compatible with 0.2**: one file per hold (`holds/<grant>.json`, its lease in
  `holds/<grant>.lease`), decided under a short-lived `gate` file. Let every 0.2 lock go before
  upgrading. Ticket and grant names end in five digits.

## 0.2.0 — 2026-10-25

- New command `jlock renew [<key...>] [--owner id] [--group g] [--ttl 2h]`: pushes the deadline (`--ttl`) of
  locks you hold back to now plus the ttl (by default the ttl the lock was taken with) and renews their
  heartbeat. The lock is never let go: `holder.json` is not rewritten, so a `hold` or `run` keeping it, and its
  OS lock, go on as before. The new deadline lives in `lease.json` next to `holder.json` and only counts for the
  grant it names. Exit 13 for a key that is not yours, 14 for a lock already past its deadline.
- `status` shows how long a lock has left (`expires in 1h59m`); `status --json` and `list --json` carry
  `expires`.
- A `hold` started by an older jlock does not read the new deadline and ends at the old one.

## 0.1.1 — 2026-10-24

- `jlock hold --release-on-eof`: the lock ends the moment the program holding the pipe dies.

## 0.1.0 — 2026-10-24

- First release.
