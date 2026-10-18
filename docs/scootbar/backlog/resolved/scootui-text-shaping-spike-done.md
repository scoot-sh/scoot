---
title: "Spike: shaping and bidi for notification bodies and app names (rustybuzz), measured"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M7"
resolved: "2026-10-17"
---

# Spike: shaping and bidi

Filed 2026-10-12 from the [shell-apps design](../../../../dev/design/shell-apps.md#text-shaping-wrapping-emoji).
Serves **daily-drive**: notification bodies are arbitrary text in any language. Unblocked
(research).

## The gap

The bar draws one glyph per `char`, with no shaping, kerning, bidi or color emoji
(`crates/scootbar/src/text.rs:1-21`). Arabic, Hebrew and Indic text draws wrong.

## What to do

Behind a `shaping` feature in a throwaway copy of `text.rs`:

- `rustybuzz` 0.20.1 (MIT) for shaping. It needs `ttf-parser ^0.25` and so shares the 0.25.1
  that `ab_glyph`'s `owned_ttf_parser` already pulls in (checked by the #579 review); confirm
  in `Cargo.lock`.
- `unicode-bidi` (MIT OR Apache-2.0) for RTL runs.

Measure the `.text` and stripped size added, RSS, and the time to shape a 500-byte body.
Shaping happens once per notification, never per frame. Render a fixed corpus with and
without: Latin, Cyrillic, CJK, Arabic, Hebrew, Devanagari, and emoji with a monochrome emoji
fallback font.

## Acceptance

A recorded decision against the design's gate: shaping ships in scootnotify's default build
if it costs under about 250 KB of `.text` and under 1 ms per body. Otherwise it does not, and
the docs list the scripts that draw wrong.

## Evidence the review needs

Side-by-side screenshots of the corpus, the size and time rows with the SHA, and the license
check of every crate added (`deny.toml`).

## Result (measured 2026-10-18)

NO-GO. `rustybuzz` 0.20.1 + `unicode-bidi` 0.3.18 cost **343.4 KiB of
`.text`** (702,260 B vs 350,608 B baseline, release, dev VM aarch64) against
the gate's ~250 KiB, plus 168.4 KiB of `.rodata` tables. The 500-byte body
shapes in ~213 µs mean, well under the 1 ms budget, so time is not the
problem; size is, and the feature set is already minimal. `ttf-parser`
resolves to a single 0.25.1 shared with `ab_glyph`, as expected. Full
record with figures, raw logs, SHAs and the license table:
`dev/spikes/scootbar/text-shaping.md`.

Shaping does not ship in scootnotify's default build. Without it, Arabic
(unjoined, logical order), Hebrew (logical order) and Devanagari (no
conjuncts) draw wrong; Latin, Cyrillic, CJK and monochrome-fallback emoji
draw acceptably.

Landed as PR #656 (docs-only).

## Not in this ticket

Color emoji (CBDT/COLR), a v1 non-goal. Line breaking beyond spaces and CJK.
