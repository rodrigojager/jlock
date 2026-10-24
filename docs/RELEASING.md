# Releasing jlock

A release is a git tag `vX.Y.Z`. Pushing it runs `.github/workflows/release.yml`, which builds jlock
for every target, packs the archives, writes `SHA256SUMS` and publishes the GitHub Release that
`install.sh`, `install.ps1`, `cargo binstall` and `jlock update` download from.

## What gets published

| Target | Runner | Archive |
|---|---|---|
| `x86_64-pc-windows-msvc` | `windows-latest` | `jlock-X.Y.Z-x86_64-pc-windows-msvc.zip` |
| `x86_64-unknown-linux-musl` (static) | `ubuntu-latest` | `jlock-X.Y.Z-x86_64-unknown-linux-musl.tar.gz` |
| `aarch64-unknown-linux-musl` (static) | `ubuntu-24.04-arm` | `jlock-X.Y.Z-aarch64-unknown-linux-musl.tar.gz` |
| `x86_64-apple-darwin` | `macos-14` (cross-built from arm64) | `jlock-X.Y.Z-x86_64-apple-darwin.tar.gz` |
| `aarch64-apple-darwin` | `macos-14` | `jlock-X.Y.Z-aarch64-apple-darwin.tar.gz` |

Each archive holds the binary (`jlock` or `jlock.exe`), `LICENSE` and `README.md` at its root.
`SHA256SUMS` lists every archive in the `sha256sum` format (`<hash>  <file>`).

## Step by step

1. Make sure `main` is green: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
   `cargo test`.
2. Bump `version` in `Cargo.toml` (and run `cargo build` so `Cargo.lock` follows).
3. Add a section to `CHANGELOG.md` whose heading contains the version, such as `## 0.2.0 — 2026-11-02`.
   Its text becomes the release notes; without it, GitHub generates notes from the commits.
4. Commit: `git commit -am "release 0.2.0"`.
5. Tag and push: `git tag v0.2.0`, `git push`, `git push --tags`.
6. Watch the `release` workflow. It stops early if the tag does not match `Cargo.toml`
   (`v0.2.0` ↔ `0.2.0`), runs the tests on every runner, then builds and packs.
7. Check the Release page: five archives and `SHA256SUMS`.
8. Try the one-line installers in a clean shell:
   - Linux/macOS: `curl -fsSL https://raw.githubusercontent.com/rodrigojager/jlock/main/install.sh | sh`
   - Windows: `irm https://raw.githubusercontent.com/rodrigojager/jlock/main/install.ps1 | iex`

   then, in a new terminal, `jlock --version` and `jlock doctor`. `jlock update --check` on an older
   install should now see the new version.

## When a target fails

- The `publish` job only runs when every build succeeded, so a failed target publishes nothing. Fix the
  problem on `main`, then move the tag: `git tag -d v0.2.0`, `git push --delete origin v0.2.0`, tag the
  fixed commit and push the tag again.
- If a Release was already created (for example by a manual run) and is wrong, delete it on GitHub
  before pushing the tag again; `gh release create` refuses an existing tag.
- A runner image that GitHub retired (as happened to `macos-13`) shows as a job that waits forever: move
  that target to a current runner in the matrix.

## What the checksum protects, and what it does not

`SHA256SUMS` comes from the same Release as the archives. The installers and `jlock update` check it,
which catches a download that was cut short or corrupted. It does not protect against a compromised
GitHub account or repository: whoever can replace the archives can replace the sums too. There is no
code signing yet.
