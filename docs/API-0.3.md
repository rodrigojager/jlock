# jlock 0.3 for integrators

What changed in 0.3 for a program that drives jlock through `--json` (the first one is jtasks). The full
reference is [`USAGE.md`](USAGE.md) (sections Modes, Owners, Resources); this page is the short version,
with the exact JSON. jlock 0.3 does not read locks taken by 0.2 (the on-disk layout changed): let every
0.2 lock go before switching.

## 1. Owners are paths

`--owner` is a path of names separated by `/` (`gt:<run>/<task>`, `gt:<run>/<task>/exec-<n>`). Empty
parts and a `/` at either end are refused with exit 2.

- An owner, its ancestors and its descendants never block each other, in any mode, and never queue
  behind each other's tickets.
- A request covered by a hold of an ancestor (that hold's key is the same key, a folder above it, or
  `*`) is granted at once, skipping the queue, as a **nested** hold. Its JSON says `"nested": true` and
  names the ancestor's hold in `parent` (`{owner, key, grant}`; the nearest ancestor). A covered request
  still waits for an *unrelated* hold that conflicts with it.
- A nested hold ends when its parent hold ends (release, ttl, death of the holder, `unlock --force`),
  recursively. It is logged as `expired` with `"reason": "parent"`; a `hold`/`run` keeping it prints
  `lost` and exits 14. A child's own release never touches the parent.
- An ancestor asking for a key whose only other holders are its descendants is granted (not nested).
- Deadlock detection treats a descendant's wait as its ancestors' wait.

For jtasks this removes the need for the "held under the exclusive task's owner" workaround: give the
task the owner `gt:<run>/<task>` and every `jtasks exec` child the owner `gt:<run>/<task>/exec-<n>`.
The child is granted at once whatever the task holds (`*`, folders, files); other tasks' children wait
for the task as before. Releasing the task (`release --owner gt:<run>/<task>` or `--group`) also ends its
children's nested holds.

## 2. Modes

`--mode shared|exclusive` on `acquire`, `hold` and `run` (default `exclusive`), or per key with a
suffix: `src/a.ts@shared`, `resource:desktop@exclusive`. Precedence: key suffix, `--mode`, the
resource's default, `exclusive`. A key named twice is taken once, in the stronger mode.

- Shared holds of conflicting keys coexist; exclusive conflicts with any other hold of a conflicting
  key (path hierarchy applies across modes: shared `src` vs exclusive `src/a.ts` conflict).
- Fair queue: a request waits behind every older ticket that conflicts with it, so an exclusive waiter
  is never starved by later shared requests.
- Same owner: asking again in the same or a weaker mode is granted at once (no new grant,
  `"grant": null`); holding shared and asking exclusive waits like any exclusive request and adds a
  second hold. `release` gives back every hold the owner has on the key.

## 3. `resource:` keys

`resource:<name>[:<arg>]`, checked against a catalogue: built-in `audio`, `browser`, `clipboard`,
`desktop`, `gpu`, `machine`, `network`, `port:<n>` (1..65535, normalized: `resource:port:08080` is
`resource:port:8080`), plus the user's `<JLOCK_HOME>/resources.json` (format in USAGE.md). Unknown
names, a missing or extra argument, and a bad port are refused with exit 2 and
`{"result":"error","error":"unknown resource 'desktp'; did you mean resource:desktop? (jlock resources lists them)"}`.
Names are folded to lower case. A resource conflicts only with the identical resource and argument
(and `*`), never with paths or `name:` keys. Checked by `acquire`, `hold`, `run`, `status`; not by
`release`, `unlock`, `renew`, `heartbeat`.

`resource:machine` exclusive is the way to ask for a quiet machine (jobs that load it take it shared);
`*` still works but stops every lock of every project.

`jlock resources --json`:

```json
{"file":"<JLOCK_HOME>/resources.json","resources":[
  {"arg":null,"description":"the real mouse, keyboard and windows of the desktop session","key":"resource:desktop","mode":"exclusive","name":"desktop","source":"builtin"},
  {"arg":"n","description":"a TCP or UDP port of this machine, by number (resource:port:8080)","key":"resource:port:<n>","mode":"exclusive","name":"port","source":"builtin"}],
 "result":"ok","warnings":[]}
```

## 4. JSON that changed

Fields are always present unless marked optional. Unchanged commands (`renew`, `heartbeat`,
`release`, `unlock`, `gc`, `doctor`) keep their 0.2 shapes; `unlock` adds `grant` to each `forced`
entry.

### `acquire`, `hold`, `run` (and `hold --detach`): granted

```json
{"key":"d:/x/a.ts","keys":["d:/x/a.ts"],
 "locks":[{"grant":"1794179416132806-27208","key":"d:/x/a.ts","mode":"exclusive","nested":true,
           "parent":{"grant":"1794179416107717-36394","key":"*","owner":"gt:r/T1"}}],
 "mode":"exclusive","owner":"gt:r/T1/exec-1","result":"granted","waited_ms":5}
```

- New: `locks` (one entry per key, in canonical order: `key`, `mode`, `grant` — `null` when the owner
  already held it —, `nested`, `parent` — `null` unless nested). `mode` (top level) only with one key,
  like `key`.
- Text: `granted <key>`, plus ` (shared)` and ` (nested in <owner>)` when they apply.

### `acquire`, `hold`, `run`: busy (`--no-wait`, exit 10) and timeout (exit 11)

```json
{"failed":"d:/x/a.ts","key":"d:/x/a.ts","keys":["d:/x/a.ts"],"mode":"exclusive",
 "owner":"gt:r/T2/exec-1","position":1,"released":[],"result":"queued",
 "holder":{"key":"d:/x/a.ts","owner":"gt:r/T1/exec-1","mode":"exclusive",
           "parent":{"grant":"1794179416107717-36394","key":"*","owner":"gt:r/T1"},
           "pid":15256,"host":"…","cwd":"…","since":"2026-11-08T23:10:16.132Z"},
 "holders":[{"…the same hold…"},
            {"key":"*","owner":"gt:r/T1","mode":"exclusive","group":"g1","pid":21888,"…":"…"}]}
```

- `holder`: the hold in the way (one on the key itself if any), or `null` when only older tickets are
  in the way. `holders` (new): every hold that blocks the request. Hold bodies carry `mode` and, when
  nested, `parent` (optional fields `group`, `intent`, `meta`, `ttl_secs` as before).
- `mode` (new): the mode that was asked for the failed key. `via` (optional) as before.

### `status --json`

```json
{"result":"ok","key":"d:/x/a.ts","state":"held",
 "holder":{…the oldest hold…},
 "holders":[{"key":"d:/x/a.ts","owner":"gt:r/T1/exec-1","mode":"exclusive","grant":"1794179416132806-27208",
             "nested":true,"parent":{"grant":"1794179416107717-36394","key":"*","owner":"gt:r/T1"},
             "pid":15256,"host":"…","cwd":"…","since":"…","expires":null,
             "held_ms":353,"heartbeat_ms":353,"stale":false,"late":false}],
 "queue":[{"position":1,"owner":"w","mode":"exclusive","since":"…","waiting_ms":40,"ticket":"…"}],
 "conflicts":[{"key":"*","holder":{…},"holders":[{…}],"queue":0}]}
```

- New: `holders` (every hold, in grant order), `holder` is now the first of them (or `null`); each hold
  has `mode`, `grant`, `nested`, `parent`. Queue entries have `mode`. Conflicts have `holders`.
- `stale` is also `true` for a nested hold whose parent has ended (it is about to be expired).

### `list --json`

```json
{"result":"ok","locks":[{"key":"…","holder":{…first hold…},"holders":[{…as in status…}],
  "queue":[{"position":1,"owner":"…","mode":"shared","group":null,"since":"…","waiting_ms":0,
            "heartbeat_ms":0,"stale":false,"ticket":"…"}]}]}
```

- New: `holders`; hold entries now carry the whole body as `status` shows it (`mode`, `grant`, `nested`,
  `parent`, plus the 0.2 fields). Queue entries have `mode`. `--owner`/`--group` match any hold or ticket.

### `graph --json`

```json
{"result":"ok","cycles":["S → name:x (held by P) → P ⊃ P/exec → name:y (held by S) → S"],
 "edges":[{"from":"w","to":"r1","key":"name:db","via":"held","ticket":"…","mode":"exclusive","to_mode":"shared"}]}
```

- New: `mode` (what `from` asks) and `to_mode` (the mode of the hold or ticket of `to` in the way).
  A chain step taken through a descendant's wait reads `P ⊃ P/exec`.

### `hold`/`run` events (unchanged shapes)

`{"result":"lost","key","owner","holder","reason"}` (reason `lost`, `released`, `ttl`) and
`{"result":"released","keys","owner"}`. A nested hold whose parent ended gives `lost`.

### `index.json` and `log.jsonl`

- `index.json` entries: `{"key","holders":[{"owner","mode","group","intent","since","grant","parent"}],
  "queue":[{"owner","mode","since"}]}`.
- `log.jsonl`: `acquire` has `mode`, `grant`, `parent`; `queued` has `mode`; `release` has `grants`;
  `expired` and `forced` have `grant`, and `expired` may have `"reason":"parent"`.

## 5. On disk

`locks/<sha1>/holds/<grant>.json` (one file per hold; mtime = heartbeat; OS lock of `hold`/`run`),
`holds/<grant>.lease` (renewed deadline), a short-lived `gate` file, `queue/`, `broken/`, `key`.
Grant and ticket names are `<micros, 16 digits>-<5 digits>`. Nothing should read these files directly:
use `--json`.
