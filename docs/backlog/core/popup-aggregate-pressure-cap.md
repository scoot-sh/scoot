---
title: "Aggregate popup pressure across connections can still stall the compositor with no client past its cap"
status: "open"
area: "core"
priority: "medium"
blocked: null
---

# Aggregate popup pressure across connections

Filed 2026-09-26 from the PR #253 re-review. Serves **daily-drive** first:
a desktop that freezes for seconds is not one people can depend on, no
matter how well-behaved any single app is.

With the per-client cap in place (PR #253: one client may hold 128 live
`xdg_popup`s, the 129th is disconnected with `wl_display.no_memory`), no
*single* connection can reach the quadratic regime on its own. But
connections multiply: N connections x 128 bufferless popups all land in
the one global `PopupManager` tree, and the reviewer verified the hot
scans are global, not per-client (`PopupManager::find_popup`'s
per-first-commit walk of every tree into a fresh `Vec`, ~90% of the stall
at 5000; `PopupTree::insert`'s whole-tree parent search per tracked popup;
the `xdg_popup` destructor's linear `known_popups` search). So some ~40
connections sitting at 128 popups each reach ~5000 popups -- about the
ticket's ~7.3 s stall (PR #253 measured with `scripts/popup-flood/run.sh`,
bufferless commits, min-of-3 on the dev VM release binary: 423.7 ms track
+ 6714.4 ms commit + 104.0 ms destroy at 5000) -- with no client tripping
its cap and nobody disconnected.

Same class as
[`pressure-many-light-connections.md`](pressure-many-light-connections.md)
for fds: per-client bounds hold, and the aggregate residual is the open
part. (There, six connections under grace held the fd table at pressure
while shedding `scootctl`; here, connections under the popup cap hold the
global tree at stall scale while every client looks innocent.)

**What already holds (not this ticket):** the single-client worst case is
bounded and measured -- a burst of 128 is admitted for ~5 ms total (same
as before the cap, no regression at the bound), while 500 refused in
~1 ms and 2000 in ~3 ms, both in the track phase with the kill message
(PR #253, same harness). This ticket is the multi-connection aggregate
only.

Scope: a global popup-pressure bound or aggregate accounting across
connections -- design open (a global live-popup ceiling with a shed
policy, aggregate scan budgeting, or a non-scan lookup are all
candidates, each with its own trade-offs), deliberately NOT implemented
in PR #253.

Evidence: a run with many connections each at or under 128 popups showing
the stall scale with nobody disconnected.
