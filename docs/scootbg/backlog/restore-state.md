---
title: "Restoring the last wallpaper at startup"
status: "open"
area: "scootbg"
priority: "medium"
blocked: null
---

# Restoring the last wallpaper at startup

- After each successful set, write the choice per output (connector name,
  or "all") to `$XDG_STATE_HOME/scootbg/`, atomically (write and
  rename).
- `scootbg daemon` restores it; `--no-restore` skips it; a missing or
  moved image falls back to the compositor's own background with a
  warning.
- The state file also records the fingerprint of the last section
  applied through `scootbg apply-config`, which is how the rule in
  [scoot-integration.md](scoot-integration.md) (whichever you changed
  last wins) survives restarts. Only `apply-config` writes it.
- **One state file per display**, named like the socket (from the final
  component of `WAYLAND_DISPLAY`), so a nested session and the host, or a
  scoot and a sway session, never restore each other's wallpapers.
- Optional follow-up, measured first: cache the scaled buffer (compressed)
  in `$XDG_CACHE_HOME/scootbg/` so restore skips decode and scale. Only
  worth it if startup measurements say decode is the slow part.
