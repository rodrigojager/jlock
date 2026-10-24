# Deviations from `docs/PLAN.md`

Points where the implementation departs from the plan, or fills a gap the plan leaves open.

## 1. `busy` vs `queued` in the `--no-wait` answer

- **What:** the tickets call the `--no-wait` refusal `busy` (exit 10); the plan's `--json` result list has
  `queued` and no `busy`.
- **Done instead:** the plan wins for the machine-readable output: `--json` prints `"result": "queued"` with
  `holder` and `position`. The human text line says `busy <key> held by <owner> (position N)`. Exit code 10.

## 2. Keys that do not look like paths

- **What:** section 5 defines path keys as inputs containing `/`, `\`, or starting with a drive letter, `.`
  or `~`, and `name:` keys; it does not say what a bare word such as `README.md` is.
- **Done instead:** anything that is not `*` and does not start with `name:` is a path, resolved from the
  current directory (`jlock acquire a.ts` locks `<cwd>/a.ts`).

## 3. The same owner asking again

- **What:** the plan does not say what happens when an owner asks for a key it already holds.
- **Done instead:** it is granted at once and `holder.json` is left as is (no counting). Holders and
  tickets of the same owner never block that owner, including through the path hierarchy.

## 4. Exit code for an interrupted wait

- **What:** the plan says CTRL+C during the wait removes the ticket, but its exit-code table has no entry
  for "interrupted".
- **Done instead:** the ticket is removed and the command exits with 130 (the shell convention for SIGINT),
  `--json` result `error` with `"error": "interrupted"`.

## 5. Waiting on conflicting keys: one ticket, ordered across queues

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

## 6. `index.json` rename: short retries only

- **What:** section 7.5 asks for retries of 50 → 1600 ms on `EBUSY`/`EPERM`/`EACCES` for renames.
- **Why not for the index:** every command that changes something rewrites `index.json`; under load the
  rename over it collides with the other writers, and waiting up to 3 s for a file that is only derived
  would slow every acquire and release.
- **Done instead:** the index rename is tried 4 times 10 ms apart, then given up (the `.tmp` is removed);
  the next change or the global sweep writes it again. It is skipped when its content would not change.
  Lock files, tickets, `broken/` and the log rotation keep the full retry ladder.
