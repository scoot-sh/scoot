---
title: "smoke-test.sh's xwayland step keys on `Xwayland` in PATH, not on the build's `xwayland` feature"
status: "open"
area: "testing"
priority: "medium"
blocked: null
---

# Smoke xwayland step: gate on the build feature, not the binary

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
