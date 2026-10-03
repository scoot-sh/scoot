---
title: "Media module: now playing, play/pause/next over MPRIS"
status: "open"
area: "scootbar"
priority: "low"
blocked: null
milestone: "M6"
---

# Media module

Filed 2026-09-29. Serves **daily-drive**.

MPRIS players (`org.mpris.MediaPlayer2.*`) on the session bus, through the
[shared D-Bus client](resolved/dbus-client-done.md): watch `NameOwnerChanged` to learn players
appear and vanish, `PropertiesChanged` for track and playback state. All
signals, no polling; playback position is deliberately not shown (it has no
change signal and would need a timer).

## What to build

Artist and title (truncated by pixel width), a state class, click for
play/pause, scroll or right-click for next/previous, a `player` config key to
prefer one when several run. Nothing shown when no player is present.

## Edge cases

A player that dies without releasing its name, several players (follow the
most recently playing), metadata with arbitrary text (bound and sanitize, as
for [window titles](window-title-module.md)), a player that reports position
constantly (ignored).

## Done when

Track changes show within a frame, controls work against two real players,
and the module costs nothing with no player.
