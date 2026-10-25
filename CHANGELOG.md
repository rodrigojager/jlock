# Changelog

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
