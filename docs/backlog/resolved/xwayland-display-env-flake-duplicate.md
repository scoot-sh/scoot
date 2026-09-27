---
title: "Flake: display_reaches_spawned_children read DISPLAY as empty — DUPLICATE of the read_marker race"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Flake: `display_reaches_spawned_children_only_while_live` — DUPLICATE, closed

Closed 2026-09-27 as a duplicate of the `read_marker` redirect gap
(resolved by PR #284), per the #284 review: `""`-vs-`unset` is byte-for-byte
that race's signature on the test's *second* spawn (a gap poll returns zero
bytes, whose trim is `""` ≠ `"unset"`), and nextest process isolation means
a stale `DISPLAY=` would fail deterministically, not 1-in-N — flakiness
points at timing. Falsifier: if it ever reproduces post-#284, the
redirect-gap theory is eliminated and this reopens.

Seen once 2026-09-23 in a full `SCOOT_TEST_RENDERER=gles cargo nextest run
-p scoot` during PR #230's gate (`~/evidence/ssf/gate-7514a0a/` on the dev
VM): the test read `DISPLAY` as `""` where it expected unset. Passed 8/8
alone and on a full rerun. PR #230 touches nothing it tests.

nextest runs each test in its own process, so cross-test env sharing is not
the obvious cause; suspects are the spawned child inheriting a stale
`DISPLAY=` from the harness environment, or a timing window between the
withdraw and the spawn. Reproduce with a loop (`for i in $(seq 200)`) before
changing anything; a flaky gate erodes the one signal every PR leans on.
