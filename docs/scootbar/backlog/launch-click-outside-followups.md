---
title: "scootlaunch click-outside follow-ups from the #803 review"
status: "open"
area: "scootbar"
priority: "low"
blocked: null
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

## What to do

1. Add a test that delivers `closed` with no preceding keyboard leave and
   asserts the exit (a fake compositor, or the client's event handler driven
   directly), and mutation-check it against the old retry.
2. State in `site/src/content/docs/scootlaunch/panel.md` (or its
   troubleshooting page) that the panel closes when its output goes away,
   rather than moving to another output.
3. Add the inside/outside rules and the touch gap to the docs where the
   click-outside behavior is described (`panel.md`, and the layer keyboard
   section of `site/src/content/docs/scoot/protocols.md`). Decide first
   whether a popup parented to the panel should count as inside; today it
   does not, and nothing uses one.
4. Record the 190 ns and 4.49 us numbers with the machine and commit, where
   the repo keeps bench baselines, so a later change has something to compare
   against; or give the bench an assert with a generous ceiling.

## Not in this ticket

Other exclusive surfaces and launchers (fuzzel, wofi): the namespace gate is
deliberate, see the resolved ticket. Adding a `wl_touch` path.
