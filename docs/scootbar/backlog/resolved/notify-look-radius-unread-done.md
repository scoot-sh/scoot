---
title: "scootnotify ignores the look's radius: cards draw the config file's radius"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-31"
---

# scootnotify ignores the look's radius: cards draw the config file's radius

Filed 2026-10-30, found building #807 slice 3 (card styles). Serves
**daily-drive**: a look's corners should show on the cards, as its colors do.

## The gap

`crates/scootnotify/src/look.rs` parses `radius` into `Look::radius` (and
the allowlist, the renderer and the docs table all say "Corners |
`shape.radius`"), but nothing reads that field. Every paint, the shadow's
corner tables and the stack's tests read `Config::radius`, the config
file's value (default 8): `crates/scootnotify/src/view/render.rs`
(`paint_card`, `shadow_of`) and `popup.rs` (`prepare_shadow`).

Measured live (headless scoot, `SCOOTNOTIFY_REQUIRE_SCOOT=1`, a
`notify.toml` look fragment of `radius = 40`, no config file): the card's
pixel at 5 px in and 1 px down from its corner is the full ring color
(`[249, 226, 175]`, where the desktop is `[20, 20, 25]`), which is the
default radius 8's corner, not a 40 px one. So `ginger-night`'s
`shape.radius = 14` and `radial-burst`'s 14 never reach a card.

## What to do

Make the look's radius the cards' radius, with the file winning per key
only when it sets one (`Config::radius` has to become an `Option`, or the
file's default must stop standing in for "unset"). The documented rule is
"the user's file wins per key over the look over the built-ins".

Edge cases to pin: a look with no `shape.radius` renders 0 (`resolved.rs`
defaults it), which would square every card of a look that never set one,
so the renderer should emit `radius` only when the look sets it; the
shadow's corner tables follow the new value (`card_radius`); a re-theme
that changes the radius must rebuild them; `card.shape = "pill"` keeps
winning over both.

It changes how every shipped look's cards look (8 becomes 14 on
`ginger-night`), so it wants its own PR and its own screenshots, not a ride
along with a styling slice.

## Not in this ticket

Radii above 64 (`card.shape = "pill"` is the pill); scootlaunch's panel
radius, which reads its fragment's `radius` already.

## Resolution

Landed 2026-10-31. `Config::radius` is now an `Option<u32>` (unset by
default); a card's radius is the file's key when set, else the look's
`radius`, else the built-in 8 (`Look::card_radius`). The paint, the shadow's
corner tables and the stack's tests all read that one function, and a
re-theme already marked the tables stale. `card.shape = "pill"` still wins.

One edge case was not done as written. The ticket asked the renderer to emit
`radius` only when the look sets `shape.radius`. `ResolvedLook` carries
`shape.radius` as a plain `0` for an unset one, the same `0` the bar's
`radius`, the window `corner_radius` and the launcher's panel take, so a look
that never set one is square on every surface. Emitting it only for
scootnotify would need a new "was set" field on the resolved look and would
make the cards the one rounded surface of a square look. The cards follow
the look; the site page says so.
