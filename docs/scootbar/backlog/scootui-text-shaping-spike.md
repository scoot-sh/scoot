---
title: "Spike: shaping and bidi for notification bodies and app names (rustybuzz), measured"
status: "open"
area: "scootbar"
priority: "medium"
blocked: "maintainer go-ahead on the shell-apps design (dev/design/shell-apps.md, Decisions needed 1)"
milestone: "M7"
---

# Spike: shaping and bidi

Filed 2026-10-12 from the [shell-apps design](../../../dev/design/shell-apps.md#text-shaping-wrapping-emoji).
Serves **daily-drive**: notification bodies are arbitrary text in any language.

## The gap

The bar draws one glyph per `char`: no shaping, kerning, bidi or color emoji
(`crates/scootbar/src/text.rs:1-21`). Arabic, Hebrew and Indic text draws wrong.

## What to do

Behind a `shaping` feature in a throwaway copy of `text.rs`: `rustybuzz` (0.20.1, MIT) for
shaping and `unicode-bidi` (MIT OR Apache-2.0) for runs. Check whether `rustybuzz` shares
`ttf-parser` with `ab_glyph`'s `owned_ttf_parser` 0.25.1 (one parser in the binary or two).
Measure `.text` and stripped size added, RSS, and the time to shape a 500-byte body (once
per notification, never per frame). Render a fixed corpus (Latin, Cyrillic, CJK, Arabic,
Hebrew, Devanagari, emoji with a monochrome emoji fallback font) with and without.

## Acceptance

A recorded decision against the gate in the design: shaping ships in scootnotify's
default build if it costs under about 250 KB of `.text` and under 1 ms per body;
otherwise not, and the docs list the scripts that draw wrong (Decisions needed 7).

## Evidence the review needs

Side-by-side screenshots of the corpus, the size and time rows with the SHA, and the
license check of every crate added (`deny.toml`).

## Not in this ticket

Color emoji (CBDT/COLR) — a non-goal for v1. Line breaking beyond spaces and CJK.
