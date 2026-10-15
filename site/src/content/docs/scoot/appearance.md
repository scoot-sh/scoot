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
| `cursor_size` | integer (pixels) | `16` | The **logical** size of the built-in pointer cursor, both dimensions (clamped `4..=256`; drawn only under `--tty`). The bitmap is rasterised at `round(cursor_size × output scale)` physical pixels -- 16 at 1x, 24 at 1.5x, 32 at 2x -- so the on-screen size never changes but a high-DPI output samples native art 1:1 instead of a stretched bitmap. A real cursor theme likewise contributes the image nearest the physical size; `XCURSOR_SIZE` still exports the logical size. Where no theme is installed the cursors are Mocu, drawn by scoot itself -- see [the built-in cursors](#the-built-in-cursors) below. |
| `cursor_color` | `"#rrggbb"` / `"#rrggbbaa"` | `#fcfcfc` (Mocu white) | Fill color of the built-in cursors. The outline follows Mocu's pairing: `#222222` around a light fill, `#f9f9f9` around a dark one, both at the fill's own alpha. So the default is Mocu's white theme and `"#111111"` is its black theme. A translucent fill genuinely composites (outline and shadow follow its alpha). No effect inside a real theme's artwork. |
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
cursor_color = "#111111"   # Mocu's black theme
```

Under home-manager with Stylix, the ring and background colors and the
cursor theme and size default from the scheme instead — a value written
here always wins. The three built-in color defaults are stored as raw
floats no hex string reproduces exactly: leave a color field unset to
get the real default; only set it to change it. (The cursor color
is the exception: its default, `#fcfcfc`, is exactly representable.)

The look you picked in [the desktop profile](../desktop/index.md#pick-a-look)
writes these same keys — `settings.appearance.background_color =
"#123456"` beside a look replaces that one color and keeps the rest.

## The look layer

A look can supply any key in this table except `cursor_hide_after_ms`
and `prefer_no_csd`, plus the `[wallpaper]` section. The compositor
reads the active look's fragment and merges it under this file, key by
key, before anything is derived. A key set here always wins. An unset
key takes the look's value. Anything neither sets falls back to the
Default column above.

Two exceptions. `[wallpaper]` merges as one unit: any `[wallpaper]` key
set here keeps the whole section yours, otherwise the look's whole
section applies. And `focus_ring_inactive_width` follows the layer that
set `focus_ring_width`: setting only the active width drops the look's
inactive width, so the ring follows yours.

The reload reply lists every key of yours that overrides the look
(`shadowed`). A bad color value here falls back to the look's value for
that key, and is listed as `invalid` (and shadowing). A look fragment
with a forbidden key or a bad value is refused whole: startup runs
without it and logs, and a reload answers an error and keeps everything.
With no look active, this file behaves as before.

Where the look lives: the active look's fragment is
`$XDG_STATE_HOME/scoot/look/scoot.toml` (usually
`~/.local/state/scoot/look/scoot.toml`), or `$SCOOT_LOOK_STATE/look/scoot.toml`
when that variable names a state dir explicitly. The `look` entry itself
is a link to the rendered look, so switching looks swaps one link.
`scoot-look build <sourcedir> --out DIR` writes `scoot.toml`, `bar.toml`
and `render.json` into DIR and creates no link: pass `--out <state>/look`
to render in place, or point `<state>/look` at DIR afterwards. With Nix
the catalog install does this.
When the link is absent there is no look layer and this file behaves as
before. A dangling link or an unreadable fragment warns at startup and
refuses a reload whole, keeping everything.

## The built-in cursors

Where no cursor theme is installed (a webtop or minimal container),
scoot draws its own cursors: [Mocu](https://github.com/sevmeyer/mocu-xcursor),
Severin Meyer's "modest xcursor theme", released under CC0 1.0 (public
domain). Mocu's SVG sources are built into scoot unchanged, with their
license, and scoot rasterises them itself at each output's pixel size, as
Mocu draws them: white fill, dark outline, a faint drop shadow. The busy
spinners show the first frame of Mocu's animation. `cursor_color` picks the
fill (see above). Nothing is loaded from an image file at runtime.

With the desktop profile you rarely see these: the profile installs Mocu
as real XCursor theme files too (same vendored art, white at its native
24 px with `Mocu-Black` beside it), names it in `cursor_theme`, and
exports it for X11 apps — so theme and fallback are the same family
either way. See [the desktop profile](../desktop/index.md#app-theme).

Mocu is drawn on a 24-pixel grid, which is how scoot draws it at 24px and
on multiples of it. The default 16px is Mocu downscaled, with each shape
fitted to the pixel grid so straight edges stay one crisp pixel and no
stem or finger gap blurs away. On a high-DPI output the cursors are
rasterised natively at the physical size (`round(cursor_size × scale)`
device pixels), so edges stay sharp where a stretched logical bitmap would
go soft. Mixed-DPI sessions keep one bitmap set per output scale
(bounded, evicted when no output uses a scale); screenshots and captures
draw the same per-scale pixels the frame shows. A theme image padded
beyond its nominal size draws at that nominal size, like everything else.

![The built-in cursors](../../../assets/cursor-family.png)

Every CSS cursor name draws Mocu's cursor of the same name: `default`,
`context-menu`, `help`, `pointer`, `progress`, `wait`, `cell`, `crosshair`,
`text`, `vertical-text`, `alias`, `copy`, `no-drop`, `not-allowed`,
`grab`, `grabbing`, the eight edge resizes (`n-`, `e-`, `s-`, `w-`, `ne-`,
`nw-`, `se-`, `sw-resize`), `ns-`, `ew-`, `nesw-`, `nwse-`, `col-` and
`row-resize`, `all-scroll`, `zoom-in`, `zoom-out` and `dnd-ask`. Two names
Mocu has no file for take its own alias: `move` draws `grabbing`, and
`all-resize` draws `all-scroll`. Any name newer than these draws `default`.

> **Symptom:** the pointer is a plain arrow everywhere.
> A real cursor theme answered first: `cursor_theme` (or `$XCURSOR_THEME`)
> resolved, and the theme supplies that shape. Unset the theme to see the
> built-in cursors.
