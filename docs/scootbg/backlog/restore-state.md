---
title: "Restoring the last wallpaper at startup"
status: "open"
area: "scootbg"
priority: "medium"
blocked: null
---

# Restoring the last wallpaper at startup

- After each successful set, write the choice per output (connector name,
  or "all") to `$XDG_STATE_HOME/scootbg/state.toml`, atomically (write and
  rename).
- `scootbg daemon` restores it; `--no-restore` skips it; a missing or
  moved image falls back to the compositor's own background with a
  warning.
- `scootbg daemon --initial <values>` (what scoot passes from
  `[wallpaper]`) follows the one rule in
  [scoot-integration.md](scoot-integration.md), whichever changed last
  wins: the state file also records a fingerprint of the last `--initial`
  applied; a matching fingerprint restores state (keeping any later
  `scootbg set`), a different one applies `--initial` and records it.
- Optional follow-up, measured first: cache the scaled buffer (compressed)
  in `$XDG_CACHE_HOME/scootbg/` so restore skips decode and scale. Only
  worth it if startup measurements say decode is the slow part.
