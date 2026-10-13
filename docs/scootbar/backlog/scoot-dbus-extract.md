---
title: "Extract `scoot-dbus`: move the hand-rolled D-Bus client into its own crate"
status: "open"
area: "scootbar"
priority: "low"
blocked: "scootui-extract-draw"
milestone: "M7"
---

# Extract `scoot-dbus`

Filed 2026-10-12 from the [shell-apps design](../../../dev/design/shell-apps.md#scoot-dbus-library-cratesscoot-dbus).
Serves **both**: scootnotify serves `org.freedesktop.Notifications`, scootlaunch calls
`systemd1`, scootgreet calls `login1`; none of that is drawing.

## What to do

Move `crates/scootbar/src/dbus/{mod,conn,proto,link,fuzz}.rs` and their tests and
fixtures to `crates/scoot-dbus` (MPRIS and BlueZ readers stay in scootbar). The `dbus` fuzz
target and corpus move too. Add `scoot-dbus` to CLAUDE.md's library scope list
(`docs(claude)` line in this PR) and its CI arms (fanning out to every consumer).

## Acceptance

scootbar's tray, media, bluetooth and power tests pass unchanged against a real
`dbus-daemon`; the ratchet's tray and media rows within noise; `Cargo.lock` gains no
external crate.

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.

## Not in this ticket

The additions scootnotify needs (an error body, a reply to an over-cap call, a
per-connection cap): they land in `notify-server` with their consumer.
