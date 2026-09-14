---
title: "Custom/client cursor support \u2014 (a) DONE as item 8; (b) DONE for size and color as item 13; a theme *name* remains open and needs a real licensed asset source first."
status: "open"
area: "rendering"
priority: "low"
blocked: "blocked on a license-clean asset"
---

# Custom/client cursor support — (a) DONE as item 8; (b) DONE for size and color as item 13; a theme *name* remains open and needs a real licensed asset source first.

Custom/client cursor support — (a) DONE as item 8; (b) DONE for size and
color as item 13; a theme *name* remains open and needs a real licensed
asset source first.
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

**Still open, and deliberately scoped out of item 13: drawing a different
shape per requested theme name.** That needs either an actual cursor-theme
asset this project is allowed to ship (niri's are GPL, Adwaita's aren't
MIT-clean per `CLAUDE.md`, and nothing MIT-clean has been found or vetted
yet) or real xcursor-file loading infrastructure — a much larger feature
than a config knob, and one that is pointless without an asset to load. So
this stays blocked on sourcing a license-clean theme, not on code. Until
then `Named` draws item 13's configurable triangle whatever shape was
asked for, which is at least a visible, user-tunable pointer rather than a
wrong-shaped fixed one.
