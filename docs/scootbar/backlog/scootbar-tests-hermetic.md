---
title: "scootbar integration tests are hermetic (no real config, no sound server needed)"
status: "open"
area: "scootbar"
priority: "medium"
blocked: null
---

# scootbar integration tests are hermetic (no real config, no sound server needed)

Filed 2026-10-05. Serves **daily-drive**: a contributor running the suite on
their own desktop (the M2 is the maintainer's daily machine) should get the
same answer CI does, and a red local run should mean a real bug.

## The gap

Found reviewing PR #463. `cargo nextest run --workspace` on the Asahi M2
reported 22 failures, all `scootbar::{bar,clock,visibility,workspaces}`, and
every one reproduced identically on `origin/main`. CI was green. There were
two causes, neither of them in the code under test:

- **The tests read the maintainer's real config.** `scootbar/tests/common`
  does not sandbox `XDG_CONFIG_HOME`, so the daemon loads
  `~/.config/scoot/bar.toml`. That file puts `clock` in `center`, and the
  clock tests' own placement then fails `clock is placed twice; a module goes
  in one place`.
- **They need a running sound server.** The volume module fails `no sound
  server at …/pulse/native` in a shell with no PulseAudio/PipeWire-pulse
  socket.

A reviewer spent a round proving these were environmental. The next one may
not, and may block a good PR or wave through a bad one.

## What to do

- In `scootbar/tests/common`, give every spawned daemon a fresh temp
  `XDG_CONFIG_HOME` (and `XDG_STATE_HOME`/`XDG_CACHE_HOME` where read), so no
  file outside the test's control can change a result. Pin it with a test that
  writes a conflicting `bar.toml` into the real-looking path and passes anyway.
- Make the sound-server dependency explicit. Either the volume module's tests
  run against a stub server (preferred, if cheap), or they detect a missing
  server and skip with a message, the way
  `config::tests::an_unwritable_parent_is_a_loud_refusal` probes its premise.
  Tests that are not about volume must not fail for its absence: leave volume
  out of their layouts.
- Document any remaining prerequisite in `docs/scootbar/testing.md`.

## Not in this ticket

The scoot compositor suites (they already run headless with their own
runtime dir) and CI changes (CI is already green, which is the point).
