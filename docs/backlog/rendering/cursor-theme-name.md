---
title: "Custom/client cursor support \u2014 (a) DONE as item 8; (b) DONE for size and color as item 13; per-shape drawing DONE with wp-cursor-shape-v1; a theme *name* remains open and needs a real licensed asset source first."
status: "open"
area: "rendering"
priority: "low"
blocked: "blocked on a license-clean asset"
---

# Custom/client cursor support — (a) DONE as item 8; (b) DONE for size and color as item 13; per-shape drawing DONE with `wp-cursor-shape-v1`; a theme *name* remains open and needs a real licensed asset source first.

Custom/client cursor support — (a) DONE as item 8; (b) DONE for size and
color as item 13; per-shape drawing DONE with `wp-cursor-shape-v1`; a theme
*name* remains open and needs a real licensed asset source first.
Item 5 shipped a fixed, procedurally-generated triangle for every
`CursorImageStatus` variant — `Named` (a requested xcursor theme name) and
`Surface` (a client-supplied cursor image, e.g. a text-input I-beam or a
resize arrow) both drew the exact same shape, ignoring what was actually
requested. ~~(a) honor `CursorImageStatus::Surface` by rendering the
client's actual supplied buffer as the cursor element~~ — landed as item 8
above. ~~(b) a user/config-level override for the fallback shape's size and
color (`[appearance]`'s `cursor_size`/`cursor_color`)~~ — landed as item 13
above, which is what `Named` will always fall back to: there is no client
buffer behind a `Named` request, only a theme name.

~~**Still open, and deliberately scoped out of item 13: drawing a different
shape per requested theme name.**~~ — **largely answered from the other
direction**, 2026-09-15, by `wp-cursor-shape-v1`
(`docs/backlog/resolved/foot-protocol-warnings-done.md`). `Named` no longer
draws one triangle for every name: `cursor/shapes.rs` draws ten shapes
procedurally — an I-beam, a crosshair, the four resize double-arrows, a
four-way move arrow, a circle-and-slash — and `Shape::for_icon` maps every
`CursorIcon` onto one of them, with the arrow as the fallback for the names
none of them fits (`help`, `wait`, `progress`, `pointer`, `zoom-*`).

What remains open here is narrower than it was, and still blocked for the
same reason: **honoring a theme *name*** (`[appearance] cursor_theme`,
i.e. loading real xcursor assets so a user's chosen theme is what appears)
needs either an asset this project is allowed to ship (niri's are GPL,
Adwaita's aren't MIT-clean per `CLAUDE.md`, and nothing MIT-clean has been
found or vetted yet) or real xcursor-file loading infrastructure. That is
still a much larger feature than a config knob and still pointless without
an asset to load, so it stays blocked on sourcing a license-clean theme
rather than on code. The practical gap it leaves is now cosmetic — flexwm's
own line art instead of the user's theme — rather than functional, which is
why this drops in priority rather than closing.
