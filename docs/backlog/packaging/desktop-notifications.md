---
title: "Desktop notifications: mako now, scootnotify later"
status: "open"
area: "packaging"
priority: "high"
blocked: null
---

# Desktop notifications: mako now, scootnotify later

Filed 2026-10-04, child 3 of `desktop-paved-path`. Serves
**daily-drive** (a desktop with no notification daemon drops password
prompts, calendar pings, low-battery warnings on the floor).

Was blocked on `fix/scoot-session-target-after-display`, merged as #431 (2026-10-04): same
`WAYLAND_DISPLAY`-before-target ordering — a notification user unit would
start too early and skip.

## The gap

`scootnotify` is a pointer only (`docs/scootbar/backlog/scootnotify.md`:
low, M7, needs the shared D-Bus client + ui crate first, maintainer
starts it). `mako` appears only in doc examples. The flake installs,
configures and starts no notification daemon.

## What to do

Fill the `desktop.notifications` slot with `mako` (lightest
well-maintained layer-shell daemon; record closure/RSS vs dunst/swaync,
say why), wired so `scootnotify` later replaces it unchanged:

- User unit bound to `graphical-session.target` (starts after the target
  fix), `layer=overlay` config so notifications show above fullscreen
  windows (compositor hides `top`-layer surfaces under fullscreen —
  `docs/protocols.md` fullscreen section; mako's default layer needs the
  override, say the exact setting).
- DND state + unread count surfaced to the bar's `push` module (the shape
  `scootnotify` promises in its pointer entry — implement the bar side of
  the contract now so the swap is daemon-only).
- Lock behavior: no notification content over the session lock (same rule
  the pointer entry states; mako's `group-by`/history handling under lock
  — say what it does).
- Edge cases: daemon crash (restart policy, queued notifications);
  fullscreen games/video; per-look theming of mako config from `look`.

Acceptance: eval pins in `nix/tests.nix`; real-login proof on the M2
(notification above a fullscreen window, nothing over the lock screen,
DND toggle from the bar); docs in `docs/nix.md`.

## Not in this ticket

Building `scootnotify` itself (pointer entry owns that); notification
portal (`xdg-desktop-portal` Notification forwarding — note if it falls out
for free, otherwise leave it).
