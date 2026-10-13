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
| `cursor_size` | integer (pixels) | `16` | The **logical** size of the built-in pointer cursor, both dimensions (clamped `4..=256`; drawn only under `--tty`). The bitmap is rasterised at `round(cursor_size × output scale)` physical pixels -- 16 at 1x, 24 at 1.5x, 32 at 2x -- so the on-screen size never changes but a high-DPI output samples native art 1:1 instead of a stretched bitmap. A real cursor theme likewise contributes the image nearest the physical size; `XCURSOR_SIZE` still exports the logical size. The fallback shapes are a 22-shape family built from the CC0 Mocu cursor art and rasterised by scoot itself -- see [the fallback family](#the-fallback-cursor-family) below. |
| `cursor_color` | `"#rrggbb"` / `"#rrggbbaa"` | `#1a1a1a` (near-black) | Body fill color of the built-in pointer cursor. The rim auto-contrasts the body (dark body → white rim, light body → black rim, both at the fill's own alpha), so setting this to white reproduces the old white-pointer look. A translucent fill genuinely composites (body, rim and accent all follow its alpha). No effect inside a real theme's artwork. |
| `cursor_accent` | `"#rrggbb"` / `"#rrggbbaa"` / `"none"` | `#e57f29` (ginger orange) | The ginger mark the fallback shapes carry: the arrow's tail tip, the paw badge, the resize and split shafts, the move centre, the "not allowed" slash, the hand's cuff, and friends. `"none"` (any case) draws no mark; an unset or empty value keeps the default. Follows the body's alpha, so a translucent body never wears an opaque mark. No effect inside a real theme's artwork. |
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
cursor_size = 16
cursor_color = "#1a1a1a"
cursor_accent = "#e57f29"
```

Under home-manager with Stylix, the ring and background colors and the
cursor theme and size default from the scheme instead — a value written
here always wins. The three built-in color defaults are stored as raw
floats no hex string reproduces exactly: leave a color field unset to
get the real default; only set it to change it. (The two cursor colors
are exactly representable — `#1a1a1a` and `#e57f29` — the exceptions.)

The look you picked in [the desktop profile](../desktop/index.md#pick-a-look)
writes these same keys — `settings.appearance.background_color =
"#123456"` beside a look replaces that one color and keeps the rest.

## The fallback cursor family

Where no cursor theme is installed (a webtop or minimal container, the
everyday case), scoot draws its own 22-shape pointer family: a near-black
body, a contrasting rim, and a ginger mark where it reads. The outlines
are [Mocu](https://github.com/sevmeyer/mocu-xcursor), Severin Meyer's
"modest xcursor theme", released under CC0 1.0 (public domain) and vendored
with its license in the source tree; scoot fills Mocu's vector paths with
its own rasteriser in your colors and adds the ginger marks. The two paw
shapes, which Mocu has no art for, are scoot's own. Those paws and the
placement of the ginger marks were drawn against concept art the project
owner generated with ChatGPT; nothing is traced from it. Nothing is loaded
from an image file at runtime.

Mocu is drawn on a 24-pixel grid; at the default 16px each shape is
fitted to the pixel grid so straight edges stay one crisp pixel and no
stem or finger gap blurs away. On a high-DPI output the family is
rasterised natively at the physical size (`round(cursor_size × scale)`
device pixels), so edges stay sharp where a stretched logical bitmap would
go soft. Mixed-DPI sessions keep one bitmap set per output scale
(bounded, evicted when no output uses a scale); screenshots and captures
draw the same per-scale pixels the frame shows. A theme image padded
beyond its nominal size draws at that nominal size, like everything else.

![The fallback cursor family](../../../assets/cursor-family.png)

| Shape | Drawn for these requests |
|---|---|
| Arrow (ginger tail tip) | `default`, `context-menu` |
| Arrow + paw badge | `progress`, `alias`, `copy` |
| I-beam | `text` |
| Side I-beam | `vertical-text` |
| Pointing hand (ginger cuff) | `pointer` |
| Paw print | `dnd-ask` |
| Arrow + spinner badge (ginger spinner) | `wait` |
| Thin cross (ginger centre) | `crosshair` |
| Bold plus | `cell` |
| Double arrows (ginger shaft) | the edge resizes (`n-`, `s-`, `e-`, `w-`, `ne-`, `nw-`, `se-`, `sw-`, `ns-`, `ew-`, `nesw-`, `nwse-resize`) |
| Four arrows around a disk (ginger disk) | `move`, `all-scroll`, `all-resize` |
| Ring and slash (ginger slash) | `not-allowed`, `no-drop` |
| Arrow + "?" | `help` |
| Open hand | `grab` |
| Closed hand | `grabbing` |
| Arrows either side of a bar (ginger bar) | `col-resize`, `row-resize` |
| Magnifier (ginger plus / minus) | `zoom-in`, `zoom-out` |

> **Symptom:** the pointer is a plain arrow everywhere.
> A real cursor theme answered first: `cursor_theme` (or `$XCURSOR_THEME`)
> resolved, and the theme supplies that shape. Unset the theme to see the
> drawn family.
