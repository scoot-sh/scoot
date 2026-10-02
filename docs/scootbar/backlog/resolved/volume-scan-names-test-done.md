---
title: "Volume: scan_names test passes for the wrong reason"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M5"
resolved: "2026-10-02"
---

# Volume: scan_names test passes for the wrong reason

Filed 2026-10-02 from the volume-module review (PR #376). Serves
**daily-drive**: the directory watch is what reconnects the volume module
after a server restart, and its scan test does not test the scan.

## The gap

`scan_names` (`crates/scootbar/src/modules/volume/mod.rs`) reads the
inotify name length with `u32::from_ne_bytes`, but
`scan_names_finds_native` (`modules/volume/tests.rs`) writes it with
`to_be_bytes()`. On little-endian the length decodes as a huge number, so
`rest.len() < len` returns `true` through the malformed-tail path — the
"finds native" assertion passes without ever comparing a name.

## What to do

Write the fixture length with `to_ne_bytes()`, and add a separate
malformed case (truncated record, garbage length) asserting the
conservative `true`. Keep both paths pinned: well-formed match, and
malformed-tail-is-interesting.

## Not in this ticket

The re-probe behavior the conservative scan exists for — see
[volume-reprobe-present-socket](volume-reprobe-present-socket-done.md).

## What landed

Commit `a57fb4cd1`. The fixture writes the name length with `to_ne_bytes()`
through one `inotify_record` helper. `scan_names_compares_the_names` pins
the well-formed path (other name, `native`, a longer and a shorter name, both
orders, a NUL-padded field, empty input) and
`scan_names_treats_a_malformed_tail_as_interesting` pins the conservative
`true` (cut header, cut name, `u32::MAX` length, truncated tail after a good
record).

Evidence (dev VM, tree copied by tar to `/dev/shm/sbvol-src`, own
`CARGO_TARGET_DIR=/dev/shm/sbvol-target`, `CARGO_PROFILE_DEV_DEBUG=0` because
the VM's tmpfs was full): `cargo nextest run -p scootbar --bin scootbar` gave
`793 tests run: 793 passed, 0 skipped`; `cargo clippy -p scootbar --all-targets
-- -D warnings` clean; `cargo fmt --check -p scootbar` clean (on the Mac).
