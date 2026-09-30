---
title: "Icons and fonts: symbol glyphs, a small fallback chain, and what is out of scope"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M3"
resolved: "2026-09-30"
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
   the [tray](../tray.md) or window icons force it.

Prefer 1 with 2 as the built-ins' default if the numbers allow, 3 as the opt-in
for user buttons; record the costs of each.

## Fonts

The rasterizer is `ab_glyph` (M0,
[the record](dependencies-done.md#1-font-rasterizer)), over a mapped
font file. Hinting is the one quality lever that choice gives up: hinted
`swash` is visibly crisper at 15 px, for about +770 KB of binary and 1 MB of
RSS. Decide here, with the fallback chain, whether 1x text needs it.

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

## Landed (2026-09-30, `feat/scootbar-icons-fonts`)

- **Fallback chain**: `bar.fallback-fonts`, at most two files, config file
  only; consulted only for a codepoint the primary lacks; a character in no
  font draws the primary's `.notdef`; a fallback that cannot load is a loud
  refusal (start-up error, or a refused reload). `src/text.rs`, `src/font.rs`.
- **Glyph cache**: was already capped (512 glyphs, 4 MiB, dropped whole past
  either); the key now carries the font. Tested under a fuzzed 3000-title
  stream at four sizes through a two-font chain
  (`text::tests::the_cache_stays_bounded_under_a_fuzzed_title_stream`). It is
  *not* dropped on scale change: it is keyed by size, and outputs at different
  scales alternate every frame, so a drop per change would rasterize on every
  frame; the bound is what keeps memory finite.
- **Icons, option 1**: `clock.icon = "..."` (exactly one character) drawn
  before the time from the chain, so a symbol font as fallback supplies it.
  Tested in units, and on headless scoot (`tests/icons.rs`).
- **Mixed Latin and CJK title**: draws with no blank and no panic
  (`text::tests::a_title_of_latin_and_cjk_draws_with_no_blank`, and the
  `.notdef` box with no CJK font). Shaping, RTL and color emoji are stated as
  out of scope in `docs/scootbar/cli.md`.

Measured (release, `lto = "fat"`, `strip`): the binary is 1,405,712 bytes at
`origin/main` and 1,413,904 with this change (+8 KiB, no new dependency);
`measure`+`draw` of a 41-character line at 21 px, glyphs cached, is
17.6 and 16.6 us against 18.2 and 18.4 us before (no regression).
Each fallback file costs its size in the heap unless it is a mapped store file
(the same rule as the primary).

## Landed, second slice (2026-09-30, `feat/scootbar-icons-fonts-2`)

The first slice is above (`0c3f3e2`: the fallback chain, the bounded glyph
cache, `clock.icon`, option 1). The remaining options are done, and this entry
is resolved. ([icons.md](../../icons.md) has the mechanism and every number):

- **Option 2, path icons**: `clock.icon-path` (with `icon-viewbox`), SVG path
  data parsed by hand (every command, bounded, fuzzed), filled by the bar's own
  analytic anti-aliased rasterizer, tinted from the state's theme token, cached
  per (icon, size) and bounded, at the output's real scale. +24.6 KB of binary,
  3.5 us to rasterize a 24 px icon, no allocation on a warm repaint. This is
  the mechanism the button, volume, network and battery modules reuse
  (`config::icon` and `View::show_icon`): **their per-module `icon` keys arrive
  with those modules**, none is invented ahead of them, and the built-in
  bitmaps or paths for mute, wifi bars and battery levels are theirs to draw.
- **Option 3, PNG icons**: `clock.icon-image` behind the **`icon-image`** Cargo
  feature, decoded once with the workspace's `png` (bounded: 8 MiB, 1024 x
  1024, regular files only), premultiplied, scaled with a triangle/area filter
  at the real scale, dropped at reload. **Off by default**: the decoder is
  +127 KB (+8.7%), which the resource ratchet does not allow for a feature a
  path icon replaces; `--no-default-features` is the smallest either way.
- **Hinting: decided, do not ship `swash`.** Measured in a throwaway copy:
  +843,768 bytes (+57%) and +0.79 MB RSS (+16.7%) for a small gain at 12 to
  16 px at 1x (the share of blurry ink pixels moves by three or four points, and
  at 15 px it rises); side-by-side images in [icons.md](../../icons.md#hinting-swash-against-ab_glyph-decided).
- **Real fonts checked**: DejaVu Sans, Symbols Nerd Font and Noto Sans CJK
  (a variable `.ttc`) together on headless scoot draw Latin, a symbol icon and
  Japanese, Korean and Chinese with no box.

## Not built

- **Option 4, icon-theme lookup**, which only the [tray](../tray.md) or window
  icons force, and it stays with them.
- **Icons in other modules**: only the clock and workspaces exist. Per-module
  `icon` keys, and the built-in paths for mute, wifi bars and battery levels,
  arrive with the button, volume, network and battery modules, through the same
  `config::icon` and `View::show_icon`.
- **A libFuzzer target for the path parser**: the stable property tests run on
  every `cargo test`; see [icons.md](../../icons.md#what-was-not-built).
