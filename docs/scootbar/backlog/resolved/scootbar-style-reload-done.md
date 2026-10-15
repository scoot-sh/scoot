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

## Round 2 (2026-10-15, PR #609, commit `ee6b25977`)

Review follow-ups on top of the above: a delay-changing reload closed a
stuck tooltip (`reset_hover` closes the tip with the hover state), the
restyle remake uses the buffer-fits check (no more remake per reload at
scale 2 / fractional scales), and a reload while any module is left out
as unavailable takes the full path and retries it. The vacuous
child-keeps unit test is dropped (covered live); new integration tests
cover the font-size remake, the refused style reload with a popup open,
the delay-change tooltip, and the netns retry.

Evidence (dev VM, `scoot --headless --outputs 1`, 7 modules: clock,
push, `exec` x4, bluetooth on the system bus; binaries
`/dev/shm/sbfix2-target/debug/scoot{,bar}` at `ee6b25977`; scripts
`/dev/shm/sbev3.sh` + `/dev/shm/sbev4.sh`, copies kept at those VM
paths; JetBrains Mono ExtraLight):

- Wall time, client-observed (`date +%s%N` around `scootbar msg
  reload`), ms. Colors-only (20): 2.6, 7.9, 3.1, 4.4, 2.7, 2.5, 2.6,
  6.9, 3.1, 6.0, 4.4, 2.7, 4.4, 11.5, 12.7, 6.4, 2.8, 2.7, 3.6, 1.9
  (median 3.4). Full, height 28/30 (10): 5.3, 12.3, 11.9, 7.7, 26.0,
  24.0, 21.7, 23.6, 7.5, 2.4 (median 12.1). The VM is shared with other
  agents' builds, so the outliers are scheduling noise; the style path
  stays in single digits throughout.
- CPU: 0.00 user + 0.00 sys per reload on both paths (`time -p`,
  below its resolution).
- D-Bus reconnects: the bar's system-bus unique name (`busctl
  --system list`) is `:1.109` before, still `:1.109` after 20 style
  reloads (zero reconnects), `:1.121` after the full reloads
  (reconnected, as designed), still `:1.121` after 20 more style
  reloads. Bluetooth is in `query` throughout.
- `exec` PIDs identical across all 51 reloads on both paths; a
  `subscribe` connection survived all of them and heard a post-reload
  `set` (`sub.log` 3488 B, tail ends with the ping event).
- fds 21 to 22 (the subscriber itself), flat after. RSS 9000 KB to
  12056 (20 style) to 13640 (10 full) to 16520 (20 more style):
  `/proc/PID/maps` holds 52 JetBrainsMono mappings at the end, one per
  reload plus the start — the same pre-existing font-map ratchet as
  above, unchanged by this round.
- Allocations beyond that: not measurable with the system allocator;
  RSS/fds/maps are the reported bounds. Bar log: no errors, no
  left-out notes (all 7 modules available, so the style samples really
  took the fast path).
