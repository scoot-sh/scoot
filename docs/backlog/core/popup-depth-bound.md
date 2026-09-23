---
title: "A deep chain of nested popups overflows the compositor's stack or freezes it — any client can crash every session"
status: "open"
area: "core"
priority: "high"
blocked: null
---

# Bound popup-tree depth (client-triggerable crash / hang)

Filed 2026-09-23 by the PR #225 review. Pre-existing on `main` (measured on
`c9d50cc`'s handler). Serves **both** priorities: a compositor crash takes
every client's unsaved state with it (`CLAUDE.md` treats that like data
loss).

## What is wrong

PR #225 refuses popup *cycles*, but an acyclic chain is unbounded. Measured
on the dev VM (debug builds): with a 2 MB stack, a chain ~2000 deep
overflowed the stack **on the compositor state thread** while drawing a
frame, ~3000 deep while creating popups; with an 8 MB stack a 6000-deep
chain took ~9 s to create and ~6.4 s per frame, and 9000/12000 hit a 10 s
harness timeout. The recursion is Smithay's (`PopupNode::try_insert`,
`iter_popups_relative_to`) at the pinned rev. Release builds were not
measured — measure them.

## Why a naive ancestor-count cap is not enough

Smithay at the pinned rev does not enforce two xdg-shell errors, so a cap
that counts only a new popup's ancestors can be bypassed by re-parenting:
- `xdg_surface.already_constructed` — `get_popup` again on an `xdg_surface`
  that already has a role object;
- `xdg_wm_base.not_the_topmost_popup` — destroying a popup that still has
  child popups.

A sound fix enforces those two errors (so the tree can only grow at its
leaves and a popup's depth is fixed at creation) and then caps depth at
creation, **or** tracks depth over the whole tree. It must keep allowing
the parentless layer-shell popup (`zwlr_layer_surface_v1.get_popup` adopts
it later — see `handlers.rs` `new_popup`'s doc) and input-method popups.
Pick a cap generously above any real menu (real menus nest a handful deep;
e.g. 64) and refuse with a protocol error + disconnect, the same shape as
`popup_parent.rs`'s cycle refusal.

## Evidence expected

Fail-first harness tests: a chain past the cap is refused and a second
client keeps being served; a chain at the cap works; the two newly enforced
errors fire (and a well-behaved client — GTK submenu open/close — is
unaffected, live). Measure frame time with a max-depth tree in release. No
allocation added on the per-frame popup walk.
