---
title: "scootnotify: the org.freedesktop.Notifications server and its model, headless"
status: "open"
area: "scootbar"
priority: "low"
blocked: "notify-prototype, scoot-dbus-extract"
milestone: "M7"
---

# scootnotify: the D-Bus server and the model

Filed 2026-10-12 from the [scootnotify design](../../../dev/design/scootnotify.md#spec-conformance). Serves **both**.

## What to do

Create `crates/scootnotify` (CI classify output and job, CLAUDE.md scope already listed):
the conformance table in the design, without drawing. Name ownership with
`DO_NOT_QUEUE`, never replacing and never replaceable, refusing with the owner named
(exit 3); the four methods and three signals; ids (0 skipped, live skipped, reserved in
blocks in `$XDG_RUNTIME_DIR`); `replaces_id` and close rules; expiry from presentation
(a stub presenter in tests); urgency defaults; caps and rate limits; markup parser;
`image-data` validation and downscale; hints. In `scoot-dbus`: an error reply body, an
error reply (`LimitsExceeded`) for an over-cap method call, a per-connection message cap.
`scootnotify daemon --check`.

## Acceptance

Against a real `dbus-daemon`: every method and signal with the right reasons, refusal
naming a running mako, `LimitsExceeded` for a 5 MiB call (no client hang), ids not reused
across a restart, flood test bounded. Fuzz targets for Notify body, markup, image-data with
committed corpora replayed on stable.

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.
- `notify-send`'s exit and timing for the over-cap case, before and after.

## Not in this ticket

Popups, control socket, Nix.
