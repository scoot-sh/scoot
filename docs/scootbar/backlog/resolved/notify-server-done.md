---
title: "scootnotify: the org.freedesktop.Notifications server and its model, headless"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M7"
resolved: "2026-10-18"
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

## Resolution

Shipped 2026-10-18 in PR #668 (`feat(scootnotify,scoot-dbus)`, 36 commits): `crates/scootnotify` (the server and the model, headless — name ownership with `DO_NOT_QUEUE` and exit-3 refusal naming the owner, the four methods with Peer/Introspectable, `NotificationClosed` reasons 1/3/4, block-reserved u32 ids, replaces/close policy, expiry from presentation, caps and rate limits, markup, image-data validation and downscale, hints, bad-config defaults, `daemon --check`), the three `scoot-dbus` additions (error body, `LimitsExceeded` for over-cap method calls, per-connection cap), the `scootnotify` CI job with classify fan-out from `scoot-dbus` and `scoot-rt`, three fuzz targets with 19 committed seeds replayed on stable, and the `/scootnotify/` site section (overview with the conformance table, CLI reference).

Evidence (dev VM, aarch64, release binary unless said): 90 + 83 unit tests and 11 daemon tests green; idle RSS 2,080 kB, 0 wakeups in 60 s; 5 MiB call answered `LimitsExceeded` in ~24 ms end-to-end with no hang (before: silent skip, sender waiting past 10 s — the prototype record); refusal exit 3 naming a `mako`-named holder; ids 1 then 1025 across a restart; 500/500 flood answered at +40 kB RSS. `notify-send` itself was unavailable (no Nix store access — the VM's disk is full), so a scripted peer sent the identically-framed giant; `mako` the package was likewise unavailable, so a `mako`-named process held the name. Review re-runs the standard set on the head commit plus the fuzz budgets in CI.
