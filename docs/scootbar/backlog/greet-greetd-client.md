---
title: "scootgreet: the greetd client, the conversation state machine, the secret's path, `scootgreet login`"
status: "open"
area: "scootbar"
priority: "low"
blocked: "greet-baseline, scootui-extract-draw"
milestone: "M7"
---

# scootgreet: the greetd client

Filed 2026-10-12 from the [scootgreet design](../../../dev/design/scootgreet.md#the-greetd-client). Serves **computer use**
(`scootgreet login` for VM tests and agents) and **daily-drive**.

## What to do

Create `crates/scootgreet` (CI classify and job; add `scootgreet` to CLAUDE.md's scope list).
The frame codec (native-endian u32 + JSON, 64 KiB cap), the pure conversation state machine
for every response kind and order, the hand-built zeroized `post_auth_message_response` frame,
`PR_SET_DUMPABLE` 0 (verify `rustix` exposes it safely; record whether `mlock` can be done
under `forbid(unsafe_code)`), a fake greetd for tests, and the headless `scootgreet login`.

## Acceptance

Unit tests for every flow in the design (OTP, password change, info-only, auth error,
cancel); a buffer-after-clear test; fuzz target on frames; `scootgreet login` logs a test user
into a scoot session through a real greetd on the dev VM.

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.
- The real-greetd login transcript (journal lines, no secrets).

## Not in this ticket

The screen.
