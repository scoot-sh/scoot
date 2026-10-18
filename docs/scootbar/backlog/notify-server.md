---
title: "scootnotify: the org.freedesktop.Notifications server and its model, headless"
status: "open"
area: "scootbar"
priority: "low"
blocked: null
milestone: "M7"
---

# scootnotify: the D-Bus server and the model

Filed 2026-10-12 from the [scootnotify design](../../../dev/design/scootnotify.md#spec-conformance). Serves **both**.

## What to do

Create `crates/scootnotify`, with its CI classify output and job (the CLAUDE.md scope is already
listed). Build the design's conformance table, without drawing:

- **Name ownership** with `DO_NOT_QUEUE`, never replacing and never replaceable, refusing with
  the owner named (exit 3).
- **The four methods and three signals.**
- **Ids**: 0 skipped, live ids skipped, reserved in blocks in `$XDG_RUNTIME_DIR`.
- **`replaces_id` and close rules.**
- **Expiry** from presentation (a stub presenter in tests), and the urgency defaults.
- **Caps and rate limits**, the markup parser, `image-data` validation and downscale, and hints.
- **A bad config at start runs on built-in defaults** and reports the error (never an exit
  loop).

In `scoot-dbus`:

- an error reply with a body;
- a `LimitsExceeded` reply for an over-cap method call (the discard path already reads the
  header fields);
- a per-connection message cap.

`scootnotify daemon --check`.

## Acceptance

Against a real `dbus-daemon`:

- every method and signal, with the right reasons;
- refusal naming a running mako;
- `LimitsExceeded` for a 5 MiB call, with no client hang;
- ids not reused across a restart;
- the flood test bounded.

Fuzz targets for the Notify body, markup and image-data, with committed corpora replayed on stable.

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.
- `notify-send`'s exit and timing for the over-cap case, before and after.

## Not in this ticket

Popups, the control socket, Nix.
