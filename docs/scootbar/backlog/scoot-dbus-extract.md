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
Serves **both**: scootnotify serves `org.freedesktop.Notifications`, scootlaunch calls `systemd1`
and scootgreet calls `login1`. None of that is drawing.

## What to do

- Move `crates/scootbar/src/dbus/{mod,conn,proto,link,fuzz}.rs`, their tests and fixtures to
  `crates/scoot-dbus`. The MPRIS and BlueZ readers stay in scootbar. The `dbus` fuzz target and
  corpus move too.
- Its `crate::print::warn` sites (`conn.rs:640,658,766,778,789,797,828,840`;
  `link.rs:353,388`) and `runtime_dir` sites (`conn.rs:111,207`, `link.rs:302`) call `scoot-rt`.
  The hard-coded `scootbar: dbus:` prefix becomes the binary's name.
- **`scoot-dbus` depends on `scoot-rt` and on nothing that draws** (checked with `cargo tree`).
- Add `scoot-dbus` to CLAUDE.md's library scope list, and add its CI arms (fanning out to every
  consumer).

## Acceptance

- scootbar's tray, media, bluetooth and power tests pass unchanged against a real `dbus-daemon`.
- The ratchet's tray and media rows are within noise.
- `Cargo.lock` gains no external crate.

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.
- `cargo tree -p scoot-dbus` output.

## Not in this ticket

What scootnotify needs: an error body, a reply to an over-cap call, a per-connection cap.
Those land in `notify-server` with their consumer.
