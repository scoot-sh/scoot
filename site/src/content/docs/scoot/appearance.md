---
title: Appearance
description: "Focus ring, background, corners, cursor, and client decorations."
---

scoot draws no titlebars by design — a focused window gets a colored
ring drawn *around* it (in the layout's own gap), and there's a solid
background behind everything. This table controls the ring, the
background, and the built-in pointer cursor. Everything here re-applies
live by `scoot msg reload`.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `focus_ring_width` | integer (pixels) | `3` | Ring thickness around the focused window, drawn around what the window actually draws (a window smaller than its slot is ringed where its content ends). Clamped to at most half of `gap`. |
| `focus_ring_inactive_width` | integer (pixels) | unset (same as `focus_ring_width`) | Ring thickness around every window that is not focused. `0` draws no ring at all on unfocused windows. |
| `focus_ring_active_color` | `"#rrggbb"` / `"#rrggbbaa"` | `#6ba6fa` (accent blue) | Ring color around the focused window. |
| `focus_ring_inactive_color` | `"#rrggbb"` / `"#rrggbbaa"` | `#595961` (muted gray) | Ring color around every other window. |
| `background_color` | `"#rrggbb"` / `"#rrggbbaa"` | `#141419` (near-black) | Cleared behind all window content — the frame clear color; what shows wherever [wallpaper](../scootbg/index.md) shows nothing. |
| `corner_radius` | integer (pixels) | `0` (square) | Window corner radius in logical pixels. Rounds window content and ring together; clamped per window to half its smaller dimension. Costs a little per frame when non-zero. Popups stay square. |
| `cursor_size` | integer (pixels) | `16` | Both dimensions of the built-in pointer cursor (clamped `4..=256`; drawn only under `--tty`). Also picks which size is taken out of a real cursor theme. |
| `cursor_color` | `"#rrggbb"` / `"#rrggbbaa"` | `#ffffff` (white) | Fill color of the built-in pointer cursor (1px black outline, not separately configurable). No effect inside a real theme's artwork. |
| `cursor_theme` | string | unset | Which installed xcursor theme named cursor shapes are drawn from. Unset follows `$XCURSOR_THEME`, then `default`. A name matching nothing installed is not an error: named shapes come from scoot's own drawn set. |
| `cursor_hide_after_ms` | integer (milliseconds) | `0` (never hides) | How long the pointer sits still over a covering fullscreen window before scoot hides it. While hidden, frames carry no cursor elements, so a fullscreen window can scan out directly even where the pointer cannot ride a plane of its own (no cursor plane and no free overlay, the pointer at a screen edge, a client's shared-memory cursor image; see [backends](./backends.md#which-renderer-draws-the-frames)). Next motion, button or scroll shows it again. Never arms behind the session lock. |
| `prefer_no_csd` | boolean | `true` | Answer decoration requests with `ServerSide`, so a well-behaved client stops drawing its own titlebar (which would double up with the ring). |

```toml
[appearance]
focus_ring_width = 4
focus_ring_inactive_width = 2
focus_ring_active_color = "#ffaa00"
focus_ring_inactive_color = "#333333"
background_color = "#101014"
corner_radius = 8
```

Under home-manager with Stylix, the ring and background colors and the
cursor theme and size default from the scheme instead — a value written
here always wins. The three built-in color defaults are stored as raw
floats no hex string reproduces exactly: leave a color field unset to
get the real default; only set it to change it. (`cursor_color`'s
`#ffffff` is exactly representable, the one exception.)

The look you picked in [the desktop profile](../desktop/index.md#pick-a-look)
writes these same keys — `settings.appearance.background_color =
"#123456"` beside a look replaces that one color and keeps the rest.
