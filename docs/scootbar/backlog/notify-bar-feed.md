---
title: "scootnotify: native DND and unread feed to the bar's `notifications` push module"
status: "open"
area: "scootbar"
priority: "low"
blocked: "notify-dnd-history-rules"
milestone: "M7"
---

# scootnotify: the bar feed

Filed 2026-10-12 from the [scootnotify design](../../../dev/design/scootnotify.md#the-bar). Serves **daily-drive**.

## What to do

On a change only, push `set notifications <payload>` to the bar's control socket, using the
exact payload shape `scoot-notify-sync` sends (`nix/modules/notifications-home.nix:143-154`).
Keep a retry watch on the socket appearing, existing only while a send is owed. Log the
not-placed guidance once per run.

## Acceptance

A headless test with a real scootbar:

- arrivals, dismissals and DND toggles are reflected exactly;
- a bar started after the daemon catches up on the next change;
- zero wakeups while nothing changes and no send is owed.

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.

## Not in this ticket

The Nix switch that removes `scoot-notify-sync` (`notify-nix`).
