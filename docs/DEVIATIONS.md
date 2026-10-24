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
