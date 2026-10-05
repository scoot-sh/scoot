---
title: "Run the scoot-session launcher harness in CI, and note what it cannot see"
status: "open"
area: "testing"
priority: "medium"
blocked: null
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
