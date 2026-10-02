---
title: "Volume: scan_names test passes for the wrong reason"
status: "open"
area: "scootbar"
priority: "medium"
blocked: null
milestone: "M5"
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
[volume-reprobe-present-socket](volume-reprobe-present-socket.md).
