---
title: "scootgreet: the supervisor, its wrapper and the agreety fallback, the locked secret path"
status: "open"
area: "scootbar"
priority: "low"
blocked: "greet-headless-login; GO verdict recorded in greet-prototype (a human removes this clause)"
milestone: "M7"
---

# scootgreet: the greetd client

Filed 2026-10-12 from the [scootgreet design](../../../dev/design/scootgreet.md#the-greetd-client). Serves **daily-drive**: the
supervisor that keeps a graphical greeter from stranding a login, and the secret path.

## What to do

- **Build on `crates/scootgreet`**, which `greet-headless-login` created with the frame codec,
  the conversation (with its cancel discipline) and `scootgreet login`. A fingerprint prompt
  cannot be cancelled while PAM blocks (`context.rs:233-236`), so the UI does not offer to.
- **The supervisor `scootgreet session`**:
  - cancel first;
  - start cage by absolute path, with a ready pipe;
  - a first-frame deadline (config `startup-timeout`, default 15 s, set from
    `greet-baseline`'s measurement) and a 90 s heartbeat, **both on `CLOCK_MONOTONIC`** so a
    suspend at the login screen does not trip them;
  - on a non-zero exit or a missed deadline: kill the group, cancel, **print the reason to the VT
    and append it to `/var/lib/scootgreet/fallback.log`**, and exec agreety by absolute path.
- **The `scootgreet-greeter` wrapper** that greetd runs. greetd `exec`s its command under
  `sh -c` (`worker.rs:228-235,267-272`), so the fallback must live in a script, not in the
  command text. On a non-zero exit, the wrapper logs the supervisor's status itself, ignoring a
  failed write, then execs `scootgreet-fallback`.
- **`scootgreet-fallback`**, with the session command **baked in at build or install time**
  (never read from `greet.toml`, which only the failed supervisor reads). An empty command omits
  `--cmd` so agreety prompts `Command:` (`agreety/src/main.rs:103-106,139,159`). It re-runs
  agreety only on a non-zero exit (the stale-session case). agreety exits 0 after its
  `--max-failures` too (`main.rs:173-182`), and greetd's restart handles that.
- **The supervisor's only exit-0 path is cage exiting 0.** Everything else is non-zero.
- **A hung cage is not caught.** The heartbeat proves scootgreet alive, not cage drawing; this is
  documented as accepted.
- **The secret path**:
  - a locked, zeroized two-page region: a new type in `scootbg-mem`, behind a `locked` feature
    off by default so scootbg's release-gated binary never builds it, with the crate's
    description updated (rustix's `mlock` is `unsafe`, `src/mm/mmap.rs:312` in 1.1.5; never
    `mlockall`);
  - a hand-built `post_auth_message_response` frame;
  - `set_dumpable_behavior` (safe, `prctl.rs:140`).

## Acceptance

- A buffer-after-clear test on the locked page, and the password frame's escaping.
- On a **real greetd**:
  - wrong-then-right;
  - a SIGKILL with a prompt pending, landing on agreety;
  - a hang (`SCOOTGREET_TEST_HANG=1`), landing on agreety after the deadline;
  - the supervisor itself missing, exiting non-zero at once, and aborting, each through the real
    wrapper, each reaching an agreety login that starts the scoot session, with the wrapper's
    status line logged;
  - a fallback with no baked-in command, where agreety prompts `Command:`;
  - a non-executable wrapper caught by the tests before it ships;

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.
- The real-greetd transcripts (journal lines, no secrets).

## Not in this ticket

The screen.
