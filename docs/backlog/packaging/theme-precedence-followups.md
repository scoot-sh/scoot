---
title: "Theme follow-ups: upstream gtk/qt win, QT_PLUGIN_PATH composes, bar font from the look"
status: "open"
area: "packaging"
priority: "high"
blocked: null
---

# Theme follow-ups: upstream gtk/qt win, QT_PLUGIN_PATH composes, bar font from the look

Filed 2026-10-06. Serves **daily-driving**: a user who themes GTK/Qt
through home-manager's own modules (or exports `QT_PLUGIN_PATH` for an input
method) currently gets an eval error or silently unthemed Qt apps the moment
they adopt the desktop profile with a look.

## The gap

Review of PR #474 (non-blocking findings 1-3) established, against
home-manager master source (`modules/misc/gtk/gtk3.nix`, `gtk4.nix`,
`modules/misc/qt/default.nix`, `modules/home-environment.nix`,
`modules/systemd.nix`):

1. `nix/modules/theme-home.nix` guards only the four app files
   (foot/starship/helix/btop) with `upstreamOwns`. A user with
   `gtk.enable` (writes `xdg.configFile."gtk-3.0/settings.ini".text` and
   `"gtk-4.0/settings.ini".text`) or `qt.enable` + `qt6ctSettings`
   (writes `xdg.configFile."qt6ct/qt6ct.conf".source`) beside the profile
   gets a duplicate-definition eval error instead of graceful precedence.
   Same for the dconf `color-scheme` leaf when `gtk.gtk3.colorScheme`
   is set.
2. `QT_PLUGIN_PATH` is a lone `lib.mkDefault` in
   `systemd.user.sessionVariables`: a user's own value wins and the
   theme's qt6ct/adwaita-qt6 plugin dirs are silently dropped, so Qt
   apps fall back to unthemed fusion. Home-manager composes path-like
   shell variables with `home.sessionSearchVariables` (prepends the new
   dirs, keeps `$VAR` via `${VAR:+:}$VAR` — see `prependToVar` in
   `modules/lib/shell.nix`); the systemd manager env has no such
   composition (HM's own qt module joins a plain string there too).
3. `nix/modules/scootbar.nix` builds the bar font from a hardcoded
   `pkgs.nerd-fonts.droid-sans-mono` while `theme.fonts.uiPackage` is
   the overridable package for the same face: a future look that
   changes `fonts.ui` (or a user `uiPackage` override) rebuilds the
   bar font from the wrong package.

## What to do

1. Extend `upstreamOwns` to the GTK, Qt and dconf writes (upstream
   HM module owns the path → theme stays out, user's file wins).
   Pin: `gtk.enable` and `qt.enable` (+ `qt6ctSettings`) beside the
   profile and a look evaluate cleanly, user's file winning.
2. Publish the theme's plugin dirs through
   `home.sessionSearchVariables.QT_PLUGIN_PATH` so shells compose them
   with a user's own value; keep the `mkDefault` manager-env value for
   the no-user case. Pin both cases (with and without a user value).
3. Thread `theme.fonts.uiPackage` through the bar font instead of the
   literal. Pin: a user `uiPackage` override reaches the bar's font file.
4. Docs: the desktop page's App theme section and the theming page get
   the exact precedence for the upstream HM modules and for
   `QT_PLUGIN_PATH`.

## Not in this ticket

Live greeter/login proof (eval-level behaviors; no session needed);
changing what any look looks like; per-key dconf merging beyond the
`color-scheme` leaf.
