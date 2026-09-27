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
- **One state file per profile** (`--profile NAME`, default `default`),
  not per display: scoot binds the first free `wayland-N`, so the socket
  name shifts with start order and stale sockets and is no session
  identity. scoot passes `scoot` (or `scoot-nested`), a sway user's
  autostart can pass `sway`, and sessions with different profiles never
  restore each other's wallpapers. A daemon takes its profile from its own
  `--profile`, and adopts the profile of each `apply-config` it receives (see
  [scoot-integration.md](scoot-integration.md)), so a redundant
  `[autostart]` daemon never splits state across two profiles. Two
  concurrent sessions sharing a
  profile share state, last writer wins; that is the documented
  trade-off of a name the user controls.
- Optional follow-up, measured first: cache the scaled buffer (compressed)
  in `$XDG_CACHE_HOME/scootbg/` so restore skips decode and scale. Only
  worth it if startup measurements say decode is the slow part.
- **Measured in [ticket 8](resolved/memory-and-idle-done.md#startup),
  without restore:** a color is on screen 3.0–4.1 ms after `scootbg
  daemon` is started, but an image `set` sent straight away takes
  642–720 ms against about 440 ms on a running daemon, because it
  **decodes the file twice**: the trial finds no configured output to draw
  for, so it decodes only to validate, and the `configure` that follows
  asks for a render that decodes it again (two opens of the file, by
  strace; one when the `set` comes after the `configure`). A restore at
  login is exactly that request, so this ticket should make it one decode
  (let a trial with no configured target wait for the first `configure`
  of its outputs, bounded like the replies are) and measure startup again
  with a restored 4K JPEG.
