---
title: "Resolve the look's font family to a file at scoot look apply, not at every open"
status: "open"
area: "scootbar"
priority: "medium"
blocked: null
---

# Resolve the look's font family to a file at scoot look apply, not at every open

Filed 2026-10-31. Serves **daily-drive**. From the review of #836 (#807).

## The gap

#836 turns a look's font family into a file by walking the font directories
(`scootui::font::family`) every time scootlaunch opens and whenever
scootnotify loads or reloads. `scoot look apply` already resolves `bar.font`
once with fc-match; the other two apps do the walk at runtime instead.

- The walk pays on every open and cannot see a fontconfig-only name (a
  family fontconfig knows by alias or from a config-added directory).
- It cannot see a look-carried font: files from the look's `[fonts.files]`
  live in the look directory, not in a font directory.
- A look-named family that is missing, with no well-known font to fall back
  to, ends in the generic "no font" error and never names the family.
- The docs say the bold face is "in the same font directory"; the code looks
  in the same directory root. Say which.

## What to do

Resolve the family at `scoot look apply` time (fc-match, as `bar.font`
does, plus the look's own font files) and render the file path into
`notify.toml` and `launch.toml`, so the apps read a path and walk nothing.
Keep `--font` and the config's `font` winning. Name the family in the
missing-font note and error. Fix the bold-face wording in the docs to match
what the code does.

## Not in this ticket

Drawing bold in scootbar, which draws none.
