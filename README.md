# jlock

Machine-wide locks for files, folders and named resources, as one small CLI. A fair queue in order of
arrival, heartbeat-based recovery of locks left by dead processes, deadlock detection, a list of
everything locked and by whom, and `unlock --force`. No server: the state is a folder on disk, so it
survives any crash, and with nothing locked nothing is left behind. Written in Rust; Windows, Linux and
macOS.

## Install

```
cargo install --path .
```

This puts `jlock` (`jlock.exe` on Windows) in `~/.cargo/bin`, which `rustup` already adds to `PATH`.
Once published: `cargo install jlock`.

## Three examples

```
# An agent takes the files it is about to edit; waits its turn if someone else has them.
jlock acquire D:\proj\src\a.ts D:\proj\src\b.ts --owner agent-7 --group task-42 --intent "refactor"
jlock release --group task-42 --owner agent-7

# One build at a time on this machine, like flock(1); the exit code is the build's.
jlock run D:\proj\dist -- cargo build --release

# Everything locked right now, redrawn every second.
jlock list --watch
```

Keys are paths (a locked folder also covers everything inside it), `name:<anything>`, or `*`.
Exit codes: 0 ok, 10 busy, 11 timeout, 12 deadlock, 13 not the owner, 14 lock lost, 2 usage,
20 file system error.

## Documentation

- [`docs/USAGE.md`](docs/USAGE.md): every command with examples, exit codes, environment variables,
  the layout on disk, and how to use it from a script or an agent.
- [`docs/PLAN.md`](docs/PLAN.md) (Portuguese): the full design.
- [`docs/DEVIATIONS.md`](docs/DEVIATIONS.md): where the implementation departs from the plan.

## License

MIT, see [`LICENSE`](LICENSE).
