---
title: "Resolve the look's font family to a file and use a real bold face (shell apps draw one font file, synthetic bold)"
status: "open"
area: "scootbar"
priority: "low"
blocked: null
---

# Resolve the look's font family to a file and use a real bold face

Filed 2026-10-30, found building #807 slice 3 (typography). Serves
**daily-drive** (a look's type should show) and the beautiful half of
"lightweight is not an excuse for plain".

## The gap

scootnotify, scootlaunch and scootbar draw with one font file each:
`--font PATH`, else the first usable entry of `scootui::font::WELL_KNOWN`
(DejaVu Sans, then Noto Sans, regular faces only). There is no fontconfig.
The look's `font` key is validated (`ValueKind::Identifier`) and kept in
`Look::font_family`, and never resolved to a file. Bold is synthetic: a run
is drawn again 1 px to the right (`view/render.rs` `paint_line`), and the
card summary's `title.weight` (#807 slice 3) maps weights onto one, two or
three strikes of that same face.

Nothing can show a real bold, a semibold, a different family for titles,
or the look's own typeface, which is why `ginger-night`'s
`DroidSansM Nerd Font Propo` reaches no card.

## What to do

A small resolver, not fontconfig: given the look's family, find a file in
the usual directories (and `/run/current-system/sw/share/X11/fonts` and the
Nix profile paths), pick the regular face, and look for a bold sibling by
file name (`DejaVuSans.ttf` and `DejaVuSans-Bold.ttf`,
`NotoSans-Regular.ttf` and `NotoSans-Bold.ttf`). Variable fonts are an
option only if the pinned `ab_glyph` exposes the axis on `FontArc`; static
files are what the well-known list holds today.

`Text` would hold an optional second face; the glyph cache key already
carries a font index. Layout must measure with the face paint draws with
(`wrap_spans`, `ellipsize` and `paint_line` all call `advance`/`measure`).
Loading the bold costs its file (about 700 KB read for DejaVu when not
mapped), so load it only when a look asks for weight above regular.

## Not in this ticket

Shaping, kerning, ligatures (see `crates/scootui/src/text.rs`); per-app
font overrides beyond what `[apps.*] font` already validates.
