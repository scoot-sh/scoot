---
title: "A style-only reload keeps every module running and repaints"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-15"
---

# A style-only reload keeps every module running and repaints

Filed 2026-10-12, split out of [`look-layer-scootbar`](look-layer-scootbar.md)
for the [`live-looks`](../../backlog/core/live-looks.md) epic, and useful on
its own. Serves **daily-drive**.

## The gap

Every reload restarts every module except an unchanged `exec`
(`crates/scootbar/src/modules/mod.rs:615-661`), so a colors-only reload tears
down and rebuilds every D-Bus client, and it closes any open popup
(`daemon/surfaces.rs:260-266`).

## What to do

A reload whose only differences are style (`[colors]`, the `[bar]` style
keys, the font) keeps every module and its state and repaints; only a
layout or module-table change restarts modules.

## Evidence to record

Before/after for a colors-only reload: wall time, CPU, allocations, D-Bus
reconnects (zero after); a module's subscription survives; the resource
ratchet unchanged.

## Not in this ticket

The look layer.

The design this implements against: [live looks](../../../dev/research/live-looks-design.md).

## Resolution (2026-10-15, PR #609)

Review follow-ups on the style-only reload, on top of `df61e2e16`.

1. The `style_only` gate is fail-closed: it resets every known style
   field on a scratch copy and requires full equality for the rest, so a
   field added to `Config` or `Layout` later takes the full reload until
   deliberately listed as style. The old allow-list (plus its backwards
   comment) would have treated a new field as style-only — never applied,
   recorded as applied. Pinned by `every_style_field_at_once_is_style_only`
   and `every_non_style_field_at_once_is_not_style_only`.
2. An open popup or tooltip is restyled, not left mixed:
   `State::restyle_popups` remeasures at the new em, lays out again and
   marks dirty; a new size or frame shape is remade at once (popup with
   its grab serial, tooltip with no new delay); only a failed remake
   closes. `State::restyle` also measures the rebuilt scenes, because the
   popup's live check reads spans and a rebuilt scene leaves them zero
   until measured — without that every style reload closed what it kept.
   Tooltips turned off close. Docs (`cli.md`, `configure.md`) say the
   real behavior instead of "leaves any open popup alone".
3. Coverage: `tests/style_reload.rs` (a colors-only reload keeps the
   `exec` child and repaints; a refused reload changes nothing and the
   next one still routes on the running bar), popup/tooltip repaint
   tests, and two gate tests proving every field's classification. The
   pre-existing popup/tooltip reload tests were updated to the real
   behavior (style-only keeps, full still closes).

Evidence (dev VM, `scoot --headless --outputs 1`, 6 modules: clock, push,
`exec` x4, `target/debug/scootbar` at this PR's head; raw numbers,
commands and SHAs are in the fix report): colors-only reloads 1.3–2.5 ms wall (20 samples, first cold
one 6.2), full reloads (height 28/30) 1.5–1.8 ms; `exec` PIDs unchanged
across 80+ reloads on both paths; a `subscribe` connection survived 18
reloads and heard a post-reload `set`; fds 20 to 21 (the subscriber
itself) and flat after. One ratchet found while measuring, pre-existing
and out of scope: RSS grows ~150 KB per reload with modules placed —
each reload maps the font file again (`map_if_read_only` returns
`&'static`, never unmapped; `/proc/PID/maps` gains exactly one
JetBrainsMono mapping per reload). The empty bar (no font load) is flat.
Main does the same on every reload (`stage` loads the font), so this is
not a regression; proposed follow-up is a mapping cache keyed by file
identity in `font.rs`/`scootbg-mem`.
