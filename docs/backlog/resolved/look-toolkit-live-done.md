---
title: "GTK, libadwaita, Qt, cursor and fonts follow a swap through dconf and the portal"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-15"
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

## Resolution (2026-10-15)

Shipped in PR #630 (non-Nix path first; no `theme-home.nix` change —
the Nix `@import` switch and the D11 Nix key-name list stay for
`look-nix-catalog`).

What landed: `apply` renders per-version toolkit fragments
(`gtk/gtk-3.0.css`, `gtk/gtk-4.0.css`, `qt/scoot-look.conf`) into every
cached render and owns the full toolkit step — six dconf keys plus six
owned config files through the state link, no resident daemon.
`look.json` carries `dconf`/`dconf-orig` maps (legacy `color-scheme`
fields migrate on read); `toolkit off` restores every owned key.

Evidence (dev VM, NixOS 26.11 aarch64, pinned versions named per shot):

- Qt question settled: under `style=Adwaita-Dark` a QWidget app resolves
  Adwaita's fixed palette (window `#ff353535`, highlight `#ff15539e`)
  for a vivid custom scheme the same app resolves exactly under Fusion
  (qtbase 6.11.2, qt6ct 0.11, adwaita-qt 1.4.2, PyQt6 6.11.0). Filed
  `qt-adwaita-palette.md`; the look still drives style variant, icons
  and fonts. In-session Qt under moonrise shows the fixed dark UI.
- qt6ct watcher: an in-place scheme rewrite fires nothing; a new scheme
  file plus conf rewrite re-palettes a running Fusion app (~3 s
  debounce); deleting the scheme falls back to default. Swaps ship the
  stable link path (next-launch for running Qt apps).
- GTK 3 (3.24.52): the combined css dies at `:root` (`Theme parsing
  error: gtk.css:18:1`); user `@define-color theme_*` overrides have no
  effect on its embedded Adwaita (selection `#15539e`, base `#2d2d2d`
  sampled). The shipped `gtk-3.0.css` carries only direct selection
  rules, proven amber (`#FFA45C`) live on `gtk3-widget-factory`;
  `* { color }` proves the file loads.
- libadwaita (1.9.3): the full css through the `file://` `@import`
  paints amber buttons on slate (`#FFA45C` on `#2B3648` sampled).
- Plain GTK 4 (4.22.4): `settings.ini` dark/light follows for new apps;
  the accent stays stock blue in every css variant tried (defines,
  vars-only, direct). Running apps keep everything (no portal runs).
- dconf: all six keys written and read back on dark and light looks;
  `toolkit off` resets to unset; per-key ownership verified (a foreign
  key is left alone while the other five rewrite).
- Suites: `cargo clippy --workspace --all-targets -D warnings` clean on
  Linux; `nextest`/`cargo test` for `scoot-look`+`scootctl` green
  (288); `nextest -p scoot look` green (23, incl. the real-daemon swap);
  CI green except the features job (no feature-gated code touched).

Deviations: font sizes use `fonts.app-size` (Nix hardcodes 11);
theme-name flip tested (no live change — restart needed); portal
`contrast` unverified (no portal on the VM); per-swap scheme churn for
live Qt deliberately not shipped.

Handoff for `look-nix-catalog`: `sync` must carry the `dconf_wrote` /
`dconf_orig` maps in every record it writes (see `apply` and rollback);
the Nix side adopts these renderers and the per-version shims when it
lands.
