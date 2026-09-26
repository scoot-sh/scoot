---
title: "Tests: unit, and end to end on headless scoot"
status: "open"
area: "scootbg"
priority: "high"
blocked: null
---

# Tests: unit, and end to end on headless scoot

- Unit tests for fit-mode geometry (every mode against odd sizes, portrait
  on landscape, 1×1 sources, sizes that round at fractional scales),
  state-file round trips, and request parsing.
- End to end: a script like `scripts/smoke-test.sh` that starts
  `scoot --headless --outputs 2`, runs `scootbg daemon`, sets a colour and
  an image per output, and samples pixels from `scootctl screenshot
  --output N` at known points (centre, letterbox bars, corners).
- The precedence rule end to end: every order in
  [scoot-integration.md](scoot-integration.md)'s table, plus a config
  with two per-output overrides and a changed `command` across a restart,
  where the `scootbg set` pick must survive.
- Hotplug on the headless backend if scoot can add and remove virtual
  outputs at runtime; otherwise note the gap and cover it on `--tty`.
- Fuzz the image-loading entry point with truncated and corrupt files
  (the decoders are third-party; the guard is ours).
