---
title: "scootnotify: native DND and unread feed to the bar's `notifications` push module"
status: "open"
area: "scootbar"
priority: "low"
blocked: "notify-control"
milestone: "M7"
---

# scootnotify: the bar feed

Filed 2026-10-12 from the [scootnotify design](../../../dev/design/scootnotify.md#the-bar). Serves **daily-drive**.

## What to do

Push `set notifications <payload>` to the bar's control socket on change only, the exact
payload shape `scoot-notify-sync` sends (`nix/modules/notifications-home.nix:143-154`), with
the retry-on-socket-appearance watch existing only while a send is owed, and the
not-placed guidance once per run.

## Acceptance

Headless test with a real scootbar: arrivals, dismissals, DND toggles reflected exactly;
bar started after the daemon catches up on the next change; zero wakeups while nothing
changes and no send is owed.

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.

## Not in this ticket

The Nix switch that removes `scoot-notify-sync` (`notify-nix`).
