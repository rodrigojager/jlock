# Contributing to jlock

Thank you for looking. Bug reports, fixes, translations and **reviews of the wording** are all welcome.

## Running the tests

```
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

The tests start copies of the `jlock` binary as child processes, each with its own temporary
`JLOCK_HOME` under `target/tmp` (never your real registry) and short times set through the `JLOCK_*`
variables. They only ever stop processes they started, by PID. The stress test (`tests/leftovers.rs`)
runs 1000 cycles; `JLOCK_TEST_CYCLES=200 cargo test` makes it shorter.

The design is in [`docs/PLAN.md`](docs/PLAN.md) (Portuguese); where the code departs from it, the reason
is in [`docs/DEVIATIONS.md`](docs/DEVIATIONS.md).

## Languages

Everything jlock prints as text goes through a catalog: `locales/<tag>.json`, embedded in the binary.
English (`locales/en.json`) is the reference; any text missing from another catalog falls back to it.

### Adding a language

1. Copy `locales/en.json` to `locales/<tag>.json` (a BCP 47 tag: `pt-BR`, `zh-CN`, `ja`).
2. Translate **only the values**. Keep every key, and keep every `{marker}` exactly as it is: `{key}`,
   `{owner}` and the others are filled in by jlock. You may move a marker inside the sentence.
3. Fill in `_glossary` first (how you translate key, owner, group, intent, queue, ticket, lock,
   heartbeat, expire, force, sweep) and use those words the same way everywhere.
4. Register the tag in `src/i18n.rs` (`CATALOGS`).
5. Run `cargo test --test i18n`: it fails if a key is missing or extra, or if the markers of a value
   differ from English.

### Plurals

A text that depends on a count has two keys: `<name>_one` (used when the count is exactly 1) and
`<name>_other` (every other count), both with `{n}`. Languages with more plural forms (Russian, Arabic,
...) or none (Chinese, Japanese) write in `_other` a phrasing that reads well for any count other than 1,
for example by putting the number after a noun ("waiting: {n}").

### What is never translated

- Command and option names (`acquire`, `--no-wait`), `name:`, `*`, and the `JLOCK_*` variables.
- Everything printed with `--json`, the lines of `log.jsonl`, `index.json` and the exit codes: they are
  the interface for scripts and stay the same in every language.
- File names (`holder.json`, `log.jsonl`) and the result words of the JSON (`granted`, `queued`) when a
  text quotes them.
- The operating system's own message inside a file system error, which comes in the system's language.
- `JLOCK_DEBUG` traces.

### Width and direction

Help pages are wrapped at 100 terminal columns. Chinese and Japanese characters take two columns each
and jlock counts them that way (`src/i18n.rs`, `width`), so a line of CJK text breaks earlier; keep the
example comments in `help.cmd.*.examples` short (they are not wrapped). Arabic is printed as plain text:
jlock adds no bidirectional control characters and leaves the display to the terminal, which may show
mixed Arabic and Latin text (command names, keys) in an order that reads oddly. Suggestions that read
better in common terminals are welcome.

### Status of the languages

| Tag | Language | Status |
|---|---|---|
| `en` | English | complete (reference) |
| `pt-BR` | Português (Brasil) | complete |
| `es` | Español | complete |
| `fr` | Français | complete |
| `de` | Deutsch | complete |
| `it` | Italiano | complete |
| `zh-CN` | 简体中文 | complete, **native review pending** |
| `ja` | 日本語 | complete, **native review pending** |
| `ru` | Русский | complete, **native review pending** |
| `ar` | العربية | complete, **native review pending** |
| `hi` | हिन्दी | complete, **native review pending** |

"Native review pending" means the translation was written without a native speaker and may sound
stiff or strange. If you speak one of these languages, a review is the most useful contribution you can
make: read `jlock --lang <tag> help`, `jlock --lang <tag> help acquire` and the catalog itself, and
send what you would say differently.

## Reviewing the text

**Reviews of the text are accepted and welcome in any language, English included**: clarity, tone,
terms that read oddly, a sentence that says too much or too little. You do not need to know Rust:

- open a pull request that changes only `locales/<tag>.json` (run `cargo test --test i18n` if you can;
  CI runs it anyway), or
- open an issue with the sentence as it is today, the command that prints it, and your suggestion.
