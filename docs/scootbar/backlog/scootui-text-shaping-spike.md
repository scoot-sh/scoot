---
title: "Spike: shaping and bidi for notification bodies and app names (rustybuzz), measured"
status: "open"
area: "scootbar"
priority: "medium"
blocked: null
milestone: "M7"
---

# Spike: shaping and bidi

Filed 2026-10-12 from the [shell-apps design](../../../dev/design/shell-apps.md#text-shaping-wrapping-emoji).
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

## Not in this ticket

Color emoji (CBDT/COLR), a v1 non-goal. Line breaking beyond spaces and CJK.
