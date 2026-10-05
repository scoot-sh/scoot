---
title: "Run the scoot-session launcher harness in CI, and note what it cannot see"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-05"
---

# Run the scoot-session launcher harness in CI, and note what it cannot see

Filed 2026-10-04 from PR #431's review (N1, N3). Serves **daily-drive**
(the launcher is what stands between a login and a stranded seat).

## The gap

- `scripts/scoot-session-test.sh` (17 asserts, the proof that
  `graphical-session.target` is reached only after the display import) is run
  by hand only: nothing in `.github/` or `nix/` calls it. It needs only `sh`
  and `python3` (it fakes `systemctl`), and runs on ubuntu-latest and macOS.
- Its fake `start scoot-session.target` re-fires the graphical-bound probe
  unconditionally; real systemd would not for an already-active
  `graphical-session.target`, so the harness cannot see the carried
  limitation below.
- Carried limitation (pre-dates #431): after a launcher killed past target
  start (SIGKILL, power loss), the next login heals `scoot-session.target`
  and `scoot.service` but deliberately not the shared
  `graphical-session.target`, so already-active graphical units (bar,
  swayidle) keep the old session's display until that session's logout.

## What to do

Add the harness as a CI step in a job path-filtered on
`resources/scoot-session`, `resources/systemd/**`,
`scripts/scoot-session-test.sh` and `nix/tests.nix` (CLAUDE.md: CI runs only
what changed). Make the fake match systemd's activation semantics (Wants
fire only on inactive to active) and add a case that shows the carried
limitation, then decide whether the heal should restart the session's own
graphical-bound units (without touching another desktop's) and say why.

## Not in this ticket

Any change to the unit design #431 landed.

## Resolved by PR #453 (2026-10-05)

- CI: new `session-harness` job in `.github/workflows/ci.yml`, gated on a
  new `session` classify output (`resources/scoot-session`,
  `resources/systemd/*`, `scripts/scoot-session-test.sh`; `nix/tests.nix`
  stays on the catch-all, which runs this job too). sh + python3 only, no
  nix, ubuntu-latest, ~20 s. Green on the PR head.
- Fake matches systemd activation semantics (Wants fire only on the
  inactive-to-active transition); T10 pins the carried limitation (a heal
  over a live foreign graphical target leaves its units on the old
  session's display until logout).
- Decision: the heal does NOT restart graphical-bound units. It cannot
  tell the session's own graphical-bound units from another desktop's in
  the shared-target case (the harness T5 shape: an active graphical target
  with scoot's units never run is another session's), and stopping them
  all would tear down that session. Stale-display units die at logout;
  the next login is clean.
- What CI still cannot see (stays manual): a real `systemd --user`
  (activation edge cases the fake may not match) and the Asahi idle-CPU
  measurement (the sibling ticket's before/after).
