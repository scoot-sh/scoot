---
title: "Flake: xwayland::tests::display_reaches_spawned_children_only_while_live read DISPLAY as \"\" instead of unset"
status: "open"
area: "testing"
priority: "low"
blocked: null
---

# Flake: `display_reaches_spawned_children_only_while_live`

Seen once 2026-09-23 in a full `SCOOT_TEST_RENDERER=gles cargo nextest run
-p scoot` during PR #230's gate (`~/evidence/ssf/gate-7514a0a/` on the dev
VM): the test read `DISPLAY` as `""` where it expected unset. Passed 8/8
alone and on a full rerun. PR #230 touches nothing it tests.

nextest runs each test in its own process, so cross-test env sharing is not
the obvious cause; suspects are the spawned child inheriting a stale
`DISPLAY=` from the harness environment, or a timing window between the
withdraw and the spawn. Reproduce with a loop (`for i in $(seq 200)`) before
changing anything; a flaky gate erodes the one signal every PR leans on.
