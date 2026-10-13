---
title: "scootgreet: the greetd client, the cancel discipline, the locked secret path, `scootgreet login`"
status: "open"
area: "scootbar"
priority: "low"
blocked: "GO verdict recorded in greet-prototype (a human removes this clause); scootui-extract-draw"
milestone: "M7"
---

# scootgreet: the greetd client

Filed 2026-10-12 from the [scootgreet design](../../../dev/design/scootgreet.md#the-greetd-client). Serves **computer use**
(`scootgreet login` for VM tests and agents) and **daily-drive**.

## What to do

- **Create `crates/scootgreet`**, with its CI classify output and job; add `scootgreet` to
  CLAUDE.md's scope list.
- **The frame codec**: a native-endian u32 plus JSON, capped at 64 KiB.
- **The pure conversation state machine**, with `cancel_session` before every `create_session`
  and after every error. greetd 0.10.3 keeps a failed or abandoned session: an EOF never
  cancels (`server.rs:73`), and the next `create_session` fails with "a session is already
  being configured" (`context.rs:185-186`). A fingerprint prompt cannot be cancelled while PAM
  blocks (`context.rs:233-236`), so the UI does not offer to.
- **The supervisor `scootgreet session`**:
  - cancel first;
  - start cage by absolute path, with a ready pipe;
  - a 15 s first-frame deadline and a 90 s heartbeat;
  - on a non-zero exit or a missed deadline: kill the group, cancel, and exec agreety by
    absolute path.
- **`scootgreet-fallback`**, the `sh` script that runs agreety up to twice.
- **The secret path**:
  - a locked, zeroized two-page region, a new type in `scootbg-mem` (rustix's `mlock` is
    `unsafe`, `src/mm/mmap.rs:312` in 1.1.5; never `mlockall`);
  - a hand-built `post_auth_message_response` frame;
  - `set_dumpable_behavior` (safe, `prctl.rs:140`).
- **`scootgreet login`**, reading every answer from stdin or `--answers-fd`, never from argv.
- **A fake greetd that models the retained `configuring` state**, checked against the real
  greetd on the same two flows.

## Acceptance

- Unit tests for every flow: OTP, password change, info-only, auth error, cancel, reconnect.
- A buffer-after-clear test.
- A fuzz target on frames.
- On a **real greetd**:
  - wrong-then-right;
  - a SIGKILL with a prompt pending, landing on agreety;
  - a hang (`SCOOTGREET_TEST_HANG=1`), landing on agreety after the deadline;
  - `scootgreet login` logging a test user into a scoot session.

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.
- The real-greetd transcripts (journal lines, no secrets).

## Not in this ticket

The screen.
