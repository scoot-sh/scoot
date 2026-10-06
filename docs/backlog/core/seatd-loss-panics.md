---
title: "scoot panics when seatd dies under a running session (libseat unwrap on ENOTCONN)"
status: "open"
area: "core"
priority: "high"
blocked: null
---

# scoot panics when seatd dies under a running session (libseat unwrap on ENOTCONN)

Filed 2026-10-06 from the PR #485 live work on the Asahi M2. Serves
**daily-drive**: a compositor crash takes every client's unsaved state
with it, and CLAUDE.md treats a crash path with the same severity as data
loss.

## The gap

Killing `seatd` under a running `--tty` scoot panics the compositor:
`backend/session/libseat.rs:215` in the pinned Smithay fork (`035d447c`)
calls `unwrap` on an `ENOTCONN` from the seat connection. On a systemd
login the seat comes from logind, so the trigger is uncommon there. It still
applies to seatd setups and to a logind restart, if that path shares the
code. Check both.

## What to do

- Reproduce on the M2 with the private seatd + `openvt` pattern (or the
  dev VM): start `--tty` scoot, kill seatd, and see the panic.
- Find where the error surfaces. If it is Smithay's own `unwrap`, decide
  between a scoot-side guard (catching the session event and shutting
  down cleanly) and a fork commit, with the evidence `docs/forks.md`
  requires.
- The behavior to aim for: a clean, logged shutdown, or a session pause
  with a later resume if libseat supports reconnecting. Never a panic.
- A test that fails before the fix, if the session path can be exercised
  headless; otherwise a recorded live repro.
