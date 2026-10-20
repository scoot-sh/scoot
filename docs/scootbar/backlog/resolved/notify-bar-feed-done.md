---
title: "scootnotify: native DND and unread feed to the bar's `notifications` push module"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M7"
resolved: "2026-10-19"
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

## Docs (same PR)

Part of this ticket's work, not a follow-up: update `site/src/content/docs/scootbar/ (the notifications module) and site/src/content/docs/scootnotify/index.md` with the bar feed protocol and module config keys. Task-first, type/default/example/reload per option, `> **Symptom:**` boxes, snippet gates green (`site/OUTLINE.md`). A change with no user-visible effect says so in the PR instead.

## Evidence the review needs

- The standard set re-run by the reviewer on the commit under review: `cargo nextest run` for the touched crates, `cargo clippy … -- -D warnings`, `cargo fmt --check`, and the crate's fuzz targets for their CI budget.
- Raw numbers and the exact commit SHA for every measured row (CLAUDE.md, Verification and evidence); docs updated in the same PR.

## Not in this ticket

The Nix switch that removes `scoot-notify-sync` (`notify-nix`).

## Resolution

Landed 2026-10-19 in PR #689 (`feat(scootnotify): native DND and
unread feed to the bar`). The daemon pushes `set notifications
<payload>` to the bar's control socket on change only (arrival,
dismissal, expiry, restore, DND toggle), with the exact
`scoot-notify-sync` payload shape (byte-identical after key order),
live cards plus unseen history as the count, per-update icons from
`bar_icon_unread` / `bar_icon_dnd`, a socket watch armed only while a
send is owed, and the not-placed guidance once per run. Docs in the
same PR: the bar feed section in `scootnotify/configure.md` and the
`notifications` module in `scootbar/modules.md`.

- **Review findings, fixed before merge** (R-689, FIX-FIRST): F1, the
  idle test's exact-zero 2 s window, flaked on loaded runners — the
  test now asserts the load-independent core (no inotify fd, disarmed
  timerfd) with a bounded switch count; F2, one transient push failure
  froze the bar until the next change — the push is now non-blocking
  off the serving path with a 30 s deadline (it must clear loaded
  turn periods, which CI showed past 5 s) plus a bounded, capped
  re-dial (1 s, 4 tries) while owed-and-reachable, so slow and
  transient bars converge on their own; F3, a stuck bar stalled bus
  service ~3.8 s per change — the round trip now stays near baseline
  (9/4/6 ms vs 3-6 ms baseline); F4/F5, two docs lines corrected (owed
  sends land on socket appearance; the count is live plus unseen).
- **Evidence** (at the merge): `cargo nextest run -p scootnotify`
  and `cargo test -p scootnotify` green with the REQUIRE variables as
  CI; clippy `-D warnings` and `fmt --check` clean; unit pins for the
  slow/transient/silent-bar machine mutation-proved; live release
  probes (stuck bar <50 ms, 3 s-delayed converges, silent parks and
  recovers on rebind, 0 wakeups idle and owed-with-no-bar, 2 log lines
  for 2 failed changes); CI green on the exact head.
- **Handoff to `notify-nix` (M2)**: under the old Nix wiring
  `scoot-notify-sync` is still installed and the bar's `on-click`
  still runs `makoctl mode -t do-not-disturb`
  (`nix/modules/scootbar.nix`); notify-nix must remove the script and
  flip the click to `scootnotify msg dnd toggle`. Until then a stale
  sync script fights the daemon with empty pushes — but the two
  daemons cannot double-feed: mako owning the name makes scootnotify
  exit 3.
