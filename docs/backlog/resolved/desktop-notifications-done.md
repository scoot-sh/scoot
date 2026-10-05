---
title: "Desktop notifications: mako now, scootnotify later"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-05"
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

## Landed (PR #441)

Filled the `desktop.notifications` slot: mako as a `Type=dbus` user
unit (`mako.service`, `WantedBy graphical-session.target`, unending
`Restart=on-failure`, `ExecCondition` on `WAYLAND_DISPLAY`, D-Bus
activation through mako's own service file), its config on
`layer=overlay` with the DND mode section and the look's roles as mako
leaves (plus a critical-urgency ring), a `theme.targets.notifications`
opt-out, `settings` winning per key, and the bar feed
(`scoot-notify-sync`: DND state and unread count into the bar's `push`
module from mako's `PropertiesChanged` signals -- no polling -- with a
click toggling DND through `makoctl`, which needs the `push` feature).
On with the profile, each switch disable-able.

Decisions: mako over dunst (smaller closure, but X11-first: with no
config it drew nothing on the pure-Wayland session, while mako drew
immediately) and swaync (1.3 GiB GTK closure); RSS with one
notification up is a wash (mako 16.5 MB, dunst 17.1 MB), so the pick
is fit -- Wayland-only, overlay-native, mode DND, `list -j` feed --
not bytes. No `on-notify` hook (the bus watcher covers arrivals, so
every mako key stays overridable). No bus queue across restarts
(mako's history is in-memory); activation covers the gap while the
unit restarts. Portal forwarding falls out for free (forwards to the
name owner once a backend runs). No `max-history`/`group-by` options
(upstream defaults: history 5, no grouping).

Measured live on the M2 (real greetd login as scoot-test, units from
this tree's rendered files, bar from this tree): popup over a
fullscreen foot (urgent ring red), DND on and off by real bar clicks
(`makoctl mode` confirming, bar reading `DND 2`), nothing over the
session lock (queued popup held, shown on unlock), feed exact through
arrivals/dismissals/toggles, 0 wakeups over 60 s idle for all four
slot processes. Eval pins in `nix/tests.nix` (units, config content
for the look plus opt-out, bar push table, refusals, Darwin nulls);
docs in `docs/nix.md` ("Notifications", slot table, three IPC
screenshots).

Skipped cleanly: the `nh os switch` proof path (the machine's config
drifted past the brief's premise -- DMS/noctalia bare sessions with no
systemd wiring, running system three weeks old -- so a switch would
swap the login screen; proved instead through a real login plus the
unchanged launcher ordering, and the WantedBy symlinks verified);
swaync RSS (closure only: downloading 1.3 GiB to confirm the obvious
was not justified); journal playback (no persistent journal for the
test user). PR: #441.
