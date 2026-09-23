---
title: "A deep chain of nested popups overflows the compositor's stack or freezes it — any client can crash every session — RESOLVED"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Bound popup-tree depth (client-triggerable crash / hang) — RESOLVED

RESOLVED 2026-09-23 (PR #226). The rules, and why each, are in
`crates/scoot/src/compositor/popup_parent.rs`; the tests in
`popup_parent/tests/`.

- **Cap: 64 popups per chain**, checked in `new_popup` by a walk up the
  chain that stops at 65 whatever the chain looks like (it also still
  refuses the self-parent loop). Refused as `invalid_popup_parent` on the
  popup, like #225's loop refusal.
- **The chain cannot grow after admission**, which is what makes a
  creation-time cap sound. Enforced, where the pinned Smithay does not:
  `xdg_surface.already_constructed` (per `wl_surface`: nothing refuses a
  second `xdg_surface` either), posted on the `xdg_surface`;
  `xdg_wm_base.not_the_topmost_popup` at destroy (a synchronous check in
  `popup_destroyed`, skipped during client teardown, when `client()` is
  `None`), posted on the destroyed popup's `xdg_surface`; and a third
  bypass this ticket did not list -- a popup of a parent with no live
  role object (a bare `xdg_surface`, or a popup surface whose `xdg_popup`
  is gone), whose children deepen when that parent later becomes a popup.
  Children are counted in scoot's own per-surface record (weak handles),
  xdg popups only.
- **A fourth, in Smithay's tree rather than the chains:** `try_insert`
  matches a parent node by `wl_surface` alone, dead nodes included, until
  the per-dispatch reaping. A surface made a popup again, with a child in
  the same flush, put the child under the dead node -- invisible, then
  disconnected by Smithay's own lazy `not_the_topmost_popup` -- and
  repeating it in one flush nested the *tree* without bound whatever the
  chains said. `new_popup` reaps dead nodes first on that path.
- **Layer-shell adoption needed no check:** it only ever sets a layer
  surface, which is not a popup, as the parent, so a chain is as long
  after adoption as before (tested at 64 and 65 under an adopted
  dropdown). There is no hook for it anyway: implementing
  `WlrLayerShellHandler::new_popup` double-tracks (see `handlers.rs`).
- **Real clients:** GTK 3.24 was measured, live, destroying child popups
  before parents in every path tried (click outside, Escape, picking an
  item, submenu and menubar hover-switching, and scoot's own
  `popup_done`), and it never reuses a menu's `wl_surface`. wlroots and
  mutter also enforce `not_the_topmost_popup`.

Measured (dev VM, release, the harness's 2 MB test-thread stack): on
`main` a 3000-deep chain froze the compositor for ~60 s and then drew at
~420 ms a frame, and 10000 overflowed the stack; now both are refused at
65 and the next frame takes ~0.05 ms. A frame with a 64-deep chain open
costs the same before and after (~0.21 ms).

---

The original report follows.


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
