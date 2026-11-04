---
title: "scootlaunch click-outside follow-ups from the #803 review"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-11-04"
---

# scootlaunch click-outside follow-ups from the #803 review

Filed 2026-10-29. Serves **daily-drive**: the dismiss behavior shipped in #803
(the resolved [launch-panel-bare-desktop-click](resolved/launch-panel-bare-desktop-click-done.md));
these are the review's nits, none of them a defect in what shipped.

## The gap

Four loose ends, each checked against `main` at d9ba8ef54:

1. **No isolated test for the client half.** `scootlaunch` now exits 1 on
   any `zwlr_layer_surface_v1.closed` (`Event::Closed` in
   `crates/scootlaunch/src/panel.rs`, which sets `Exit::Dismiss`).
   `a_click_outside_the_panel_dismisses_it` (`crates/scootlaunch/tests/panel.rs`)
   cannot tell that handler from the old one: the old client also exited 1,
   through the keyboard leave that follows its own destroy. The #803
   description says as much ("Not pinned by a test"). The handler needs a test
   where `closed` arrives with no keyboard leave before it.
2. **The docs do not say an unplugged output now dismisses the panel.** The
   client used to re-create its surface once on `closed` (a retry copied from
   the bar); it no longer does, so an output unplugged under the panel closes
   it instead of moving it. The `panel.rs` comment says so; nothing under
   `site/src/content/docs/scootlaunch/` does.
3. **The resolved ticket is silent on two cases and on touch.**
   `State::dismiss_launcher_on_outside_press`
   (`crates/scoot/src/compositor/layer_shell.rs`) tests the press with
   `layer_under(&ABOVE_WINDOWS, ...) == launcher`. #803's bug-bash notes say a
   hit on the panel counts as inside, subsurfaces included, and that an
   `xdg_popup` parented to the panel "would count as outside" (scootlaunch has
   none). A surface stacked in front of the panel (for example another
   `top`/`overlay` layer surface over it) also resolves to something other
   than the launcher and so counts as outside. The ticket says neither.
   Touch is not covered either: scoot has no `wl_touch` path (tablet tip,
   IPC click and `zwlr_virtual_pointer` do reach `pointer_button`).
4. **`pointer_press_cost` has no baseline.** The bench
   (`crates/scoot/src/compositor/headless/bench/pointer.rs`) is an `#[ignore]`d
   test that prints for a human and asserts nothing. #803 recorded 190 ns
   median for the check alone and 4.49 us for a whole press and release, on
   the dev VM, release build, but only in the PR body, and with no before/after
   (the call is the only delta, so the check-alone number stands as its cost).

## Resolution (2026-11-04)

All four done, with one premise corrected:

1. **Client test.** `App::on_layer_event` now holds the layer-surface
   event handling the `Dispatch` impl used to hold inline, so a test can feed
   it `Closed` with no seat and no keyboard leave
   (`crates/scootlaunch/src/panel/closed_tests.rs`:
   `closed_alone_exits_without_a_keyboard_leave`, plus a stale `closed`
   from a destroyed role is ignored). Mutation: replacing the `Closed` arm's
   `app.exit = Some(Exit::Closed)` with a no-op fails the first test.
2. **Unplugged output.** `panel.md` and the troubleshooting page now say the
   panel exits and is not moved.
3. **Inside/outside rules.** The premise about popups was wrong. `layer_hit`
   walks `WindowSurfaceType::ALL` and the layer map's hit box is
   `bbox_with_popups` (Smithay fork `b65cbc5`, `LayerSurface::surface_under`
   and `LayerMap::layer_under`), so a popup the panel opened, even one
   hanging outside the panel's rectangle, resolves to the panel and counts as
   inside. No decision was needed. Both rules are now pinned in
   `launcher_dismiss.rs` (`a_popup_of_the_panel_counts_as_inside`,
   `a_surface_in_front_of_the_panel_counts_as_outside`) and documented in
   `panel.md` and `protocols.md`, with the touch gap.
4. **Baseline.** Recorded in `dev/benches/pointer-press/README.md` (171 ns
   check alone, 4.434 us press + release, dev VM, release, 737cec6cf). The
   bench keeps asserting nothing: a ceiling on a 171 ns figure would be
   either flaky or too loose to catch anything.

## Not in this ticket

Other exclusive surfaces and launchers (fuzzel, wofi): the namespace gate is
deliberate, see the resolved ticket. Adding a `wl_touch` path.
