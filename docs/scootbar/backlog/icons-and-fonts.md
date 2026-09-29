---
title: "Icons and fonts: symbol glyphs, a small fallback chain, and what is out of scope"
status: "open"
area: "scootbar"
priority: "medium"
blocked: "baselines-and-spikes, config-cli-and-reload"
milestone: "M3"
---

# Icons and fonts

Filed 2026-09-29. Serves **daily-drive**. The first milestones (clock and workspaces) need
digits and a few letters; the button, volume, network and battery modules
need icons, and window titles need more than Latin.

## Options for icons, decide by measurement

1. **Glyphs from a symbol font** (Nerd Font, Material Symbols, Font Awesome),
   given as text in config: `icon = "󰕾"`. Zero new mechanism; cost is one more
   font file mapped and the config being font-dependent.
2. **Built-in bitmaps or small path icons** compiled in for the built-in
   modules (mute, wifi bars, battery levels). Theme-independent and sharp at
   any scale if drawn as vectors; costs binary size per icon.
   The same mechanism can take a user's own icon as **SVG path data**
   (`path = "M12 2 ..."`, one or a few `d` strings, not a file): a small
   hand-written path parser and the bar's own anti-aliased coverage fill,
   tinted from a theme token. Sharp at any scale and it follows Stylix, which
   is what people actually want from SVG.
3. **A PNG file path in config** (`image = "/path/launcher.png"`), for a
   button that should just be a picture. Decoded once at load with the
   `png` crate scootbg already carries, scaled to the icon size at the output's
   real scale, held as a premultiplied bitmap and dropped at reload. Behind a
   Cargo feature so the smallest build has no decoder. **No SVG files in the
   bar**: a renderer such as `resvg` is a large dependency and an untrusted-markup
   parser for a job path data does more cheaply (spike its size to confirm before
   ruling it out for good). A full-color SVG is converted to PNG ahead of time:
   the Nix module can do it in a derivation, so the bar loads only the PNG.
4. **Icon-theme lookup** (`.desktop` and themed names, needed for tray and
   window icons): loads and decodes files, which is the heavy path. Only if
   the [tray](tray.md) or window icons force it.

Prefer 1 with 2 as the built-ins' default if the numbers allow, 3 as the opt-in
for user buttons; record the costs of each.

## Fonts

- A primary font and at most one or two fallbacks, as file paths from config
  (Stylix supplies the paths; no fontconfig at runtime).
- A missing glyph draws the font's `.notdef` box, never panics or blanks the
  bar; a fallback is consulted only for a codepoint the primary lacks, and the
  result cached.
- **Out of scope at first, stated**: shaping (ligatures, complex scripts), RTL and
  bidirectional layout, color emoji. A window title in such a script renders
  per-codepoint and may look wrong; say so in the docs rather than pull in a
  shaper.
- Glyph cache bounds: cap the number of cached glyphs and drop the cache on
  scale change, so a title stream of arbitrary text cannot grow memory
  without limit.

## Done when

An icon appears in a module in the chosen way, a title with mixed Latin and
CJK draws without a panic or a missing-glyph blank, and the glyph cache is
bounded under a fuzzed title stream.
