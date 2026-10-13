---
title: "GTK, libadwaita, Qt, cursor and fonts follow a swap through dconf and the portal"
status: "open"
area: "packaging"
priority: "medium"
blocked: "look-apply-cli"
---

# GTK, libadwaita, Qt, cursor and fonts follow a swap through dconf and the portal

Filed 2026-10-12, child of [`live-looks`](../core/live-looks.md). Serves
**daily-drive**: GTK and Qt apps are most of a desktop.

## The gap

The live knobs GTK honors are dconf keys read through the Settings portal
(GTK 4, libadwaita) or GSettings (GTK 3): `color-scheme`, `gtk-theme`,
`icon-theme`, `cursor-theme`, `cursor-size`, `font-name`. The profile writes
only `color-scheme`, at activation (`theme-home.nix:628-631`); the rest goes
to `settings.ini`, which GTK 4 under portal-gtk likely ignores for those
keys (inferred; the design doc's liveness table). `gtk.css` is never
re-read, and libadwaita's accent snaps to a named accent. qt6ct reloads its
palette when its directory changes, but its style only at restart, and
adwaita-qt may override the look's palette in widget apps altogether
(inferred from `adwaitastyle.cpp:503-506` and qtbase's
`qapplication.cpp:1089-1113`).

## What to do

1. **First, settle the Qt question** with a screenshot: does a QWidget app
   under `style=Adwaita-Dark` show the look's qt6ct palette at all today?
   If not, that is an existing bug: file it, fix it there (for example
   Fusion with the generated palette), and continue.
2. On swap, write the dconf keys above from the look (`gtk-theme` carrying
   GTK 3's dark/light variant). Through portal-gtk, GTK 4 and libadwaita
   follow live (portal-gtk `src/settings.c:266-337`).
3. `gtk.css` becomes one `@import` of the link's `gtk/gtk.css`, so new GTK
   apps take the new palette; running ones keep theirs.
4. qt6ct: `color_scheme_path` through the link; check whether a swap fires
   qt6ct's directory watcher and document the result.
5. Research step, not a promise: whether alternating between two theme
   names recolors running GTK 3 and plain GTK 4 apps
   (`gtksettings.c:1713-1722`). Ship only if it is measured working.
6. Docs: the theming page's "Takes effect" column made exact.

## Evidence to record

Screenshots before and after a swap of a running GTK 3, a GTK 4, a
libadwaita and a Qt app, with which properties changed live. The pinned
versions named (GTK 3.24.52, GTK 4.22.4, libadwaita 1.9.3, portal-gtk
1.15.3, qt6ct 0.11).

## Not in this ticket

Accent colors beyond libadwaita's named set; the GNOME portal (not in the
profile: `nixos.nix:1445`).

The design doc this implements against: [live looks](../../../dev/research/live-looks-design.md).
