---
title: "GTK, libadwaita, Qt, cursor and fonts follow a swap through dconf and the portal"
status: "open"
area: "packaging"
priority: "medium"
blocked: "look-apply-cli"
---

# GTK, libadwaita, Qt, cursor and fonts follow a swap through dconf and the portal

Filed 2026-10-12, child of [`live-looks`](../core/live-looks.md). Serves
**daily-drive**.

## The gap

GTK's live knobs are dconf keys (portal-gtk for GTK 4 and libadwaita,
GSettings for GTK 3; the design doc's liveness table); the profile writes
only `color-scheme`, at activation (`theme-home.nix:628-631`). `gtk.css` is
never re-read. qt6ct reloads its palette on a directory change, its style
only at restart, and adwaita-qt may override the look's palette in widget
apps (inferred from `adwaitastyle.cpp:503-506`, qtbase
`qapplication.cpp:1089-1113`).

## What to do

1. **First, settle the Qt question** with a screenshot; if the palette never
   reaches QWidget apps today, file and fix that bug first.
2. On swap, write the dconf keys of section 4.7 (D11: the look tool owns
   them unless the user set one in `dconf.settings`).
3. `gtk.css` becomes one `@import` of the link's `gtk/gtk.css` (new GTK apps
   take the new palette), unless `gtk.enable` owns the path.
4. qt6ct `color_scheme_path` through the link; check whether a swap fires
   its directory watcher.
5. Research only: the GTK theme-name flip (`gtksettings.c:1713-1722`); and
   the portal `contrast` key as a follow-up for a future high-contrast look.
6. Docs: the theming page's "Takes effect" column made exact.

## Evidence to record

Before/after screenshots of a running GTK 3, GTK 4, libadwaita and Qt app,
with what changed live; the pinned versions named.

## Not in this ticket

libadwaita accents beyond its named set; the GNOME portal.

The design this implements against: [live looks](../../../dev/research/live-looks-design.md).
