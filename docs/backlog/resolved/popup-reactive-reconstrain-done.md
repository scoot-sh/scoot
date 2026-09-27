---
title: "A reactive popup is not re-constrained when its parent moves or its output changes — RESOLVED"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Reactive popup re-constraining — RESOLVED

RESOLVED 2026-09-27 (PR pending at write time). `State::apply()` ends with
`State::reconstrain_reactive_popups()` (`crates/scoot/src/compositor/
popup_reconstrain.rs`), which recomputes `constrained_popup_geometry` for
every tracked `xdg_popup` whose committed positioner is `reactive` and, when
it differs from what is pending-or-acked, sets it pending and
`send_configure()`s -- the fresh configure pair `set_reactive` asks for. A
menu slid inside at open is re-slid when its column scrolls, the output is
resized, or a bar's exclusive zone changes. Non-reactive popups are never
touched (Smithay would answer `NotReactive`: the protocol forbids it).

- **One choke point.** Every condition change reaches `apply()` -- layout
  actions, map/unmap, output resize and add/remove, layer commits -- so one
  call there covers all of them, after the arrangement is published. The
  deferred commit flush's own `apply()` orders it correctly too: the target
  walk reads where the parent is, never mid-batch. Gated on the popup index
  being non-empty, so a popup-less `apply()` pays one length check.
- **Skipped, explicitly:** unmapped popups, dead surfaces, and popups
  dismissed out of their tree (membership walk mirroring
  `send_popup_initial_configure`'s); non-reactive committed positioners; a
  positioner newer than the acked one (an unacked `reposition` must not be
  overwritten); no known target; and anything already pending-or-acked
  (comparing against Smithay's pending-seeded state keeps a second `apply()`
  before the ack from duplicating).
- **Grabs.** A configure carries geometry, a grab routes input: a grabbed
  menu scrolled or resized past is re-constrained with its keyboard still in
  it and no `popup_done`. Pinned explicitly (below).
- **No user-facing surface:** no config, keybinding, CLI or IPC change, so
  `README.md` is untouched. No render-path change either: the committed
  geometry still feeds drawing (`drawn.rs`), now via client ack+commit as
  before.

## Evidence

All recorded on the dev VM (`ssh -p 2222 dev@localhost`, `/mnt/scoot`),
ulimit raised to 65536 for the workspace runs (the stock 1024 breaks an
unrelated `scootbg` backlog test -- see below), working tree at `4fc5a22`
plus this branch's uncommitted diff:

- `cargo test -p scoot --bin scoot compositor::popup_constraint` --
  24 passed, including the 3 new `reactive.rs` tests (ticket's pin:
  reactive `SlideX` menu re-slid on column scroll, non-reactive twin quiet,
  both twins at once).
- `cargo test -p scoot --bin scoot compositor::layer_shell::tests::popup` --
  37 passed, including `a_grabbed_reactive_popup_is_reconstrained_when_its_output_is_resized`
  and `a_dismissed_popup_is_not_reconstrained`.
- `cargo test -p scoot --bin scoot
  compositor::popup_constraint::tests::reactive` -- 4 passed, including
  `a_reconstrain_does_not_fight_an_unacked_reposition`.
- Fail-first, each new behavior against the fix disabled: the two
  scroll/resize reactive tests fail (`the compositor never sent a popup
  re-configure`), the membership test fails with the membership walk
  removed, the reposition test fails with the in-flight skip removed. The
  two non-reactive tests pass with and without the fix, as they should
  (they assert quiet).
- `cargo nextest run --workspace --no-fail-fast` -- 2457 passed, 26
  skipped, 0 failed. `cargo test -p scoot --bin scoot` (CI's runner) --
  1798 passed, 0 failed. `cargo clippy -p scoot --all-targets -- -D
  warnings` clean, `cargo fmt --check -p scoot` clean, `scripts/smoke-test.sh`
  green (22 oks, no failures).
- Pre-existing, unrelated, not touched (another agent's lane):
  `scootbg::control::tests::a_live_socket_with_a_full_backlog_is_refused_without_hanging`
  fails under the stock ssh-session `ulimit -n 1024` (its backlog-fill loop
  hits EMFILE before EAGAIN) and passes at 65536 with this diff applied.
  Pure `scootbg` unit test, no shared code with this change.

---

The original entry follows.

---
title: "A reactive popup is not re-constrained when its parent moves or its output changes"
status: "open"
area: "core"
priority: "low"
blocked: null
---

# Reactive popup re-constraining

Filed 2026-09-23 while landing
[popup constraint adjustment](../resolved/popup-constraint-adjustment-done.md),
which constrains a popup against its output at the two points the client
is told its geometry: the initial configure and `xdg_popup.reposition`.
Serves **daily-drive** (a menu left open while its column scrolls).

## What is wrong

`xdg_positioner.set_reactive` (v3) asks that "the surface is reconstrained
if the conditions used for constraining changed, e.g. the parent window
moved", answered with a fresh `xdg_popup.configure` + `xdg_surface.configure`.
scoot never re-constrains: a popup keeps the geometry it was configured
with while its parent scrolls with its column, the output is resized, or a
bar's exclusive zone changes -- so a menu slid inside the output at open
time can end up cut again. A non-reactive popup is correct as it is (the
protocol forbids re-configuring it).

In practice the window is small: opening a menu takes a popup grab, and the
things that scroll a column (focus changes, clicks elsewhere) mostly dismiss
it first. An IPC-driven scroll while a menu is open, or a hotplug, is what
would show it.

## What to do

After `apply()` (and on output/usable-area changes), for each tracked xdg
popup whose committed positioner is `reactive`, recompute
`State::constrained_popup_geometry` (`popup_constraint.rs`) and, when it
differs from the committed geometry, set it pending and `send_configure()`
(which Smithay permits exactly for a reactive positioner). This must not cost
the common `apply()` anything when no popup is open: gate it on the popup
manager having any mapped popups. Harness test: open a reactive popup with
`SlideX` near the right edge, scroll its column left by an IPC action, and
assert a second configure arrives with the re-slid position; a non-reactive
twin gets none.
