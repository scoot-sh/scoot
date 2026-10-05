---
title: "Desktop profile user units start in every graphical session"
status: "open"
area: "packaging"
priority: "high"
blocked: null
---

# Desktop profile user units start in every graphical session

Filed 2026-10-05. Serves **daily-drive**: anyone who keeps a second
desktop (GNOME, KDE, niri, Hyprland beside scoot) gets scoot's daemons
running inside the other session — its mako, its swayidle locker, its
clipboard history — which is user-facing harm, not just untidiness
(a foreign locker policy and a foreign notification daemon in your
other desktop).

## The gap

With the scoot desktop profile enabled, its Home Manager user units —
idle/lock `scoot-idle` and `scoot-audio-inhibit`
(`nix/modules/idle-home.nix`), notifications `mako` plus
`scoot-notify-sync` (`nix/modules/notifications-home.nix`), the two
clipboard watchers (`nix/modules/clipboard-home.nix`), and the
profile-managed `scootbar` (`nix/modules/scootbar-home.nix`,
`nix/modules/scootbar-nixos.nix`) — are `WantedBy`/`PartOf`/`After`
`graphical-session.target`. That target is shared with every desktop,
so the same user logging in to GNOME, KDE, niri or Hyprland starts
scoot's units there too.

Found by the five-desktop idle measurement (deviation (b) in the
accompanying report): the campaign had to stop the profile's units by
hand after every non-scoot login (mako kept as niri/Hyprland's
daemon; stopped for GNOME/KDE).

#431 introduced `scoot-session.target`
(`resources/systemd/user/scoot-session.target`, started by the
`scoot-session` launcher past the display import, `BindsTo`
`graphical-session.target`) for exactly this scoping — but the
profile's units were never moved onto it.

## What to do

Bind the profile's units to scoot's own session instead:
`WantedBy`/`PartOf`/`After` `scoot-session.target` for every unit the
profile owns (idle pair, mako, the notify feed, both clipboard
watchers, and the profile-managed bar — never the standalone bar,
which stays a generic `graphical-session.target` unit for other
compositors), so they start only in a scoot session and stop when it
ends. The home-manager side installs `scoot-session.target` itself
(it owns user units but no login entry), so a non-NixOS/manual scoot
setup — no launcher, no `session.enable` — still gets the target file
and starts the units with an explicit
`systemctl --user start scoot-session.target` past the display import
(documented on the desktop page).

Edge cases to pin in `nix/tests.nix`: every unit's install/part-of
target (including the audio inhibitor, the feed's `PartOf`, the
primary watcher's `After`/`PartOf` — each previously pinned only by
existence or in part); the HM target present exactly when a unit
needs it (`BindsTo` the graphical target it pulls in); the
standalone bar unchanged on `graphical-session.target`;
scootbg needs nothing (it has no unit — the compositor spawns it from
`[wallpaper] command`); the idle lock lines and the clipboard
lock-wipe keep working (they never named the target, only the lock
event). Live proof on hardware: a login into scoot starts them, a
login into another session does not.

## Not in this ticket

The launcher's 1 s idle poll (separate ticket); per-unit
`BindsTo` (stop propagation is one-way `PartOf`, deliberately —
a failing leaf must not take the session down); moving the NixOS
system halves (packages, logind, PAM: no units there).
