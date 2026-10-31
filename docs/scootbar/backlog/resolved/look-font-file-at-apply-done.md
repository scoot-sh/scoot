---
title: "Resolve the look's font family to a file at scoot look apply, not at every open"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-31"
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

## Resolution

Landed 2026-10-31. `scoot look apply` resolves `fonts.ui` once (fc-match
with an exact `%{family}` match, as `bar.font` already did, plus the look's
own `[fonts.files]` entries) and renders the file as `font-file` into
`notify.toml` and `launch.toml`; the apps try `--font`, then `font-file`,
then the family walk, then the well-known files. `RENDER_VERSION` 14 -> 15
so cached renders pick the key up. A missing family is named in the apply
notes and in the app's stderr note and error. Bold with a resolved file is
the `-Bold` sibling in the same folder; the font-directory-root-wide search
applies only to the family walk.

Evidence (dev VM, CPU tier): a user look with `fonts.ui = "Space Mono"`
(a font only fontconfig knows, via a scratch `FONTCONFIG_FILE` dir, absent
from every `font_dirs()` dir) applies with `font-file` in both fragments
on the branch and without it on main. Before: both apps fail headless
(`no font: none of the usual font files is usable`), empty screenshots.
After: the notify card and the launcher panel draw in Space Mono, silent
logs. At launcher open, strace shows the walk (26 `openat`, 4
`getdents64`, including the `~/.fonts` open and 8 well-known misses) reduce
to a single open of the resolved file.
