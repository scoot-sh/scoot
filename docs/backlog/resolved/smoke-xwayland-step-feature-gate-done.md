---
title: "smoke-test.sh's xwayland step keys on `Xwayland` in PATH, not on the build's `xwayland` feature"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Smoke xwayland step: gate on the build feature, not the binary

**RESOLVED (2026-09-25) with XWayland Phases 2+3.** Both defects fixed in
`scripts/smoke-test.sh`'s xwayland section:

- **The feature gate.** The step no longer asks whether `Xwayland` is on
  `PATH`; it waits for whichever of the three startup lines the binary under
  test logs for `--xwayland` -- `XWayland is ready`, the no-feature warning
  (`has no xwayland support`), or the spawn failure (`could not be
  started`) -- and branches on that. A default build (cage's wrapper puts
  `Xwayland` on `PATH`) now prints `skipped -- this build has no xwayland
  feature` instead of waiting for a READY that cannot come; an `xwayland`
  build that fails to start its server with the binary present is still a
  `BUG`.
- **The `--tty` seat.** Under `MODE=--tty` the section runs `--headless`
  (the opt-in path is backend-agnostic) instead of launching a second
  `--tty` compositor into the seat the main session holds.
- With `READY` and `xeyes` on `PATH` the section now also spawns `xeyes`,
  asserts it is listed (`app_id` `XEyes`) and closes it -- the Phase 2
  mapping, end to end over IPC.

The original report follows.

Filed 2026-09-23 by the PR #223 review (pre-existing since `772dbc0`, PR
#221). `scripts/smoke-test.sh` runs its `--xwayland` step whenever
`command -v Xwayland` succeeds. NixOS's `cage` wrapper puts Xwayland on
`PATH`, so under `MODE=--nested` in cage a default build (no `xwayland`
feature) logs "continuing Wayland-only" and the script waits forever for
READY, then fails with `BUG: --xwayland with the binary present never became
ready`. Reproduced on base `668b112`: 20 `ok:` then that BUG. Headless smoke
is unaffected (Xwayland is not on PATH there).

Fix: decide the step from whether the binary under test was built with
`xwayland` (e.g. a startup log line, a `--version`/`--help` feature
listing, or the compositor's own refusal message), skip it loudly
otherwise, and keep a real failure when the feature *is* built and the
server never comes up. Serves reliability of the standard verification set
(both priorities).

## Second defect, same section (found 2026-09-23, PR #229)

Under `MODE=--tty` the smoke script's `--xwayland` section launches a second
`--tty` compositor while the main one (`"$SCOOT" "$MODE"`, started at
`scripts/smoke-test.sh:~247` and killed only by the end-of-script trap) still
holds the seat, so the launch is refused `EPERM` and the run exits rc=1.
Reproduced on `a3b883e` (`~/evidence/gdf/gate2/15-smoke-tty-gles-before1.log`
on the dev VM). Not the malformed-config instance -- `run_broken_config_test`
hard-codes `--headless`. Fix alongside the feature gate: kill and wait on
`$compositor` before the xwayland section (the later `$LOG` grep only needs
the log file), or run that section `--headless` when `MODE=--tty`.
