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
