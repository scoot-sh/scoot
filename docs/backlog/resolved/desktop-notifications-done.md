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
module from mako's bus signals -- `PropertiesChanged` for arrivals and
mode changes, the bus name itself for restarts (re-sync on a new owner,
clear the bar when mako is gone) -- no polling -- with a
click toggling DND through `makoctl`, which needs the `push` feature).
On with the profile, each switch disable-able. The feed distinguishes
a module that is not placed yet (guidance naming it, once per event)
from a bar that is not running, and malformed `makoctl list` output
counts as empty rather than crash-looping the unit (all pinned with
stub-tool behavior tests in `nix/tests.nix`).

Decisions: mako over dunst, for fit at an acceptable weight (the
ticket prescribed mako; re-tested in the fix round, PR #441): dunst
draws on pure Wayland too -- with no config and with a minimal
`layer = overlay` config it owned the bus name and drew on the
second output while reporting the notification displayed (the earlier
"drew nothing" was captured on the wrong output) -- and dunst is
lighter (174.7 MiB closure, 43.8 MiB marginal; 172.9/41.9 Wayland-only).
What keeps mako is the feed contract: mode-based DND is exactly the
bar-toggle contract, and `makoctl list -j` gives per-notification
urgency for the bar's `urgent` class, where dunst's counts carry none
without parsing its history JSON. Stock mako's 357.4 MiB closure
(210.4 MiB marginal, almost all `wrapGAppsHook3` pulling gtk+3) does
not ship: the profile defaults to a lean mako without the GTK stack
(183.4 MiB closure, 53.3 MiB marginal, no gtk/tinysparql/cups/at-spi2/avahi
reference -- pinned in `nix/tests.nix`), with png/svg file icons proven
identical to stock on screen; themed icon names behave identically to
stock (neither resolves one in this session's theme setup). RSS with one
notification up is a wash (mako 16.5 MB, dunst 17.1 MB; lean mako 9.3 MB
empty with an 11.4 MB feed tree, 0 wakeups over 60 s idle for both).
No `on-notify` hook (the bus watcher covers arrivals, so
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
