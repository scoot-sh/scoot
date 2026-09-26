---
title: "A config file, and rotating through a directory (milestone 3)"
status: "open"
area: "scootbg"
priority: "low"
blocked: null
---

# A config file, and rotating through a directory (milestone 3)

- For use outside scoot (inside scoot, `[wallpaper]` in scoot's config
  covers this): `~/.config/scootbg/config.toml` with per-output defaults,
  fit mode, filter and transition settings. The CLI keeps working without
  it. Only if people ask; the CLI plus an autostart line may be enough.
- `scootbg set DIR --every 30m [--shuffle]`: slideshow from a directory,
  with one timer, no polling of the directory.
