# jlock

Machine-wide lock CLI for files, folders and named resources, with a fair queue, heartbeat-based recovery,
deadlock detection, a list of everything locked and by whom, and force unlock. No server: it survives any crash.
Written in Rust; works on Windows, Linux and macOS.

Status: planned, not implemented yet. See `docs/PLAN.md` (Portuguese) for the full design and
`.scratch/jlock/issues/` for the work breakdown.

```
jlock acquire D:\proj\src\a.ts --owner me --intent "refactor"   # waits its turn, then holds
jlock run D:\proj\dist -- bun run build                          # like flock(1)
jlock list --watch                                               # everything locked on this machine
jlock release --owner me
```
