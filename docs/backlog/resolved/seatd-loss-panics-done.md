---
title: "scoot panics when seatd dies under a running session (libseat unwrap on ENOTCONN)"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-06"
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

## Resolution (2026-10-07)

- **The unwrap is Smithay's own**, verified against the pinned fork's
  source: `backend/session/libseat.rs:215` at `7ab72d53`
  (`dispatch(0).unwrap()` in `process_events`; the same file also unwraps
  `dispatch` in `new` and `disable()` in the `Disable` arm). Killing
  seatd makes `libseat_dispatch` fail `ENOTCONN` (errno 107), and the
  `unwrap` panics (exit 101) before scoot's `session_event` callback ever
  runs. Reproduced live on the Asahi M2 (private seatd + `openvt`, VT2):
  `thread 'main' panicked at .../libseat.rs:215:57: called
  Result::unwrap() on an Err value: Errno { code: 107 ... }`.
- **Both seat paths share this code.** Smithay's session module has
  exactly one implementation (libseat); libseat picks its seatd/logind
  backend internally (`LIBSEAT_BACKEND` or first-successful). The seatd
  path was verified live; a logind restart was not tested live (it would
  drop the shared machine's own sessions). libseat offers no reconnect
  (`open_seat`/`close_seat` only), so resume is impossible on any route:
  the behavior is a clean, logged shutdown.
- **Fix: one fork commit**, `fdf424d` on
  `scoot/cursor-dmabuf-storage`, repinned in `crates/scoot/Cargo.toml`
  (+ `flake.nix` `outputHashes`, `docs/forks.md`). `dispatch`/`disable`
  failures now return a new `Error::ConnectionLost`, which the event loop
  propagates out of `run`: exit 1 with `scoot: ... Lost the seat
  connection...`, no panic. A scoot-side guard was tried first (a
  `catch_unwind` `EventSource` wrapper, spiked and reverted): the exit
  was clean but the panic hook still prints, and it could not cover the
  `register` sites — full evidence in `docs/forks.md`.
- **Proof:** recorded live repros on the M2, same steps both times
  (private seatd + `openvt`, `--tty`, kill seatd once up): before, panic
  + exit 101; after, `Lost the seat connection` + exit 1. No headless
  test is possible (the seat path needs a live seatd and a VT).
