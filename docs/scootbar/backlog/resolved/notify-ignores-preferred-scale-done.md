---
title: "scootnotify ignores the compositor's preferred scale: cards draw a 1x buffer upscaled on a HiDPI output"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-31"
---

# scootnotify ignores the compositor's preferred scale: cards draw a 1x buffer upscaled on a HiDPI output

Filed 2026-10-31, left over from
[launch-ignores-preferred-scale](resolved/launch-ignores-preferred-scale-done.md).
Serves **daily-drive**: notifications are the other overlay a HiDPI laptop
shows all day, and their text and border are soft.

## The gap

`crates/scootnotify/src/popup.rs` binds neither `wp_viewporter` nor
`wp_fractional_scale_manager_v1` (`Globals { viewporter: None,
fractional_scale: None }`), and `wl_surface`, `wl_output` and
`wp_fractional_scale_v1` are `empty_dispatch!`, so `Demand::scale()` stays 1:
every card draws into a buffer the size of the stack's logical size and the
compositor upscales it. The site says so (`scootnotify/index.md`: "Popups
draw at scale 1 in v1").

## What to do

Feed `wl_surface.preferred_buffer_scale`, `wp_fractional_scale_v1`'s
`preferred_scale` (bound only with a `wp_viewporter`) and the `wl_output.scale`
of the outputs the surface entered to the `Demand`, as the bar and the
launcher do, and redraw when the scale to draw at changes. The layout, the
input region, the hit test and the motion geometry stay in logical pixels
(the region is in surface-local logical pixels whatever the buffer is); only
the paint and the motion layers are in device pixels, with every rectangle
mapped by its edges so the stack's far edges land on the buffer's. The
buffer is exactly `Scale::buffer(logical)`.

Share the launcher's `OutputScales` through scootui instead of copying it.

## Not in this ticket

Changing the cards' look at 1x or at an integer scale (the 1x byte
fingerprints stay as they are); icons sharper than the 96 px they decode to
(exact at 2x).

## Resolution

Landed 2026-10-31. scootnotify feeds the compositor's three scale hints to
its `Demand` and redraws when the scale to draw at changes
(`popup/scale.rs`): `wl_surface.preferred_buffer_scale` (v6),
`wp_fractional_scale_v1.preferred_scale` (the manager is only handed to the
surface together with a `wp_viewporter`) and the `wl_output.scale` of the
outputs the surface entered (`OutputScales`, moved from scootlaunch into
`scootui::density::outputs` and given a `remove` for an unplugged output).
The buffer is exactly `Scale::buffer(logical)`, and the viewport's
destination the logical size.

The layout did not move. It is, and stays, in logical pixels at every
scale, and so do the input region (`wl_region.add` rectangles are the same
at 1x, 1.25x, 1.5x, 2x and 3x), the hit test, the pose maths and the
margins. Only the paint is in device pixels, through `view::dev::Dev`: a
position maps as the protocol maps a size (`Scale::to_device`, half up, so
the stack's far edge is the buffer's), a length maps on its own (a card is
the same size wherever it sits, which the shadow's corner tables and the
motion layers rely on), and at scale 1 every mapping is the identity, with
no float. Text, the border, the stripe, the dot, the icon slot, the button
outline and the rule under a link scale with it; the icon decodes at the
slot's device size (96 px up to 2x, at most 240).

A moving card is rasterized into a layer at device size and composited at
device offsets; the poses are logical. A bottom-anchored stack is drawn as
the stack it is in whole device rows from the surface's bottom edge, so the
held height that comes and goes while cards move shifts nothing inside it
(a card lands on its moving pixels to the pixel, pinned by
`a_moving_card_is_drawn_where_the_static_card_lands`). A change of scale
while cards are open rebuilds the shadow's tables and the layers before
anything paints with them; a card leaving at that moment vanishes instead
of being repainted without the notification it pictured.

`scootui::paint::MAX_BLUR` went from 64 to 128 px: the `deep` shadow's 28 px
blur is 84 at 3x and would have been cut short, its tail shorter than the
room the layout left for it. Nothing at or under 64 px changes.
