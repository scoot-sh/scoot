---
title: "Aggregate popup pressure across connections can still stall the compositor with no client past its cap — RESOLVED"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Aggregate popup pressure across connections — RESOLVED

RESOLVED 2026-09-26 (PR #TBD). Design winner, cheapest-first per the
ticket: **option 1, non-scan lookup** -- a scoot-side surface-to-popup
index (`compositor/popup_index.rs`) replaces `PopupManager::find_popup`
on the initial-configure path, paired with a membership check over the
popup's *own* tree (same-client parenting is all the protocol allows, so
that tree holds at most its owner's capped popups). No new refusal
semantics, no behavior change for legitimate clients, the per-client 128
cap + kill exactly as is, grab/focus untouched. No README change: no new
config, keybinding, CLI flag, or IPC surface.

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

## What was built, and why the others lost

Measured first, with a new aggregate harness
(`scripts/popup-flood/popup-flood-many.c` + `run-many.sh`: K connections
x M popups each, barriers separating track / commit / destroy, min-of-3
on the dev VM release binary). Before, all ADMITTED with nobody
disconnected:

| total | track | commit | destroy | total |
| --- | --- | --- | --- | --- |
| 10 x 128 = 1280 | 25.7 ms | 194.4 ms | 10.7 ms | ~231 ms |
| 20 x 128 = 2560 | 44.3 ms | 770.6 ms | 27.5 ms | ~842 ms |
| 40 x 128 = 5120 | 76.4 ms | 3875.6 ms | 76.7 ms | ~4029 ms |

Commit is ~96% of the stall and scales ~4-5x per doubling: it is
`find_popup`'s per-first-commit walk of every tree into a fresh `Vec`,
exactly the reviewer's diagnosis. Track (per-tree inserts, one tree per
connection's window) scales ~1.7x per doubling -- bounded by the
per-client cap, as predicted: a client names only its own objects as a
popup's parent, so no tree outgrows its owner's 128. Destroy
(`known_popups` linear search) scales ~2.7x per doubling with a small
constant.

So the scans were truly the whole story, and only one of them mattered:
**option 1 won**. `PopupIndex` files every tracked popup under its
surface id at `new_popup` (only once tracking takes it) and forgets it
at `popup_destroyed` (only when the dying popup owns its record -- the
same ownership check as the count); the configure path reads it back in
O(1). No fork, no new refusal, nothing legitimate can observe.

**Option 2 (global ceiling + shed policy) lost:** there is no stall left
that a ceiling would need to shed -- and a ceiling would invent refusal
semantics (which client sheds? silent drop vs kill? grabs held by shed
popups?) for traffic shaped exactly like legitimate use. After:

| total | track | commit | destroy | total |
| --- | --- | --- | --- | --- |
| 1280 | 18.1 ms | 40.4 ms | 11.6 ms | ~70 ms |
| 2560 | 40.8 ms | 74.9 ms | 28.0 ms | ~144 ms |
| 5120 | 73.5 ms | 131.9 ms | 76.0 ms | ~281 ms |

Commit at 5120: 3875.6 ms -> 131.9 ms (29x); total 4.03 s -> 0.28 s
(14x). Commit now scales ~1.8x per doubling (linear-ish residual: the
unmapped `position` scan, configure sends, one constant-size own-tree
walk per first commit).

**Option 3 (aggregate scan budgeting) lost:** nothing left to budget --
the remaining scans are linear-per-op with small constants (see
residual).

A subtlety the tests caught: a dismissed popup's object lives on while
its node is gone (a refused grab tears the node down mid-commit,
Smithay-side `ungrab` paths too), and the pinned test
`a_popup_grab_is_refused_while_an_ime_holds_the_keyboard` demands it is
never configured -- the old walk missed it, so the index alone would
have configured it. The lookup therefore pairs the index with a
membership check over the popup's own tree (never global), plus the
walk's `alive` filter; parentless (unmapped) popups keep the walk's old
find-and-configure. Dismissal semantics byte-identical by construction,
not by enumerating Smithay's dismissal sites.

Single-client shape unchanged (same harness as PR #253): 128 admitted
for track 1.4 + commit 3.8-4.1 + destroy 0.3 ms; 500 refused in ~1.5 ms,
2000 in ~3 ms, same kill message. The per-client cap was not relitigated.

## Honest residual (not this PR)

Two Smithay-side scans stay, both verified against the pinned rev and
both linear-per-op with small constants: `PopupManager::commit`'s
`position` scan of the unmapped list, and the `xdg_popup` destructor's
`known_popups` search (~76 ms of the ~281 ms at 5120). A further index
would be a Smithay fork change (`docs/forks.md`); with no seconds-scale
stall left, that is not justified now. If connection counts ever make
the destroy-side quadratic matter, that fork index is the follow-up.

## Evidence expected

A run with many connections each at or under 128 popups showing
the stall scale with nobody disconnected.
