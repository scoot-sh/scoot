---
title: "scootgreet: the user list, avatars, sessions through a new `scoot-xdg` crate, remembered state"
status: "open"
area: "scootbar"
priority: "low"
blocked: "greet-ui"
milestone: "M7"
---

# scootgreet: users and sessions

Filed 2026-10-12 in the #579 fix round, split out of `greet-ui`. Serves **daily-drive**.

## What to do

- **Users** from `/etc/passwd` and `login.defs`: an empty shell field means `/bin/sh` and is
  kept, and there is "Other…" for everyone else.
- **Avatars**: AccountsService icon files, read directly.
- **Sessions** from `wayland-sessions`, through the desktop-entry parser. The parser moves out
  of scootlaunch into a new **`scoot-xdg`** crate, its second consumer; this PR adds its **CI
  classify arm** (fanning out to scootlaunch and scootgreet) and its CLAUDE.md scope entry.
- **No sessions found**: the `fallback-session` command if set, else Login is disabled and the
  directories searched are named.
- **Remembered state** in `/var/lib/scootgreet/state`.

## Acceptance

- Parser and state tests (fuzzed passwd lines).
- scootlaunch's tests pass unchanged after the move.
- The no-sessions case is tested.

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.

## Not in this ticket

Power.
