# Live looks: one theming contract for the whole scoot desktop (design, 2026-10-12)

Research and design only. No code changed with this document. The
implementation is the epic [`live-looks`](../../docs/backlog/core/live-looks.md)
and its children. The question came from the maintainer: "Are we able to
live swap looks? I was thinking it'd be really cool to build out looks and
be able to swap between them, maybe even with a tool." The coordinating
session raised the bar to an integration architecture across every
first-party app, with Stylix as a first-class source.

Why this lives in `dev/research/`: `dev/README.md` says `dev/` holds the
material written for maintainers and reviewers (research notes, spikes),
never deployed. The user reference stays on the site, and each
implementing ticket updates it in its own PR. The backlog carries the
tickets; this file carries the evidence and the decisions they implement
against.

Every `file:line` below is against `origin/main` at `13173a78b`, unless a
row names an upstream source and version. Short paths are relative to the
crate that owns them: `reload.rs`, `config.rs`, `shell.rs`, `state.rs`,
`sighup.rs`, `ipc.rs` and `wallpaper.rs` to `crates/scoot/src/compositor/`;
`daemon/`, `modules/`, `control/`, `config/`, `theme.rs` and `dbus/` (bar
rows) to `crates/scootbar/src/`; scootbg rows to `crates/scootbg/src/`.

## Contents

1. [Today: what a look is](#1-today-what-a-look-is)
2. [Liveness, component by component](#2-liveness-component-by-component)
3. [The token schema](#3-the-token-schema-what-a-look-is-as-data)
4. [The runtime: one indirection, one reload protocol](#4-the-runtime-one-indirection-one-reload-protocol)
5. [Stylix: a source of a look, not a layer above it](#5-stylix-a-source-of-a-look-not-a-layer-above-it)
6. [Nix and non-Nix parity, and the home-manager rule](#6-nix-and-non-nix-parity-and-the-home-manager-rule)
7. [The tool and the integration surface](#7-the-tool-and-the-integration-surface)
8. [Authoring, sharing and the trust boundary](#8-authoring-sharing-and-the-trust-boundary)
9. [The contract for scootlaunch, scootnotify and scootgreet](#9-the-contract-for-scootlaunch-scootnotify-and-scootgreet)
10. [Order of implementation](#10-order-of-implementation)
11. [Risks and non-goals](#11-risks-and-non-goals)
12. [Decisions needed](#12-decisions-needed)
13. [Deferred from the review](#13-deferred-from-the-review)
14. [What this could not verify](#14-what-this-could-not-verify)

## 1. Today: what a look is

A look is chosen when Nix evaluates the configuration, and nothing switches
it afterwards.

- **The registry is Nix data.** `nix/modules/desktop.nix:1931-2147` lists
  five looks. Each has `appearance` (3 colors), `barColors` (6 roles;
  radial-burst has no `hover`, 2032-2038), `nightTemp`, `isDark`,
  `fonts{ui,sans,mono}`, `appFiles` (static repo files) and `wallpaper`.
  The name is an `enum` (195-204), defaulted to `ginger-night` at
  `mkDefault` (`home.nix:503-505`, `nixos.nix:2235-2237`). The one
  reading helper is `nix/modules/theme-look.nix` (`lookFor`, `themed`,
  `appColors`; cursor fixed at Vanilla-DMZ 24, icons Adwaita, 61-73).
- **Three ways a look lands, all read-only.**
  - Module-priority leaves merged into the compositor's and the bar's
    TOML. `home.nix:532-539` puts the look's `appearance.*` and
    `wallpaper.*` at `mkOptionDefault`; `scootbar.nix:374-378` does the
    same for `colors.*`.
  - Generated or static files linked from the store: GTK
    `settings.ini`/`gtk.css`, qt6ct, fontconfig, foot, starship, Helix,
    btop (`theme-home.nix:602-678`), mako (`notifications-home.nix:316`),
    swaylock (`idle-home.nix:373`), wob (`audio-home.nix:417`), fcitx5
    (`inputmethod-home.nix:203-204`).
  - Store paths baked into scripts and units: fuzzel flags inlined into
    `scoot-launcher` and the pickers (`keys-home.nix:72-79,139-141`,
    `apps-home.nix:67-73`), swaylock run as `-C ${swaylockConfig}`, a
    store path (`idle-home.nix:83`), the night temperature in the unit's
    arguments (`nightlight-home.nix:247,260-261`).
- **The example directory is not the registry.** `docs/examples/<look>/`
  holds full `scoot.toml` and `bar.toml` files that mix the look (colors,
  radius, ring widths, gap, wallpaper) with a layout (module lists, icons,
  `exec` commands, buttons: `docs/examples/moonrise/bar.toml:19-21,58-86`).
  The Nix path applies only the 3+6 colors, the fonts and the wallpaper. No
  check compares the registry with the example files.
- **install.sh copies whole files once.** `--look` downloads the example's
  `scoot.toml` and `bar.toml` to `~/.config/scoot/` and never writes over an
  existing file (`site/public/install.sh:411-422,424-462`), so it cannot
  switch the look on an install that already has one.
- **A published claim with no check behind it.** `theming.md:208-209` says
  "a check renders every look into every target and fails on a missing
  role". `nix/tests.nix` has five hand-written evaluations
  (`hmTheme*`, 3940-3968) and an AA contrast check over the generated
  `gtk.css` only (14907-14932: text ≥ 4.5, text on accent ≥ 4.5, accent
  swatch ≥ 3.0). Nothing iterates `looks`.
- **A CI gap that matters for looks.** A change under `docs/` runs no CI
  job except the ReGreet card-scope gate (`.github/workflows/ci.yml:236-246`:
  `*.md | docs/* | dev/*) ;;`). The Nix modules read `docs/examples/*`
  (`desktop.nix:1977-1983` and similar), so a look-only change that breaks
  a module check merges without `flake check` ever running.

## 2. Liveness, component by component

"Live" means a running instance changes with no restart. "Next launch"
means the next process started picks it up. "Restart" means a long-lived
process must be restarted.

### First-party

| Component | Today | Mechanism | Evidence |
|---|---|---|---|
| scoot `[appearance]` ring widths and colors, background, `corner_radius` | **live** | `scoot msg reload` or `SIGHUP`; validated whole before anything is touched, then diffed field by field | `reload.rs:23-33,371-404`; SIGHUP `sighup.rs:1-16` |
| scoot cursor (`cursor_size`, `cursor_color`, `cursor_theme`) | **live** for the compositor-drawn cursor; children spawned after the reload get the new `XCURSOR_*` | `Cursor::rebuild` in place; `State::spawn` exports the live theme per child | `reload.rs:411-455`; `state.rs:1850-1851` |
| scoot `[layout] gap` | **live**, but a full re-arrange | `World::set_config`, then `apply()`, which re-arranges and reconfigures every window | `reload.rs:343-364,308-321`; `shell.rs:375-405` |
| scoot ring or background change | **live**, and also triggers `apply()` | the same `apply()` gate lists ring and background fields, not only placement ones | `reload.rs:308-321` |
| scoot `[wallpaper]` | **live**, fire-and-forget | handed to `scootbg apply-config`, "applied meaning handed over, never waited on" | `reload.rs:125-133`; `wallpaper.rs:155-243`; queue `wallpaper/queue.rs:1-37` |
| scoot `[autostart]`, `[binds]`, `[[window_rule]]` | live (spawn delta, swapped binds) | unaffected by a look by design (§3.4), and never applied by a swap (§4.4) | `reload.rs:68-76,101-123` |
| scoot `[tty] gpu`, `[renderer]`, `[xwayland] enabled`, `[virtual_input]` | **restart**, refused by name | fixed before the first frame | `reload.rs:77-90,703-760` |
| scoot bad color inside a valid file | **silently half-applied** | per-field fallback to the default with a warning, also on reload | `config.rs:194-206,748-753` |
| scootbar `[colors]`, `[bar]` style and geometry, fonts, modules | **live**, through the control socket only | `scootbar msg reload`; validated whole, font loaded, then swapped; a geometry change destroys and recreates the layer surface | `daemon/mod.rs:370-399`; `respond.rs:177-282`; `surfaces.rs:244-311` |
| scootbar reload cost | a colors-only reload is not cheap | every module restarts on every reload (D-Bus clients included), except an unchanged `exec`; any open popup closes | `modules/mod.rs:615-661`; `surfaces.rs:260-266` |
| scootbar SIGHUP | **kills the daemon** | default action kept: `#![forbid(unsafe_code)]` rules out a handler | `crates/scootbar/src/daemon/mod.rs:27-35`; `site/.../scootbar/cli.md:437-440` |
| scootbar file watch | none | inotify used only to wait for bus and socket directories | `dbus/link.rs:5,303-307` |
| scootbg image or color | **live** | `scootbg apply-config` over its control socket; decode on a worker thread; the old wallpaper stays until the new pixels land; returns once on screen | `crates/scootbg/src/daemon/worker.rs:1-20`; `daemon/images.rs:20-31`; `cli.rs:470-471` |
| scootbg swap cost | 4K, 6000x4000 JPEG: 479 ms latency, 448 ms CPU, 101.3 MiB peak PSS; to a color: 18 ms | measured | `dev/benches/scootbg/2026-09-28-scoot-peak/table.md:6-12` |
| scootbg `fade` at 4K | 21.6 ms per frame: drops frames | measured | `site/.../scootbg/transitions.md:110-118` |
| scootbg corrupt image via `apply-config` | **not kept-last-good** | not a trial: a missing file shows the compositor background, a corrupt one fails when drawn | `daemon/config.rs:29-40,173-199`; `daemon/restore.rs:20-22` |

### Third-party (versions from the pinned nixpkgs `8ce4ef6c`)

Read from each package's source as fetched with `nix build <pkg>.src`
against `8ce4ef6c`, or from its man pages. Rows marked *inferred* were
not read end to end and need a live check before a ticket relies on them.

| Component (version) | Today | Mechanism | Evidence |
|---|---|---|---|
| foot 1.28.0, palette | **restart** for a new palette; **live** only between the two palettes loaded at start | `SIGUSR1` → `[colors-dark]`, `SIGUSR2` → `[colors-light]`; to `foot --server` it switches every client. `foot.ini` is never re-read; no portal or color-scheme following | `main.c:55-73`; `terminal.c:4782-4818`; foot(1) "Signals"; footclient(1):204-210 |
| foot, any color per terminal | **live**, per pty | OSC 4/10/11/12/17/19 written to that terminal; a later OSC 104/110/111 or `SIGUSR` resets to the config | foot-ctlseqs(7); `osc.c:1400-1610` |
| GTK 4.22.4 (plain GTK) | **live** through the Settings portal: `gtk-theme`, `icon-theme`, `cursor-theme`/`size`, `font-name`, `color-scheme`, contrast | gdk on Wayland reads `org.gnome.desktop.interface` and `org.freedesktop.appearance` from the portal and follows `SettingChanged`; no direct GSettings fallback | `gdk/wayland/gdksettings-wayland.c:268-308,391-417,432-516`; `gtk/gtksettings.c:1120-1128,1345-1378` |
| GTK 4 without a portal | next launch | `settings.ini` read at init only | `gdksettings-wayland.c:521`; `gtksettings.c:284-301` |
| GTK 4 `settings.ini` under portal-gtk | *inferred:* shadowed by dconf | portal values outrank `settings.ini`'s for every key the portal maps; `gtk-application-prefer-dark-theme` is not mapped and survives | `gdksettings-wayland.c:592-605`; `gtksettings.c:1932-1953` |
| GTK 3.24.52 | **live** via direct GSettings (unsandboxed): `gtk-theme`, `icon-theme`, `cursor-theme`, `font-name`. No `color-scheme`: dark/light means switching `gtk-theme` | `g_settings_new` + `"changed"` | `gdk/wayland/gdkscreen-wayland.c:526-531,722,831-839` |
| user `gtk.css` (GTK 3 and 4) | **restart** | loaded once into a static provider, no file monitor | gtk4 `gtksettings.c:1130-1155`; gtk3 `gtksettings.c:1881-1892` |
| libadwaita 1.9.3 | **live:** color-scheme, contrast, accent. **Restart:** the CSS-variable palette | `AdwStyleManager` follows the portal's `org.freedesktop.appearance`, falling back to GSettings. The accent snaps to the nearest *named* accent, so a look's exact hex is impossible; it pins `gtk-theme-name=Adwaita-empty` | `adw-settings-impl-portal.c:125-141,154-186`; `adw-settings.c:266-298`; `adw-style-manager.c:496` |
| xdg-desktop-portal-gtk 1.15.3 | **live** | watches `org.gnome.desktop.interface`; a `color-scheme` change is re-emitted as `org.freedesktop.appearance color-scheme`. **No accent-color** | `src/settings.c:266-296,300-337` |
| xdg-desktop-portal-gnome 50.0 | live, accent included | not in the profile: `nixos.nix:1445` routes Settings to `gtk` | `settings.c:274,412` |
| qt6ct 0.11 palette and icons | **live, qualified** | a `QFileSystemWatcher` on the *directory* `~/.config/qt6ct`, debounced 3 s, re-reads `qt6ct.conf` and the scheme; only a directory-entry change fires it (`colors/` is not watched) | `qt6ctplatformtheme.cpp:59-69,151-231,235-248` |
| qt6ct `style=` (Adwaita / Adwaita-Dark) | **restart** | read again but applied only at startup | `qt6ctplatformtheme.cpp:122,240` |
| Qt widgets under adwaita-qt6 1.4.2 | *inferred, a possible existing bug:* the look's qt6ct palette may never reach QWidget apps, at launch or live | adwaita-qt's `polish(QPalette&)` replaces the palette with its fixed one, and qtbase 6.11.2 calls it after the platform-theme palette | `adwaitastyle.cpp:503-506`; `qapplication.cpp:1089-1113` |
| Cursor | **live** for every client using `wp_cursor_shape_v1` (GTK 3/4, foot, fuzzel, Qt 6.11): scoot draws the shape from its own theme. Clients without it read `XCURSOR_*` at start | scoot offers the protocol (`state.rs:1244`) and rebuilds its theme on reload | gtk4 `gdkdevice-wayland.c:352-372`; foot `wayland.c:1534-1547`; `reload.rs:411-455` |
| fontconfig 2.18.2 | lazy-live | rescans at most every 30 s, on the next font lookup, by conf and dir mtimes; GTK re-inits on the portal's fontconfig serial | `fccfg.c:275-309`; `fonts.conf.in:94-98`; portal-gtk `src/fc-monitor.c` |
| mako 1.11.0 | **live** | `makoctl reload` re-parses and restyles notifications already on screen | `dbus/mako.c:302-330,354-366`; `config.c:993-1020` |
| fuzzel 1.14.1 | next launch | short-lived; no reload | (no signal handling in source) |
| wob 0.16 | **restart** (a live trick exists) | config read once; each input line may name a `[style.X]`, so one `wob.ini` holding every look's style switches per message | `main.c:89`; `wob.c:632-660`; wob.ini(5) |
| swaylock 1.8.6 | next launch | per lock; `SIGUSR1` unlocks | swaylock(1) SIGNALS |
| starship 1.26.0 | next prompt | a new process each prompt loads the config | `src/context.rs:138,522` |
| Helix 25.07.1 | **live** | `:config-reload` or `SIGUSR1` re-reads config and theme from disk | `helix-term/src/application.rs:389-431,520-523` |
| btop 1.4.7 | **live** | `SIGUSR2` (or Ctrl+R) re-reads the config and theme | `btop.cpp:320-326,1139-1148` |
| fcitx5 5.1.21 classic UI | **live** | D-Bus `org.fcitx.Fcitx.Controller1.ReloadAddonConfig "classicui"` re-reads `themes/<name>/theme.conf`. `fcitx5-remote -r` does **not** restyle it | `src/modules/dbus/dbusmodule.cpp:781`; `classicui.cpp:153-251`; `instance.cpp:2021-2063` |
| ReGreet 0.5.0 | static, next greeter start | a GTK 4 app under cage before any user session | *inferred* (source not read) |

### What the table means for the design

- The first-party apps are already live-capable. What is missing is a
  look *layer* they can re-read, and a tool that drives their existing
  reloads in order.
- The weak spots are concrete: a bad color is silently half-applied
  (`config.rs:194-206`), the wallpaper is never waited on
  (`reload.rs:125-133`), and a colors-only bar reload restarts every
  module (`modules/mod.rs:615-661`). Tickets below fix each one.
- The live knobs for GTK are **dconf keys**, not files: `color-scheme`
  (portal → GTK 4 and libadwaita), `gtk-theme` (GTK 3's dark/light),
  `icon-theme`, `cursor-theme`/`cursor-size` and `font-name`. Today the
  profile writes only `color-scheme` (`theme-home.nix:628-631`). A look's
  *colors* reach GTK only at the next launch (`gtk.css` is never re-read),
  and libadwaita's accent can only snap to a named accent. That is the
  standard path (`org.freedesktop.appearance` through
  xdg-desktop-portal), as `CLAUDE.md` asks; nothing bespoke is added for
  toolkits.
- foot cannot load a new palette live. A swap reaches new terminals; a
  live push into running ones (OSC per pty) is opt-in and hazardous
  (it can interleave with a program's own output), so it is a research
  item, not a default.
- Helix, btop, mako and fcitx5 have documented live reloads, so a swap
  drives them; wob restarts (it is an OSD; nothing is lost).
- Qt may be broken today, before any of this: adwaita-qt likely discards
  the look's palette in widget apps. The toolkit ticket's first step is a
  screenshot that settles it.

## 3. The token schema: what a look is as data

### 3.1 A look is a directory, and a rendered look is another

Two different things carry the word "look", and they never share a path
(the review's finding 5: one name meaning two things at two write sites is
the `Tty::active` class of bug `CLAUDE.md` warns about):

- A **source look** is what an author writes and what `extends`, `new
  --from`, `show`, `validate`, `install` and `export` act on:

  ```
  <source>/moonrise/
    look.toml          # the manifest: tokens, metadata, per-app overrides
    wallpaper.png      # optional, referenced from look.toml, inside this dir
    fonts/             # optional bundled font files (D13), license required
    preview.png        # optional, `scoot look preview --capture`
    apps/              # optional static leaves, trusted looks only (§8.3)
      foot.ini  helix.toml  btop.theme  starship.toml  regreet.css
  ```

- A **rendered look** is the output of `scoot-look build`: one directory of
  per-app fragments (`scoot.toml`, `bar.toml`, `gtk/gtk.css`,
  `qt/scoot-look.conf`, `mako`, `swaylock`, `fuzzel.ini`, `wob.ini`,
  `foot.ini`, ...) plus a `render.json` naming the source, its hash and the
  resolved inputs (font files, wallpaper path). It is never edited and never
  a source for anything; the state link only ever points at one.

Where each lives (both paths, Nix and not, produce the same two trees):

| | Sources | Rendered |
|---|---|---|
| Shipped with scoot | `<prefix>/share/scoot/looks/<name>/` (from `looks/` in the repo, D1) | built on demand into the user's cache, or at Nix build time |
| home-manager | `~/.local/share/scoot/looks/<name>` → the source in the store (read-only links) | `~/.local/share/scoot/rendered/<name>` → the rendered store path |
| NixOS module only | `/run/current-system/sw/share/scoot/looks/<name>` | `/run/current-system/sw/share/scoot/rendered/<name>` |
| `scoot look new`, `import-base16`, `save` (the user's own) | `$XDG_DATA_HOME/scoot/user-looks/<name>/` (writable, never under a home-manager link) | `$XDG_CACHE_HOME/scoot/rendered/<name>-<hash>/` |
| `scoot look install` (someone else's) | `$XDG_DATA_HOME/scoot/installed-looks/<name>/` | the same cache |

The catalog is the union of the source directories, searched in a fixed
order (user-looks, installed-looks, home-manager, system, shipped).
**Origin decides trust, and the directory records the origin:**

- *Shipped, declared (Nix) and user-authored* looks (`new`, `import-base16`,
  `save`) are the user's own choices: any of them can be a base for
  `extends` or `new --from`, so a look made with `look new` can be extended
  by the next one.
- *Installed* looks (someone else's) can be applied but never extended, and
  may not take the name of any look of the first kind (`install` refuses
  it; §8.3).
- A name that exists twice among the first kind resolves to the earliest in
  the search order, and `scoot look list` marks the shadowed one.

`extends` always names a source look; a rendered look has no `extends`.

### 3.2 `look.toml`

```toml
schema = 1
name = "moonrise"                 # [a-z0-9-]{1,32}; must match the directory name
title = "Moonrise"                # display text, grammar in §3.5
description = "Slate navy, a disc-amber ring and the night-sky illustration."
polarity = "dark"                 # "dark" | "light": GTK/Qt variant, portal color-scheme
extends = "ginger-night"          # optional: inherit every token, override below
license = "MIT"                   # SPDX expression; required for shipped and installed looks
credits = ["Illustration: ..."]   # display text, shown by `scoot look show`
preview = "preview.png"

[palette]                         # #rrggbb only; required roles marked *
background     = "#2B3648"        # * the desktop field, the bar body
foreground     = "#F6EEDC"        # * body text
dim            = "#9C8B95"        # * secondary text, separators, placeholders
accent         = "#FFA45C"        # * highlights
urgent         = "#E87F6A"        # * alerts, errors, critical borders
surface        = "#2B3648"        #   raised panels: popups, cards, menus; default = background
hover          = "#FFD54A"        #   pointer hover; default = accent
focus          = "#FF9A49"        #   the focused window ring, overlay borders; default = accent
border         = "#5E4B5B"        #   unfocused rings, low-urgency borders; default = dim
accent-fg      = "#2B3648"        #   text on an accent fill; default = background
selection      = "#FFA45C"        #   selected row or text fill; default = accent
selection-text = "#2B3648"        #   text on selection; default = accent-fg
match          = "#FFD54A"        #   matched characters in a launcher; default = hover

[terminal]                        # optional: the 16 ANSI hues, hand-placed
colors = ["#1a1512", "#c96a45", ...]   # exactly 16

[fonts]
ui   = "DroidSansM Nerd Font Propo"   # bar, notifications, launcher (module glyphs)
sans = "DejaVu Sans"                  # GTK/Qt/greeter UI text
mono = "FiraCode Nerd Font"           # terminals, editors
size = 10                             # points, desktop text (bar)
popup-size = 10                       # points, notifications and launcher; default = size
app-size = 12                         # points, apps and the greeter; default = 12
terminal-size = 10.5                  # points, terminals; default = size
# pixels = clamp(1, 256, (pt * 4 + 1) / 3), the bar's rule (scootbar.nix:97-100)

[fonts.files]                         # optional carrier (D13): files bundled in fonts/
ui = "fonts/DroidSansMNerdFontPropo-Regular.otf"

[fonts.nix]                           # optional carrier (D13): nixpkgs attribute paths
ui = "nerd-fonts.droid-sans-mono"
sans = "dejavu_fonts"
mono = "nerd-fonts.fira-code"

[opacity]                             # 0.0 to 1.0; each default 1.0
bar = 0.82
popups = 1.0
terminal = 0.80

[shape]
radius = 14                           # window corners, bar, popups
ring-width = 2
ring-inactive-width = 2               # default = ring-width

[cursor]
theme = "Vanilla-DMZ"
size = 24
accent = "#FF9A49"                    # #rrggbb or "none"; the drawn fallback cursor's accent (#572's cursor_accent); default = focus
# body: unset by default; the fallback cursor keeps its own near-black body
# accent and body tint only scoot's *drawn fallback* cursor, which shows only
# when no xcursor theme resolves; with `theme` set and installed (every
# shipped look sets Vanilla-DMZ) they are invisible, and a themed cursor
# is never tinted. "none" disables the accent, as #572's `cursor_accent`.

[wallpaper]                           # one unit: never merged per key (§4.3)
image = "wallpaper.png"               # inside the look dir; or `color = "#…"`, never both
mode = "fill"                         # fill | fit | stretch | center | tile
fill = "#2B3648"                      # letterbox color; default = background
transition = "none"                   # none | fade | wipe | grow (§4.6)

[icons]
theme = "Adwaita"

[night]
temperature = 3400

[apps.scootbar]                       # per-app overrides, allowlisted keys only (§3.4)
"workspaces.inactive-color" = "#B595AD"
```

**This is the single published token list** every first-party app reads,
the scootlaunch/scootnotify/scootgreet plan included (§9 reconciles it with
that plan's own table, which had `selection-text` and `match` but no
`surface`, `focus` or `accent-fg`; both sets are in the list above).

Why each role beyond the bar's six (`theme.rs:16-32`) earns its place,
measured on the registry (`desktop.nix:1931-2147`):

- `focus` separate from `accent`: the active ring differs from the bar
  accent on 2 of the 5 looks, moonrise (`#FF9A49` vs `#FFA45C`,
  `desktop.nix:2068,2074`) and ginger-night (`#FF9A30` vs `#E57F29`,
  `desktop.nix:2107,2113`); on the other three they are equal, which the
  default `focus = accent` reproduces. Two of five is enough: without the
  role those two looks would lose a hue their authors chose. It is also the
  border color mako and fuzzel get today (`notifications-home.nix:216-221`,
  `fuzzel-theme.nix:15-21`), so overlays keep their current borders.
- `border` separate from `dim`: the inactive ring is not `dim` on any of
  the five (ginger-night: ring `#4E2913`, dim `#8F7A63`;
  `desktop.nix:2108,2115`).
- `surface`, `accent-fg`, `selection`, `selection-text`, `match`: the
  GTK/Qt renderers (`theme-look.nix:49-59`) and the launcher already use
  them. Their defaults reproduce today's look values exactly
  (`fuzzel-theme.nix:18-20`: selection = accent, selection text =
  background, match = hover-or-accent), and the base16 mapping gives
  Stylix users the base16 meanings instead (§5.3).

Every optional role has a default derived from a required one, so the five
shipped looks lift without inventing a color.

**Not in a v1 look:** `gap` (layout, not presentation: a change reconfigures
every window and re-clamps the ring; D7), bar `height`, `margin`, `edge`,
`layer`, `exclusive`, module lists. Adding a key later is additive; removing
one is breaking, so v1 starts small.

### 3.3 Derivation: tokens once, per-app fragments only when needed

The crate is split along the review's finding 13, so the pure half stays
pure:

- **`scoot-look` (library, pure):** parse, `extends` resolution, the value
  grammar (§3.5), contrast, and a pure `render(resolved, app, inputs) ->
  fragment` per app, where `inputs` carries what the environment resolved
  (font files, the wallpaper's absolute path, the target binary's feature
  set). No Wayland, no fontconfig, no image decoder, no process spawning;
  builds on macOS beside `scoot-core`.
- **The effectful checks live in the tool** (`scoot look`, and a small
  `scoot-look` binary of the same crate with no Smithay in its closure, which
  is what the Nix derivations run): font resolution (`fc-match` on a non-Nix
  box, the font package on Nix), the wallpaper check (delegated to scootbg,
  which already owns the decoder: a new `scootbg check-image` with the pixel
  and byte caps of §8.3), and the apps' own strict parsers (`scootbar daemon
  --check`, the compositor's strict layer parse). The compositor binary and
  the macOS client gain the pure crate only.

| App | Fragment | From |
|---|---|---|
| scoot | `[appearance]` `focus_ring_*` colors and widths, `background_color`, `corner_radius`, `cursor_theme`, `cursor_size`, `cursor_accent` (after #572); `[wallpaper]` as one unit, without `command` | `focus`, `border`, `background`, `shape.*`, `cursor.*`, `wallpaper.*` |
| scootbar | `[colors]` (`background`, `foreground`, `accent`, `hover`, `dim`, `urgent`); `[bar]` `radius`, `popup-radius` (only when the binary has the `popup` feature), `opacity`, `font`, `fallback-fonts`, `font-size`, `padding`, `spacing`, `separator`; allowlisted per-module visual keys | palette, `shape.radius`, `opacity.bar`, `fonts.ui` file, `fonts.size` → px |
| scootbg | the `[wallpaper]` section, through scoot | `wallpaper.*` |
| scootnotify, scootlaunch | `[colors]` with all thirteen roles, `font`, `font-size`, `opacity`, `radius` (§9) | palette, `fonts.ui` file, `fonts.popup-size` → px, `opacity.popups`, `shape.radius` |
| scootgreet | the same, read at system build time | palette, `fonts.sans` file, `fonts.app-size` → px |
| GTK 3/4 | `gtk.css` named colors and `:root` vars; the dconf keys of §4.7 | `appColors` (`theme-look.nix:49-59`), moved from Nix into the crate |
| Qt (qt6ct) | color scheme; `qt6ct.conf` style by polarity | the same |
| mako, swaylock, wob, fuzzel, fcitx5 | their config formats, quoted per §3.5 | the mappings in `notifications-home.nix:216-232`, `idle-home.nix:31-46`, `audio-home.nix:36-75`, `fuzzel-theme.nix:15-21`, `inputmethod-home.nix:43-74`, moved into the crate |
| foot | `[colors-dark]` (or `[colors-light]`) from `[terminal]`, `alpha` from `opacity.terminal`; foot 1.28 has no `[colors]` | no `[terminal]` table means no foot fragment, never a synthesized palette (`theming.md:98-106`) |
| starship, Helix, btop, ReGreet | static leaves from `apps/` only | trusted looks (§8.3) |

**Fragment names** (inside a rendered look, read through `<state>/look/`):

| App | Fragment | Its `[apps.<app>]` table in `look.toml` |
|---|---|---|
| scoot | `scoot.toml` | `[apps.scoot]` |
| scootbar | `bar.toml` | `[apps.scootbar]` |
| scootnotify | `notify.toml` | `[apps.scootnotify]` |
| scootlaunch | `launch.toml` | `[apps.scootlaunch]` |
| scootgreet | `greet.toml` (rendered at system build time, §6.4) | `[apps.scootgreet]` |
| third-party | `gtk/`, `qt/`, `mako`, `swaylock`, `fuzzel.ini`, `wob.ini`, `fcitx5/`, `foot.ini`, `apps/*` | `[apps.<name>]` where the renderer has an override |

**The `[apps.<app>]` convention:** each key is the app's own dotted config
key, quoted (`"colors.border" = "#…"`, `"workspaces.inactive-color" =
"#…"`), its value replaces what the renderer derived for that key, and only
keys in that app's look allowlist are accepted (the same allowlist its
layer loader enforces). Each app's allowlist is published with the app
(scoot's and scootbar's in their layer tickets, the shell apps' in their
own plan), in one place: the `scoot-look` crate, so the validator and the
loaders cannot disagree.

### 3.4 Presentation only: keys *and* values

A look may set colors, geometry, fonts, opacity, the wallpaper and the
cursor. It may never set a command, a bind, an autostart entry, a module
list, an `exec`/`button`/`push` table, an `on-*` action, `[wallpaper]
command`, `[tty]`, `[renderer]`, `[xwayland]` or `[virtual_input]`, nor bar
`edge`, `height`, `margin`, `layer`, `exclusive` or `tooltip-delay`. The
shipped `bar.toml` files carry commands (`docs/examples/moonrise/bar.toml:58-64,76-86`)
and scoot's file carries `[autostart]` and `[binds]`. `validate` refuses a
non-allowlisted key by name, and each app's look-layer loader refuses it
again (the loader never trusts the tool).

The allowlist excludes every restart-only key (`reload.rs:77-90`), so a swap
never produces a "takes effect on restart" refusal. It also excludes every
key of scootbar's `Bar` (`edge`, `height`, `margin`, `layer`, `exclusive`:
`crates/scootbar/src/bar.rs:185-197`), which is what decides whether the bar
destroys and recreates its layer surface (`daemon/surfaces.rs:292-309`), so
a look never recreates the bar's surface.

**Keys are not enough: values are interpolated into line-based formats.**
The renderers write look strings (font families, theme names, glyph strings,
`title`) into mako, fuzzel, swaylock, wob, qt6ct, GTK `settings.ini` and
foot configs, and those formats have keys that run code. At the pinned
versions:

- mako 1.11.0: `on-button-left`/`-middle`/`-right=exec <command>` runs a
  POSIX shell command, and `include=` reads another file
  (`doc/mako.5.scd:32,44-54,100-114` in the pinned source).
- fuzzel 1.14.1: `launch-prefix` and `terminal` prefix every launch, and
  `include` reads another file (`doc/fuzzel.ini.5.scd:28,246,267`).
- foot 1.28.0: `shell` and `include` (`doc/foot.ini.5.scd:36,110`).
- GTK 3.24.52: `gtk-modules` is a `settings.ini` key
  (`gtk/gtksettings.c:577,3039`), and an absolute module path is loaded as
  is (`gtk/gtkmodules.c:180-181`): a shared library from anywhere.

So one newline in `fonts.ui` would be a key injection, and from there code
execution. The value grammar in §3.5 closes that for every string, and each
renderer also quotes for its own format; neither alone is trusted.

### 3.5 Validation and the value grammar

Strict, all-or-nothing, the same code in CI, `install`, `watch`, `apply`
and the Nix build. Pure checks (crate):

- `schema` known; required roles present; every color is exactly
  `#[0-9A-Fa-f]{6}` (no per-field fallback, unlike `config.rs:194-206`),
  except `[cursor] accent`, which also takes `"none"`.
- **Names** (`name`, `extends`): `[a-z0-9-]{1,32}`.
- **Identifiers** (font families, cursor and icon theme names): ASCII
  letters, digits, space and `-_.+` only, 1 to 64 bytes, starting with a
  letter or digit (so never `.`, `..` or a hidden name: a theme named `..`
  would resolve to the parent of `~/.icons`). ASCII-only is deliberate in
  v1: a font family whose name is not Latin script is not expressible, and
  a look that needs one carries the file in `[fonts.files]`.
- **Nix attribute paths** (`[fonts.nix]`): one to eight segments joined by
  `.`, each `[A-Za-z_][A-Za-z0-9_'-]{0,63}`; resolved only with
  `lib.attrByPath` against `pkgs`, never evaluated as an expression, and a
  missing attribute is an evaluation error naming it.
- **Wallpaper URLs** (`wallpaper.image` in its URL form, §8.3): `https://`
  only, at most 2048 bytes, printable ASCII without space, `"`, `'`, `\`,
  `<`, `>` or a backtick, and a required `sha256` of exactly 64 hex digits.
  The host must be a DNS name with at least one dot: no userinfo (`@`
  anywhere before the path), no IP literal (dotted IPv4 or a bracketed
  IPv6), no explicit port, and not `localhost` or a name under
  `.localhost`, `.local`, `.internal` or `.home.arpa` (R5: with the
  `sha256` a look cannot read a reply back, but a swap would still make
  scootbg send a request to a local service). The host is lowercased and
  one trailing dot stripped before these checks (`LOCALHOST.`), and a host
  whose last label is all digits or starts with `0x` is refused, since curl
  and the WHATWG parser read `127.1` and `0x7f.0.0.1` as IPv4 (F2); IDNs are
  fine given the hash pin. A public name can still resolve to, or redirect
  to, a private address, so for an *installed* look the fetch happens at
  install time instead (§8.3), never at swap time, with no redirects and
  the resolved address checked and pinned (`look-install`, F1: scootbg's own
  fetch follows up to five redirects, `crates/scootbg/src/fetch.rs:64,347-353`).
- **Display text** (`title`, `description`, `credits`, `license`): UTF-8
  without control characters (C0, DEL, C1), no `=`, `[`, `]`, `"`, `\`,
  at most 200 bytes (`title` 64). Display text is never written into an
  app config.
- **Glyph strings** (allowlisted module glyph keys): 1 to 8 Unicode scalar
  values, no control or format characters, no ASCII punctuation.
- **Paths** (`wallpaper.image`, `preview`, `fonts.files.*`, `apps/*`):
  relative, normalized, no `..` component, no leading `/`, resolving to a
  regular file inside the look directory after symlink resolution (§8.3).
- **Numbers**: each in its documented range (sizes 1 to 256 px after
  conversion, opacity 0 to 1, temperature 1000 to 10000, ring widths as
  `Appearance::clamped` bounds them).
- NUL is refused everywhere; the whole file is capped at 64 KiB (scootbar's
  cap, `crates/scootbar/src/config.rs:77-79`); `extends` resolves with no
  cycle and depth at most 8.
- Every renderer quotes for its format on top of the grammar (TOML through
  a serializer, INI values on one line with the format's own escaping where
  it has one, CSS strings quoted and escaped), and the renderer tests feed
  each format a newline, `=`, `[` and a quote to prove the grammar and the
  quoting both hold.

Contrast, with the thresholds `nix/tests.nix:14907-14932` already applies to
`gtk.css`: foreground on background ≥ 4.5, accent-fg on accent ≥ 4.5,
accent on background ≥ 3.0, focus on background ≥ 3.0. On every shipped
look `accent-fg` is the default (`background`), so the second and third
columns are the same pair there; the second check matters only when a look
sets `accent-fg`:

| Look | fg/bg | accent-fg/accent | accent/bg | focus/bg | dim/bg | urgent/bg |
|---|---|---|---|---|---|---|
| vinyl-sunset | 13.2 | 7.0 | 7.0 | 7.0 | 2.18 | 4.48 |
| music-desk | 15.7 | 6.7 | 6.7 | 6.7 | 1.57 | 2.88 |
| radial-burst | 14.4 | 6.5 | 6.5 | 6.5 | 5.27 | 3.02 |
| moonrise | 10.5 | 6.2 | 6.2 | 5.8 | 3.79 | 4.45 |
| ginger-night | 16.2 | 6.8 | 6.8 | 9.1 | 4.72 | 5.28 |

(WCAG relative luminance, the formula of the existing check; reproduced by
the independent review to 0.1.)

- **`dim` and `urgent` are reported, not refused, in v1.** `dim` is the
  role for inactive and incidental text, which WCAG exempts, and two
  shipped looks sit well under 3.0 (music-desk 1.57, vinyl-sunset 2.18);
  `urgent` is a non-text alert indicator whose bar would be 3.0, and
  music-desk is at 2.88. Refusing either would refuse a shipped look today.
  `look-shipped` puts both numbers in front of the maintainer, who decides
  whether to retune those two looks; a later schema can make `urgent ≥ 3.0`
  a refusal once they pass.
- **Who a contrast failure stops depends on who chose the look.** A shipped
  look, and one arriving through `scoot look install`, is refused below a
  bar, with the pair and the ratio named. A look the user's own
  configuration produced (the synthesized `stylix` look, an `import-base16`
  result, a path listed in `looks`) gets a loud warning and is never
  refused: base16 Solarized Dark puts `base00` on `base0D` at 4.08 and
  Solarized Light's accent pair is 3.41, and refusing them would break a
  working Stylix user's next `home-manager switch`. Every other check
  refuses for every look.
- **A high-contrast look** is not in v1 (a non-goal, §11): nothing here
  prevents one, and the portal's `contrast` key that GTK 4 and libadwaita
  read is listed in `look-toolkit-live` as a follow-up.

Effectful checks (tool):

- Fonts: each family resolves through its carrier (D13); the bar, notify
  and launch faces resolve to a file.
- Wallpaper: `scootbg check-image` (byte and pixel caps checked from the
  header before any decode, then a full decode). The verdict is cached by
  path, size and mtime, so *validation* decodes once. The swap itself still
  costs scootbg a decode every time it shows that image (479 ms CPU for a
  6000x4000 JPEG at 4K, on its worker thread, `dev/benches/scootbg/2026-09-28-scoot-peak/table.md:9-10`);
  on a first apply both happen.
- `license` present for shipped and installed looks; `apps/` leaves only on
  a trusted look (§8.3).
- The rendered fragments pass the apps' own strict parsers: the
  compositor's look-layer parse, `scootbar daemon --check` (against the
  installed binary's features), scootbg's strict section parse.

## 4. The runtime: one indirection, one reload protocol

### 4.1 The indirection

```
<state>/look         -> symlink to the active *rendered* look
<state>/look.json    -> { name, rendered, generation, chosen-by, declared-default }
<state>/look.lock    -> flock, every writer (§4.4)
```

`<state>` is resolved in this order: `--state DIR`, then `$SCOOT_LOOK_STATE`,
then `$XDG_STATE_HOME/scoot`, then `~/.local/state/scoot` (the basedir
spec's fallback). An unwritable `<state>` makes `apply` refuse loudly;
startup then runs without a look layer.

**`look.json` is the record and the link is derived from it.** A swap
writes `look.json` (temp file, `rename`), then the link (`symlink` temp,
`rename`). Startup, `sync` and every `apply` compare the two and repair the
link from `look.json`, so a crash between the two renames heals the next
time anything looks. Apps never read `<state>/look.json`: they read the
link, which is always a complete rendered directory or absent, and the
`render.json` inside it (§3.1) is what tells scoot the layer's identity
changed (§4.5). Two different files, two different names.

Consumers point at the link, never at a specific look:

- **scoot and scootbar** read `<state>/look/scoot.toml` and
  `<state>/look/bar.toml` as a look layer under the user's own file (§4.3).
  Their config files stay the user's; a swap never rewrites them. A
  dangling or missing link is "no look layer" plus a warning at startup
  (startup never fails over a look, `config.rs:16-23`).
- **Per-launch third-party apps** take the link's path in their command
  line or config: `swaylock -C <state>/look/swaylock`, fuzzel `--config`,
  `gtk.css` as one `@import`, mako through `include=`, qt6ct's
  `color_scheme_path`. A new process reads the new look with no hook.

### 4.2 Whose live choice: per display, persisted per user (D10)

The bar's socket is per display (`scootbar-$WAYLAND_DISPLAY.sock`,
`crates/scootbar/src/control/paths.rs:1-8`) while `$XDG_STATE_HOME` is per
user. Without a scope, an agent's `scoot msg look-apply` inside a headless
test session would repoint the user's real desktop, which would show it at
its next reload. So:

- **`--tty` and `--nested` sessions use the user's state**
  (`$XDG_STATE_HOME/scoot`, deterministic, no environment needed), so the
  choice persists across logins. `--nested` is a daily-drive path, not a
  test path: it is the webtop deployment (`exec scoot --nested -- …`,
  `site/src/content/docs/agents/webtop.md:17`) and the "try it first" path
  `CLAUDE.md` names, and a private state there would lose every swap at the
  next session restart.
- **Only `--headless` is private by default**, under
  `$XDG_RUNTIME_DIR/scoot-look-<display>/`, seeded at startup by copying the
  user's `look.json` (and rendering its link there). Its swaps never touch
  the user's choice. Any backend can opt in with `--private-look` (the nested
  preview of §8.1 uses it); an agent test that wants full isolation still
  sets its own `XDG_STATE_HOME` and `XDG_RUNTIME_DIR`
  (`look-golden-screenshots` requires it).
- **How the state path reaches processes:** by default it is the
  deterministic user path, which every process computes the same way, with
  no environment variable. Only a private session exports
  `SCOOT_LOOK_STATE`, to its direct children (as scoot already exports the
  live `XCURSOR_*`, `state.rs:1850-1851`). systemd user units (the bar's,
  mako's) never see it: `resources/scoot-session` imports the launcher's own
  environment before the compositor starts (`--all`, 499-506), which a
  variable scoot sets only for its children is not part of, and after it
  starts only `WAYLAND_DISPLAY`, `XDG_CURRENT_DESKTOP` and `XDG_SESSION_TYPE`
  (597-603). So units always act on the user state: correct for `--tty` and `--nested`, which use it, and
  harmless for private sessions, which are test and preview sessions that
  start no user units. A private session that did start units would have
  them act on the user state; that is a documented limitation, not a path
  any shipped launcher takes.
- `scoot look` run outside any session (an ssh shell) acts on the user
  state and reloads the session on the default socket only; `--state` and
  `$SCOOT_SOCKET` select another.

### 4.3 Precedence, merged on raw keys

At runtime, per app: **flags > the user's own file > the active look's
fragment > the app's built-in default.** The flags rung is scootbar's
`--background`/`--foreground`, which overlay everything on every reload
(`crates/scootbar/src/daemon/respond.rs:184-189`; nothing in the Nix unit
passes them).

The merge happens on **raw TOML keys, before any derivation**, by three
rules:

1. **Every key merges per key**: the user's key if their file has it, else
   the look's, else the app default. A user who sets only `cursor_size` (a
   common accessibility setting) keeps the look's cursor theme and accent;
   a user who sets only `hover` keeps the look's `accent`.
2. **`[wallpaper]` is the one unit**, including its per-output and
   per-workspace tables (`wallpaper/section.rs`): if the user's file has any
   `[wallpaper]` key, the whole section is the user's, else the whole
   section is the look's. Its keys are mutually exclusive (`image` or
   `color`): a user `color` merged per key with a look `image` gives both,
   which scootbg refuses, and the session falls back to `background_color`
   (`docs/backlog/resolved/stylix-wallpaper-color-trap-done.md:17-24`), for
   every look with an image (4 of 5).
3. **A derived key follows the layer that supplied its base.** Where an app
   derives one key from another when the first is unset, the merge must not
   pair a base from one layer with a derived value from the other. The
   pairs, published in `scoot-look` beside the allowlists:

   | App | Derived key | Base | Rule |
   |---|---|---|---|
   | scootbar | `colors.hover` | `colors.accent` (`crates/scootbar/src/config.rs:1692`) | the user's `hover` if set; else, if the user set `accent`, the look's `hover` is dropped so the bar derives hover from the user's accent; else the look's `hover` |
   | scoot | `appearance.focus_ring_inactive_width` | `appearance.focus_ring_width` (`config.rs:209-210`) | the user's inactive width if set; else, if the user set the active width, the look's inactive width is dropped so it follows the user's; else the look's |

   Values a look derives *inside itself* at render time (`cursor.accent`
   defaulting to `focus`, `wallpaper.fill` to `background`) are plain keys of
   its fragment by then and merge per key: a user who sets the ring color
   keeps the look's cursor accent.

Derived values are then computed by each app as today (the wallpaper `fill`
default for a user section, the ring re-clamp against the gap in
`reload.rs:456-468`). That one rebuild serves the whole cursor triple
(`reload.rs:411-433`) is a fact about applying, not merging: the cursor keys
merge per key and the rebuild runs once on the merged result.

**A bad value in the user's file falls back to the look's value, not the
compositor default,** and is reported: today a bad user color becomes
`Appearance::default()` with a warning (`config.rs:194-206`), which with a
look layer would show neither the user's color nor the look's while the
swap answered "applied". The layer loader changes the fallback for keys the
layer provides, and the reply lists the key as invalid and shadowing.
scootbar already refuses a bad color whole (`config.rs:1688-1694` returns
the error), so the bar needs no change here.

**Shadowed keys are reported.** Every key of the user's file that overrides
a look token is listed by name in the apply reply and in `scoot look
current`. Without it a swap on an existing install would answer "applied"
and change nothing: `install.sh --look` already wrote a look's colors and
wallpaper into users' own `config.toml` and `bar.toml`
(`site/public/install.sh:446,460`), and Nix users may set
`programs.scoot.settings.appearance.*` explicitly.

### 4.4 The reload protocol: each app's control-socket `reload`, scoped to the look

**Decision: every long-running first-party app implements `reload` on its
own control socket, re-reads, validates whole and answers applied/refused;
`scoot look apply` drives those requests in order and aggregates the
replies.** Two of the three apps already do:

- scoot: `Request::Reload` on `scoot.sock`, `Response::Reloaded { applied,
  refused }` (`scoot-ipc/src/request.rs:115-122`, `reload.rs:240-271`).
- scootbar: `{"protocol":1,"type":"reload"}` on `scootbar-NAME.sock`
  (`control/protocol.rs:1-17,52`).
- scootbg: driven by scoot.

**A swap reload is scoped to the look** (the review's finding 8). A plain
`reload` re-reads the user's file too, so a swap would also apply whatever
the user has half-edited: new `[autostart]` entries would spawn
(`reload.rs:101-112`), binds would swap, and a half-written file would
refuse the reload and roll the swap back over an unrelated file. So both
apps gain `reload` with `scope = "look"`: the app merges the new look layer
under **a snapshot of the user file's raw table from its last successful
load**. That snapshot is new state in both apps: today scoot's reload diffs
a freshly loaded `LoadedConfig` against live `State` fields
(`reload.rs:291-301,343-480`) and keeps no raw table, and scootbar's reload
re-reads and re-parses the file (`daemon/respond.rs:177-191`); both start
keeping the raw table at every successful load (startup and plain reload),
which the raw-key merge of §4.3 needs anyway. Pending edits
to the user's file wait for the user's own `reload`; autostart never runs
from a swap. A plain `reload` keeps its meaning (re-read both).

Rejected:

- **SIGHUP as the contract.** scootbar cannot catch it
  (`#![forbid(unsafe_code)]`, `daemon/mod.rs:27-35`), so a HUP kills the
  bar, and a signal has no reply channel (`reload.rs:241-247` logs for that
  reason). scoot keeps its HUP as a second trigger of a plain reload.
- **inotify on the link in every app.** `sighup.rs:5-7` rejected watching the
  config because an editor fires it mid-keystroke. The link's atomic rename
  does not have that problem, but a watch per app loses the ordering, the
  scope and the aggregated reply. The dev loop (`scoot look watch`, §8.1)
  does watch, in the tool, debounced, and still drives the scoped reloads.
- **A new `look.apply` message per app.** `reload` with a scope says what is
  needed; the apps need not know what a look is beyond one more layer.
- **Rewriting each app's config file per swap.** Clobbers the user's edits
  and cannot be atomic across files.

### 4.5 The swap transaction

`scoot look apply NAME`, under the lock:

1. **Render and validate** the target completely (§3.5). Any failure
   refuses loudly; nothing has been touched.
2. **Record and repoint**: `look.json` (bumping `generation`), then the
   link (§4.1).
3. **Reload, scoped to the look, in order:** scoot (ring, background,
   cursor, wallpaper handed to scootbg, waiting for the wallpaper run: a
   reload option, since today it is fire-and-forget, `reload.rs:125-133`)
   → every running scootbar of this session → scootnotify (once it exists)
   → the toolkit steps of §4.7 → (scoot emits `EventKind::Look` itself when
   its reload sees the layer's identity change, so a CLI swap, a `sync` and
   an IPC swap all produce the event).
4. **An app that is not running is skipped** with a note, never a refusal:
   no bar, no scootnotify, no compositor (then only the link moves, and the
   next session start reads it).
5. **On a first-party refusal** (only environmental by now: everything was
   validated): repoint back, re-run the scoped reloads that already ran, and
   exit non-zero naming the app and the reason. If a rollback reload also
   refuses, the link stays at the previous look, both errors are reported,
   and the exit code says "rolled back with errors"; each app is on its
   old or its new look, never between (each swaps whole).
6. **Report** what applied, what was skipped, and every shadowed key.

A crash between step 2 and step 3 leaves apps on the old look with the link
on the new one; re-running `apply` or `sync`, or any app's next reload,
heals it. "A half-applied look is impossible" holds per first-party app
(each swaps whole: `reload.rs:273-277`, `respond.rs:10-12`) and across them
through step 5. It does not hold across third-party processes, which cannot
be transacted (§4.7).

### 4.6 Concurrency, previews and the cache

- **One lock** (`<state>/look.lock`, `flock`) is taken by every writer:
  `apply`, `sync` (home-manager activation), `watch` (per apply), the
  `--for` revert, and the compositor's IPC child. A second writer waits a
  bounded time, then refuses.
- **`--for` previews** record the generation they will revert from; a
  revert whose generation has moved (any later apply) does nothing, so a
  preview never undoes a newer choice.
- **The render cache** (`$XDG_CACHE_HOME/scoot/rendered/`) keeps the
  active, the previous and the four most recent renders, pruned under the
  lock, never deleting a directory `look.json` or the link names. `watch`
  renders into one reused slot per watched look, so it does not grow the
  cache per save.

### 4.7 Toolkits and other third-party apps

Every step here is best-effort and reported (a missing `dconf` or no fcitx5
is a note, not a failure):

- dconf keys (D11): `color-scheme`, `gtk-theme` (GTK 3's dark/light),
  `icon-theme`, `cursor-theme`, `cursor-size`, `font-name`, followed live by
  GTK 4 and libadwaita through portal-gtk and by GTK 3 directly (§2).
- `makoctl reload`; fcitx5 `ReloadAddonConfig "classicui"`; `SIGUSR1` to
  the user's own Helix processes and `SIGUSR2` to btop (their documented
  reloads, each opt-out); a wob unit restart.
- Running GTK apps keep their `gtk.css` palette and running terminals
  their palette until they restart; the reply says so.

### 4.8 Failure contract (every app)

- An invalid look is refused before the swap, by name and reason.
- Last good is kept: the link never points at an unvalidated directory.
- Never wedge: startup with a broken or dangling link starts without the
  look layer; a scoped reload refuses; nothing exits. A look can never cost
  the user their binds, autostart or session, and a swap can never apply
  their pending edits.
- Refusals are loud on both channels: the reply and the log.

### 4.9 Under session lock

A swap applies under lock, like a reload (`reload.rs:143-171`): the ring,
background, cursor and wallpaper are config-derived pixels and disclose
nothing. IPC and the CLI need no special case. A **keybinding** does: a
locked session runs only binds flagged `allow_when_locked`
(`shell.rs:300-312`), so the default look-cycle bind (D8) does nothing while
locked, by design.

### 4.10 Performance

A swap is a rare, cold event. Nothing in it may land on a per-frame path.

- Rendering, font resolution and the image check run in the tool's process
  (or at Nix build time), never on the compositor's loop thread; scootbg
  decodes on its worker (§3.5).
- The compositor's share is a scoped reload: a TOML parse, a diff and
  `apply()` when a ring or background moved (`reload.rs:308-321`).
  `apply()` re-arranges every window (`shell.rs:375-405`), so the ring and
  background fields should request a redraw without `apply()`, as the
  cursor fields already do (`reload.rs:322-331`); `look-layer-scoot`
  measures it before and after.
- **The cursor rebuild runs on the loop thread** (`Cursor::rebuild`,
  `reload.rs:433-437`). With PR #572's 22 drawn shapes its own debug-build
  numbers are 36.4 ms at 16 px and 1.82 s at 256 px (PR #572 body, not
  measured here). A swap that changes the cursor pays that inside one loop
  turn: `look-layer-scoot` measures it in a release build at 24, 48 and
  256 px, and if it exceeds a frame at common sizes the rebuild moves to a
  worker that hands finished bitmaps back to the loop.
- scootbar restarts every module on every reload
  (`modules/mod.rs:615-661`); `scootbar-style-reload` makes a style-only
  reload keep every module and repaint.
- A 4K `fade` drops frames (`transitions.md:110-118`), so a look's default
  transition is `none`.
- Evidence the swap tickets record: frame times across a swap on headless
  and `--tty`, the swap's wall time, and zero new per-frame allocations
  after it.

## 5. Stylix: a source of a look, not a layer above it

The maintainer asked: "With all this do we keep stylix or create look
support with custom looks". **Both: looks are the system of record and the
runtime selector; Stylix becomes one source of a look.**

### 5.1 Why the current ladder cannot stay

Today a key is decided at evaluation time: user (100) > Stylix
(`mkDefault`, 1000) > look (`mkOptionDefault`, 1500) > plain default
(`scootbar.nix:7-20`; the same ladder in `home.nix:534,571-608,623-624`).
A Stylix value is baked into `config.toml` and `bar.toml` as an ordinary
setting, which sits in the *user* rung at runtime, above any look layer, so
a live swap could never change a key Stylix set. Keeping Stylix above the
look makes the swap a no-op for every Stylix user; moving it below makes
Stylix meaningless. Stylix has to stop being a layer.

### 5.2 The new ladder

**Build time (home-manager or NixOS evaluation):** what the user wrote in
`programs.scoot.settings`, `programs.scootbar.settings` and the per-slot
`settings` > the module's plain defaults. No look or Stylix value is
merged into those files any more. Evaluation decides only the catalog
(`looks`), the declared default (`look`) and the trust set.

**The declared default, with exact priorities.** The option
`programs.scoot.look` keeps `default = null` (an option default is
`mkOptionDefault`, priority 1500). Its type is a non-mergeable one (an enum
today, a catalog name after `look-nix-catalog`), whose merge throws on two
different values at the same priority (`mergeEqualOption`,
`lib/options.nix:500-514` at the pinned nixpkgs; the enum uses it,
`lib/types.nix`'s `enum`). So the definitions must sit at distinct
priorities (lower wins, `lib/modules.nix:1603-1605`: `mkDefault` = 1000,
`mkOptionDefault` = 1500, a plain assignment = 100):

| Who | Value | Priority |
|---|---|---|
| the user | anything they write | 100 (plain) |
| the Stylix source, when Stylix is present and on | `"stylix"` | 1000 (`mkDefault`) |
| the desktop profile, when on | `"ginger-night"` | 1200 (`mkOverride 1200`, moved from today's `mkDefault` at `home.nix:503-505` and `nixos.nix:2235-2237`) |
| the option | `null` | 1500 (its `default`) |

The profile cannot move to `mkOptionDefault`: it would tie with the
option's own `default = null` at 1500 and throw.

**Runtime:** flags > the user's own file > the active look's fragment > the
app's built-in default (§4.3).

**Where Stylix sits:** inside a look. When Stylix is present and on, the
catalog gains a synthesized look named `stylix` and the declared default
becomes `stylix` (table above). Swapping away and back works like any other
look. **The apps never read Stylix:** not scoot, scootbar, scootnotify,
scootlaunch nor scootgreet. Stylix reaches them only as the `stylix` look.
That is why the per-app `programs.<app>.stylix.enable` options go away
(§5.5, §9).

### 5.3 One mapping, in one place

The base16 → token mapping is implemented once, in `scoot-look`
(`import-base16`), and used by both paths:

- `scoot look import-base16 scheme.yaml [--extends LOOK]` for anyone with a
  base16 scheme file.
- Nix: one function (`nix/lib/look-from-stylix.nix`) serializes
  `config.lib.stylix.colors.withHashtag.base00..base0F`, the fonts, sizes,
  opacity, cursor and image into a JSON file in the store, and a derivation
  runs `scoot-look import-base16` on it. No module maps a base16 slot
  itself; the duplicated mappings in `home.nix:571-573` and
  `scootbar.nix:482-487` are deleted.

The mapping (the palette half is the shell-apps plan's table, which checked
Stylix's own mako and fuzzel targets at `fb28acd`, merged with today's
scoot choices):

| Token | base16 | Source of the choice |
|---|---|---|
| background, surface | base00, base00 | today (`scootbar.nix:482`); surface stays base00 because Stylix's own mako and fuzzel targets draw their cards on base00 (shell-apps plan, read at `fb28acd`) |
| foreground | base05 | today |
| dim | base03 | today (`scootbar.nix:486`) |
| accent, hover, focus | base0D | today (`scootbar.nix:484-485`, `home.nix:571`) |
| border | base03 | today's inactive ring (`home.nix:572`) |
| urgent | base08 | today |
| accent-fg | base00 | the look default |
| selection, selection-text | base02, base05 | base16 "selection background"; Stylix's fuzzel target |
| match | base0A | Stylix's fuzzel target |
| terminal (16) | base00..base0F in base16's terminal order | only when the `stylix` look has no `extends` (below) |
| fonts | `sansSerif` → sans, `monospace` → mono, `sansSerif` → ui (as `scootbar.nix:493` today) | |
| sizes | `desktop` → size, `popups` → popup-size, `applications` → app-size, `terminal` → terminal-size | pt → px by `scootbar.nix:97-100` |
| opacity | `popups` → popups, `terminal` → terminal; the bar's left unset | today the bar reads no Stylix opacity |
| cursor | `stylix.cursor` name and size when set | `home.nix:583-584` |
| wallpaper | `stylix.image` + `imageScalingMode` when set | `home.nix:587,603-606` |
| fonts carrier | the Stylix font packages | D13 |

**The `stylix` look extends a base.** `programs.scoot.stylix.extends`
(default: `"ginger-night"` when the desktop profile is on, none otherwise)
makes the synthesized look inherit everything the palette does not set: the
wallpaper when `stylix.image` is unset, `[terminal]` and the static
terminal/editor leaves, the night temperature, shape and opacity. That is
what keeps profile users closest to what they see today (§5.5).

### 5.4 What Stylix cannot supply, and how the look fills it

| Token | Stylix | Filled by |
|---|---|---|
| `hover`, `focus` | one hue | `base0D`, today's choice |
| `polarity` | `stylix.polarity` exists upstream but is not read or stubbed here | `base00`'s luminance; `stylix.polarity` once verified (§14) |
| cursor accent | no convention (`theming.md:182-183`) | `focus` |
| `[shape]` | none | the base look, else app defaults |
| wallpaper when `stylix.image` is unset | none | the base look's, else a flat `background` |
| night temperature | none | the base look's, else the default |
| starship, Helix, btop, ReGreet static leaves | Stylix's own targets theme these apps | the base look's leaves where scoot owns the path; Stylix's where its target owns it (D14) |

### 5.5 What changes for existing Stylix users, and the migration

**Users with Stylix and the desktop profile, no explicit `look`** (today:
`look` is ginger-night under Stylix). What they see now
(`home.nix:532-541`, `571-608`; `theme-look.nix`; the app modules):

- compositor ring and background, bar colors and font: **Stylix's**;
- wallpaper: Stylix's image if set, **else ginger-night's cat**
  (`home.nix:534-541`, with Stylix's block gated on `image != null`, 595);
- GTK `gtk.css`, Qt colors, mako, fuzzel and the pickers, swaylock, wob,
  fcitx5: **ginger-night's palette** (these modules never read Stylix);
- night temperature 3300; foot, starship, Helix and btop: ginger-night's
  files (where no upstream module owns them).

What they see after (the `stylix` look extending ginger-night):

- compositor and bar *colors and fonts*, wallpaper, night temperature,
  terminal and editor files: **unchanged**;
- compositor and bar *shape and opacity* change the way they change for
  every profile user (below, D15), since the `stylix` look inherits them
  from ginger-night;
- **GTK/Qt colors, mako, fuzzel and pickers, swaylock, wob and fcitx5
  change from ginger-night's palette to Stylix's.** This is a visible
  change and it is deliberate: today those apps disagree with the
  compositor and bar about the palette, and the new rule makes the whole
  desktop one palette.
- To keep the old result exactly: no single look reproduces today's mix (it
  is two palettes). The closest is a user look, `import-base16 --extends
  ginger-night` with the per-app overrides set to ginger-night's values, or
  setting `look = "ginger-night"` for ginger-night everywhere (then the
  compositor and bar flip to ginger-night too). The changelog says so.

**Every desktop-profile user, Stylix or not: shape and opacity (D15).**
Today the Nix path applies only a look's 3 compositor colors, 6 bar colors,
fonts and wallpaper (`home.nix:532-539` maps `look.appearance`, which holds
only the three colors, `desktop.nix:1933-1937`; `scootbar.nix:374-378` maps
`barColors`). The examples' style values never reach a Nix user:
`corner_radius` 12 to 16 (`docs/examples/*/scoot.toml:18-25`), ring widths,
bar `opacity` 0.7 to 0.88, `padding`, `spacing` and `separator`
(`docs/examples/*/bar.toml:22-32`). Non-Nix users who ran `install.sh
--look` have had them all along (it copied the whole files). A look must
behave one way on both paths, so under the recommendation of D15 the
shipped looks carry them, and every profile user sees rounded windows and a
translucent, padded bar on migration. That costs a little per frame (the
example's own comment: "costs a little per frame: 0 is free",
`docs/examples/moonrise/scoot.toml:19`) and an ARGB bar buffer
(`bar.toml:27`). It is a visible change, stated in `look-nix-catalog`'s
changelog, and any user keeps the old result with `corner_radius = 0` and
`opacity = 1.0` in their own files (user > look).

**Users with Stylix, no profile** (scoot and the bar only): nothing visible
changes. The `stylix` look has no base; its colors, fonts, cursor and
wallpaper equal what the per-leaf defaults produce today; a scheme below
the contrast bars builds with a warning, never a failed switch (§3.5).

**Users with Stylix and an explicit `look`:** the look now wins in the
compositor and bar where Stylix used to (`theming.md:38-44`). The apps whose
config files a Stylix home-manager target writes stay Stylix (D14), so
those users get a split desktop until they turn those targets off; the
apply reply names those apps as "owned by Stylix's target".

**D14, decided: ownership by path, as today.** scoot never writes a path an
upstream home-manager module owns (the rule at `theming.md:38-44`,
implemented as `upstreamOwns` and the `gtkOwned`/`qtConfOwned` guards in
`theme-home.nix:55,75-90`). Stylix's targets for gtk, foot, mako, fuzzel and
others write through those upstream modules, so with them on, those paths
are Stylix's, whatever the look. The alternative (the look forces its way
into those paths) means fighting another module for a file at evaluation
time, which is either a collision error or a `mkForce` war the user did not
ask for. A user who wants one look everywhere turns the Stylix targets off
(`stylix.targets.<app>.enable = false`); the docs say exactly that.

**Options (breaking; the sole-user rule ships them clean, marked `!`, with
the maintainer's own `~/nixos-config` updated in the same change):**

- `programs.scoot.desktop.look` moves to `programs.scoot.look`, with
  `programs.scoot.looks` beside it (D2).
- `programs.scoot.stylix.enable` keeps its name and changes meaning:
  "synthesize the `stylix` look and make it the default" (default true).
  `programs.scoot.stylix.extends` is new.
- `programs.scoot.stylix.wallpaper.enable` keeps its name: whether the
  `stylix` look carries `stylix.image`.
- `programs.scootbar.stylix.enable` changes meaning, not name: **the bar-only
  path** (finding 12). A user who runs scootbar without scoot (on sway, which
  CI tests) has no look catalog. With this option on and no
  `programs.scoot.looks` in the configuration, the bar module renders the
  `stylix` look's bar fragment itself (the same `scoot-look` binary) and
  points the bar at it with a new `bar.toml` key, `[look] layer = PATH` (a
  fixed layer, read like the state link). With a catalog present the option
  does nothing and says so in an evaluation warning.

**The NixOS-module-only user** (no home-manager) gets the catalog under
`/run/current-system/sw/share/scoot/` (§3.1), the declared default in
`/etc/scoot/look-default`, and `scoot look sync` run by the session script
(`resources/scoot-session`) at login, since there is no activation step.

**Published text this supersedes,** each updated by the ticket that changes
it: `theming.md:38-44` (Stylix > look), `theming.md:158-189` (the Stylix
table), `theming.md:212-219` (`look` "only ever gains values": the enum
becomes a catalog), and `site/.../scootbar/theming.md` on Stylix precedence.

### 5.6 What the tests can and cannot vouch for

`nix/scootbar-tests.nix:9-15` says it plainly: Stylix is not this flake's
input, the stub declares only what the module reads, checked by hand against
`nix-community/stylix` at `fb28acd`, and "a rename there would pass this
file and miss on a real Stylix". The stub in `nix/tests.nix:754-807`
declares `stylix.enable`, `cursor`, `image`, `imageScalingMode`,
`fonts.sansSerif`, `fonts.sizes.desktop` and five palette slots (`base00`,
`03`, `05`, `08`, `0D`). The `stylix` look reads more (all sixteen slots,
`fonts.monospace`, `sizes.popups`/`applications`/`terminal`, `opacity`,
`polarity`), so `look-stylix-source` checks each new read by hand against a
named Stylix rev, extends the stub, records the rev in the header, and keeps
the real-Stylix run `scripts/scootbar-stylix-test.sh` green.

## 6. Nix and non-Nix parity, and the home-manager rule

### 6.1 Options

```nix
programs.scoot = {
  looks = [ "ginger-night" "moonrise" ./my-looks/moonrise-green ];
  look = "moonrise";      # the declared default: what a fresh state starts on
};
```

- `looks`: names of shipped looks or paths to source look directories.
  Default: every shipped look, plus `stylix` when Stylix is on (D4). Paths
  are read with `builtins.fromTOML` at evaluation time (a source file, not
  a derivation output, so no import-from-derivation); font carriers resolve
  against `pkgs` there (D13).
- `look`: one name in the catalog, or null for no look layer at all.
- Later, not v1: a flake output `lookPacks.<name>` (a directory of looks
  usable as a flake input; `look-share` ticket).

### 6.2 What evaluation produces

- Each look is rendered by a derivation running `scoot-look build` (the
  small binary of §3.3, with the look's font packages as build inputs). It
  validates (§3.5): a parse error, a forbidden key or a missing file fails
  the build before anything changes; a contrast shortfall in a look the
  user's own config chose only warns.
- The catalog: `~/.local/share/scoot/looks/<name>` → the source,
  `~/.local/share/scoot/rendered/<name>` → the render (§3.1). Read-only,
  rooted by the home-manager generation.
- The declared default: `~/.config/scoot/look-default`.
- The live choice is **not** home-manager's: `<state>/look.json` and the
  link are mutable state. The link targets the *catalog's rendered entry*
  (`~/.local/share/scoot/rendered/moonrise`), never a raw store path, so the
  next generation re-resolves it and a garbage collection cannot leave it
  dangling while the generation lives.
- The eval-time merges of look values into `programs.scoot.settings` and
  `programs.scootbar.settings` (`home.nix:532-539`, `scootbar.nix:374-378`)
  are removed: the runtime layer replaces them.
- An activation step runs `scoot look sync` after `linkGeneration` and
  `dconfSettings`, so its dconf writes land last (D11).

### 6.3 The rule on `home-manager switch` (and rollback)

`look.json` records the declared default in force when the live choice was
made. On every activation, under the lock, in order:

1. **No state yet** → start on the declared default.
2. **The declared default changed** since the live choice was recorded →
   the declaration wins: switch to the new default.
3. **The live choice is no longer in the catalog** → fall back to the
   declared default, and say so in the activation output.
4. **Otherwise** → keep the live choice; re-resolve it through the new
   catalog, so an updated look's new contents apply.

Corner cases: the same name with changed contents applies the new contents
(rule 4); `look = null` removes the layer (rule 2); a rollback runs the same
rule against the older catalog and default, so "rolled back together" means
the catalog, the declared default and every file roll back, and a live
choice survives only when that generation still offers it and declares the
same default; with no session running, sync moves the link and stops.

**Rejected: NixOS specialisations as the swap mechanism.** A whole system
activation, root-only, chosen at boot or by `switch-to-configuration`, one
system build per look: none of that is a live, per-user swap. It remains a
fine way to pin a greeter look per boot entry.

### 6.4 The greeter is static

The login screen runs before any user session and is owned by NixOS
(`nixos.nix:2420-2477`). It takes `programs.scoot.greeter.look` (default:
the declared `look`), rendered at system build time. A live swap never
touches it. A non-Nix `sudo scoot look apply --greeter` is later, not v1.

### 6.5 Non-Nix

`install.sh --look NAME` installs scoot's shipped looks (into
`<prefix>/share/scoot/looks/`, recorded in its manifest) and runs `scoot look
apply NAME`, which renders into the cache and points the link. It stops
copying whole `scoot.toml`/`bar.toml` files; a user's existing files are
never touched, except that a file still byte-identical to what an earlier
`install.sh --look` wrote has its look keys removed so the layer shows
through (`look-install-sh`). That also makes `--look` work on an existing
install, which today it cannot (`install.sh:411-422`).

## 7. The tool and the integration surface

### 7.1 `scoot look` (a subcommand of the existing CLI)

A subcommand, not a new user-facing binary: `scoot` is already the client
on every platform (`msg/index.md:40-41`). (The crate's small `scoot-look`
binary exists for Nix builds and the bar-only path, §3.3, §5.5.)

| Command | Does | Ticket |
|---|---|---|
| `list`, `current`, `show NAME` | catalog, the active look and shadowed keys, tokens and contrast | `look-apply-cli` |
| `validate DIR\|NAME`, `build DIR --out OUT` | §3.5; render without applying | `look-apply-cli` |
| `apply NAME` | §4.5 | `look-apply-cli` |
| `next`, `prev`, `random`, `apply --for SECONDS` / `--keep` | cycling and previews (§4.6) | `look-cycle-preview` |
| `sync` | §6.3 | `look-nix-catalog` |
| `new NAME --from BASE`, `watch DIR`, `preview --capture` | §8.1 | `look-authoring` |
| `import-base16 FILE [--extends LOOK]` | §5.3 | `look-import-base16` |
| `install PATH`, `export NAME`, `save NAME` | §8.3 | `look-install` |
| `install GIT-URL` | later (D12) | `look-share` |

`new --from BASE` writes a source look under `user-looks/` with `extends =
"BASE"`, where BASE must be a *source* look (shipped, declared or
user-installed); it never copies a rendered look. Exit codes follow
`scoot msg`'s; every command takes `--json`.

### 7.2 IPC, for agents and for a Mac driving a VM

The `scoot` binary on a Mac is a client only and drives a compositor in a
VM over the socket (`msg/index.md:40-41`). A swap that only touches local
files cannot be reached that way. So IPC gains `Request::LookApply { name,
for_ms }`, `Request::Looks`, `Response::LookApplied { name, applied,
refused, shadowed }` and `EventKind::Look`.

- The compositor validates `name` against `[a-z0-9-]{1,32}` and never
  accepts a path.
- It never renders on its loop thread: it spawns `scoot look apply NAME
  --from-compositor` as a child (the scootbg run's pattern,
  `wallpaper.rs:219-243`) with its own `SCOOT_LOOK_STATE` (§4.2), and
  answers when the child exits, off the loop like `WaitIdle`
  (`ipc.rs:65,725`). One swap at a time; a second request is refused.
- `EventKind::Look` is emitted by scoot's reload when the layer's identity
  changes (§4.5), so every kind of swap produces it.
- Rejected: `Request::Action(Spawn "scoot look apply …")`, which exists
  today but answers before the swap and never says whether it refused.
- Scoot-specific by the rule in `CLAUDE.md`: no Wayland protocol selects a
  desktop theme, and the portal's Settings interface is read-only for
  clients.

### 7.3 Bar, keybinding, launcher, notifications

- **A keybinding:** `Mod+Shift+T` → `spawn scoot look next` (D8), inert
  while locked (§4.9). Since "next" is otherwise invisible, the bar's
  `looks` module or scootnotify's toast shows the new name.
- **A bar button** works today: `on-click = { exec = ["scoot", "look",
  "next"] }` (`config/bindings.rs:6-18,123-140`).
- **A bar picker popup** needs a built-in `looks` module implementing
  `Module::popup` like `power` (`modules/mod.rs:206-232`,
  `power/mod.rs:76-77`), because `[button.*]` cannot open one
  (`config/bindings.rs:86-97`). At most 16 widgets per popup
  (`site/.../scootbar/modules.md:354`), so a long catalog pages. A swap
  reload closes the popup (`surfaces.rs:260-266`): the expected end of a
  pick.
- **scootlaunch** (optional): `scoot look list | scootlaunch --dmenu |
  xargs scoot look apply`, shipped as a profile script.
- **scootnotify** (optional): a "Look: moonrise" toast on `EventKind::Look`,
  drawn in the new look since it reloads first.

### 7.4 Wallpaper and cursor coupling

A look carries its wallpaper (`[wallpaper]`, one unit) and its cursor
(`[cursor]` theme, size, accent). Both flow through the compositor's reload
(`reload.rs:411-455`, `125-133`). A user who sets any `[wallpaper]` key
keeps their whole wallpaper section across every swap; the cursor keys merge
per key, so a user `cursor_size` alone keeps the look's theme and accent
(§4.3).

The cursor tokens follow PR #572 (open at the time of writing): there,
`cursor_color` becomes the near-black *body* (`#1a1a1a`) and a new
`cursor_accent` (`#e57f29`) the accent. A look's `[cursor] accent` maps to
`cursor_accent` (default `focus`); a look leaves the body alone unless it
sets `[cursor] body` explicitly, so no look paints an amber body. Until
#572 lands, the renderer emits no cursor color at all.

### 7.5 Automatic switching (opt-in, later)

`scoot look auto --light A --dark B` with `--schedule`, `--sun` or
`--follow-portal`, as systemd user timers or a path unit, never a daemon;
off by default. Later, not v1.

### 7.6 Per-output looks

A non-goal for now: the compositor has one `Appearance`
(`reload.rs:371-373`) and the bar one `Theme` per process. A stretch later:
scootbg already supports per-output wallpapers
(`crates/scootbg/src/section.rs:11-31`).

## 8. Authoring, sharing and the trust boundary

### 8.1 The loop

1. `scoot look new moonrise-green --from moonrise` writes
   `user-looks/moonrise-green/look.toml` with `extends = "moonrise"`.
2. `scoot look watch user-looks/moonrise-green` validates on every save and
   applies when valid, debounced on `close_write`/`moved_to` (which answers
   `sighup.rs:5-7`'s mid-keystroke objection); an invalid save is refused
   loudly and the last good look stays.
3. `scoot look apply moonrise --for 20` previews another look and reverts.
4. A nested preview (`scoot --nested --private-look`, §4.2) tries a look
   without touching the running desktop.

### 8.2 Inheritance

`extends` names a source look. Every token is inherited; `[apps.*]` merges
per key; the `[wallpaper]` unit, `preview`, `fonts.files` and `apps/`
leaves are inherited unless the child names its own. Resolution is in the
crate, cycle-checked and depth-limited (§3.5). `extends` resolves to a
shipped, declared or user-authored look of that name (so looks made with
`new` chain), never to an installed one (§3.1).

### 8.3 The trust boundary

A look is never code: keys (§3.4) and values (§3.5) are both bounded. On
top of that, for anything arriving from elsewhere:

- **`install PATH`** (v1): copies **regular files only**; a symlink, device,
  FIFO or socket anywhere in the source refuses the install (a symlink to
  `~/.ssh/id_ed25519` would otherwise land in the catalog and later in
  `export`). Every referenced path is resolved and must stay inside the look
  directory. File count and total size are capped. **Only the file kinds a
  look uses are copied**, by name and place, and anything else refuses the
  install by name (so a look directory cannot carry, say, a `.so` into the
  catalog): `look.toml`; `preview.png`; wallpaper images ending in `.png`,
  `.jpg`, `.jpeg`, `.webp` or `.gif` (scootbg's decoders,
  `crates/scootbg/Cargo.toml:56-60`); `fonts/*.ttf` and `fonts/*.otf`;
  `apps/foot.ini`, `apps/helix.toml`, `apps/btop.theme`,
  `apps/starship.toml`, `apps/regreet.css`; `LICENSE*`, `COPYING*`,
  `README.md`.
- **Images:** byte and pixel caps checked from the header before decoding
  (scootbg's limits), then a full decode in `scootbg check-image`.
- **Trust comes from origin, not name:** an installed look lands in
  `installed-looks/`, may not take the name of a shipped, declared or
  user-authored look, and `extends` never resolves to it (§3.1, §8.2); a
  look the user made with `new`, `import-base16` or `save` is the user's
  own and can be extended. `apps/` static leaves (foot, starship, Helix and btop
  configs can run commands) are dropped from an installed look unless
  `--trust` is given, and `--trust` prints them first. `license` is
  required.
- **`install GIT-URL`** (later, D12): `git clone --depth 1
  --no-recurse-submodules -- URL` (the `--` before the URL, so a URL cannot
  be an option), `https://` and `ssh://` only, then the `install PATH` path
  on the result.
- **Wallpaper by URL** (the vinyl-sunset case, `theming.md:31-34`): a look
  may reference `https://` plus a required `sha256` (the URL grammar of
  §3.5), which scootbg fetches once and caches
  (`site/.../scootbg/from-url.md`). For a shipped, declared or
  user-authored look the first swap to it makes that fetch, and the apply
  reply says so. For an *installed* look, `install` fetches and verifies the
  image into the cache at install time and refuses the install if the fetch
  fails, so a swap to someone else's look never makes a network request.

### 8.4 Tests

- **CI on every change to `looks/`** (D1's path filter): every shipped look
  validates and renders, as a `scoot-look` test over the directory (no CLI
  needed), in `look-shipped`. That makes `theming.md:208-209` true.
- **Golden screenshots per look** from the headless harness, with a private
  `XDG_STATE_HOME`/`XDG_RUNTIME_DIR` (§4.2).
- **Swap tests:** atomicity, refuse-and-keep, rollback, scope (a pending
  user edit is not applied by a swap; a pending `[autostart]` entry does not
  spawn), the wallpaper unit, per-key cursor keys, both derived pairs in
  both directions, shadowed and invalid-user keys, the lock and
  the `--for` generation check, and each corner of §6.3.
- **Value grammar tests** per renderer: a newline, `=`, `[` and a quote in
  every string token refused by the grammar, and quoted correctly by the
  renderer if the grammar were bypassed.

## 9. The contract for scootlaunch, scootnotify and scootgreet

This is what each planned app implements; what each app *does* is its own
plan (`dev/design/shell-apps.md`, `scootnotify.md`, `scootlaunch.md` and
`scootgreet.md`, PR #579 on `docs/plans-scootlaunch-scootnotify`, read at
`cdadd787` and its report at `302c48f6`). That plan adopted this section as
its theming contract; where the first drafts disagreed, this section is the
decision:

1. **Tokens: the single list in §3.2.** Their `selection`,
   `selection-text` and `match` are in it, with their look defaults
   (selection = accent, selection-text = background via `accent-fg`, match =
   hover) and their Stylix slots (base02, base05, base0A). Their `link` and
   `error` capability names map to `accent` and `urgent`.
2. **Border color: `focus`,** which is what mako and fuzzel get today
   (`appearance.focus_ring_active_color`) and what their table names as the
   look's border source. scootnotify's critical border is `urgent`, its
   low-urgency border `border`.
3. **Fonts, sizes, opacity:** `fonts.ui` + `fonts.popup-size` for
   scootnotify and scootlaunch, `fonts.sans` + `fonts.app-size` for
   scootgreet; `opacity.popups` for scootnotify's and scootlaunch's
   background, scootgreet opaque. Stylix's `sizes.popups`,
   `sizes.applications` and `opacity.popups` reach them through the
   `stylix` look (§5.3). Font files come from the look's carrier (D13); the
   shell-apps plan's shared `font-file.nix` helper becomes the Nix side of
   that carrier.
4. **Precedence: the apps never read Stylix.** Stylix reaches them only as
   the synthesized `stylix` look. There are no `programs.<app>.stylix.enable`
   options and no per-leaf Stylix `mkDefault` in their modules; their Nix
   modules write the user's own settings and nothing look-derived.
5. **Look files:** no per-app full configs (`notify.toml`, `launch.toml`,
   `greet.toml`) carry colors in `docs/examples/<look>/`. A look's colors
   reach the apps as rendered fragments from `look.toml`. Example configs for
   layout (anchor, width, timeouts) may ship beside the bar's, with no
   colors in them.
6. **Read the look layer** at `<state>/look/<fragment>` (the names in
   §3.3: `notify.toml`, `launch.toml`, `greet.toml`), merged under the
   app's own config per §4.3, allowlist enforced.
7. **A long-running app (scootnotify) implements `reload`** on its control
   socket with whole-file validation, keep-last-good, an applied/refused
   reply, and `scope = "look"` (§4.4). **A per-launch app (scootlaunch)**
   reads the link at launch. **The greeter** reads a look at system build
   time only (§6.4).
8. **Optional hooks:** `EventKind::Look` for a toast; a `Look:` dmenu list.

Agreed in both plans already: reload through `<app> msg reload`, no SIGHUP,
no file watch in the apps, scootlaunch needs no signal, the greeter is
static and root-owned, and `X-Reload-Triggers` for scootnotify.

**The five asks of the shell-apps plan (PR #579), decided:**

| Ask | Decision |
|---|---|
| `focus` in the notify and launch fragments | **Accepted.** Both fragments carry all thirteen roles; borders are `focus`, so the two looks whose ring differs from the accent keep today's mako and fuzzel border colors. |
| popup font size and popup opacity | **Accepted** as `fonts.popup-size` and `opacity.popups`, mapped from Stylix `sizes.popups` and `opacity.popups` by the one importer (§5.3). |
| fragment names and the override convention | **Accepted:** `notify.toml`, `launch.toml`, `greet.toml`; `[apps.scootnotify]`, `[apps.scootlaunch]`, `[apps.scootgreet]` with the convention above. |
| a greeter fragment | **Accepted:** `greet.toml`, rendered at system build time from the greeter's look (§6.4); applying it is the root-level rebuild, never a user swap. |
| a text color paired with `selection` | **Accepted** as `selection-text` (default `accent-fg`; Stylix base05). |

**What a Stylix user's scootnotify and scootlaunch draw.** Through the
`stylix` look they draw what Stylix's own mako and fuzzel targets draw:
cards on base00 (surface = base00), borders base0D (focus), selection
base02 with base05 text, match base0A. The difference the shell-apps plan
recorded (accent selection, base01 cards) came from this design's first
draft and no longer exists. With a non-Stylix look the apps draw what that
look's mako and fuzzel draw today (selection = accent with background text,
match = hover). This is consistent with D14: the look owns the first-party
apps always; only paths an upstream module writes stay that module's.

## 10. Order of implementation

**The minimum slice** proves the architecture end to end on a non-Nix box
with no Nix change: `look-format` → `look-shipped`, `look-layer-scoot` →
`look-scoped-reload`, and `look-layer-scootbar` → `look-apply-cli` (apply,
list, current, validate, with the lock, rollback and shadowed keys).

**Dogfooding before `look-nix-catalog`:** on a home-manager machine the
eval-time merges write look and Stylix values into the user's own
`config.toml` and `bar.toml` (`home.nix:532-539,571-608`,
`scootbar.nix:374-378,479-496`), and those shadow every look (§4.3). The
first slice's acceptance therefore runs on a non-Nix VM. The maintainer can
dogfood it on the M2 by setting `programs.scoot.desktop.look = null`,
`programs.scoot.stylix.enable = false` and `programs.scootbar.stylix.enable
= false` for the duration (no look or Stylix keys then reach the files), and
`scoot look current` lists anything still shadowing. The cost of those
settings: with `desktop.look = null` the profile's theme slot is inert too
(`themeOn = theme.enable && look != null`, `theme-home.nix:37`), so GTK, Qt, fontconfig and
foot lose their theming while dogfooding; that is expected, and it returns
with `look-nix-catalog`. `look-apply-cli` documents both.

| # | Slug | Area | Priority | Serves | Blocked by |
|---|---|---|---|---|---|
| 1 | `look-format` | core | high | both | |
| 2 | `look-shipped` | core | high | both | `look-format` |
| 3 | `look-layer-scoot` | core | high | both | `look-format` |
| 3a | `look-scoped-reload` | core | high | both | `look-layer-scoot` |
| 3b | `look-session-state` | core | medium | computer use | `look-layer-scoot` |
| 4 | `look-layer-scootbar` | scootbar | high | daily-drive | `look-format` |
| 5 | `scootbar-style-reload` | scootbar | medium | daily-drive | |
| 6 | `look-apply-cli` | core | high | both | `look-shipped`, `look-scoped-reload`, `look-layer-scootbar` |
| 7 | `look-cycle-preview` | core | medium | daily-drive | `look-apply-cli` |
| 8 | `look-ipc` | ipc | medium | computer use | `look-apply-cli`, `look-session-state` |
| 9 | `look-golden-screenshots` | testing | medium | computer use | `look-ipc` |
| 10 | `look-nix-catalog` | packaging | high | daily-drive | `look-apply-cli` |
| 11 | `look-import-base16` | core | medium | daily-drive | `look-format` |
| 12 | `look-stylix-source` | packaging | high | daily-drive | `look-nix-catalog`, `look-import-base16` |
| 13 | `look-toolkit-live` | packaging | medium | daily-drive | `look-apply-cli` |
| 14 | `look-leaves-daemons` | packaging | medium | daily-drive | `look-apply-cli` |
| 15 | `look-leaves-launchers` | packaging | medium | daily-drive | `look-apply-cli` |
| 16 | `look-leaves-terminal` | packaging | medium | daily-drive | `look-apply-cli` |
| 17 | `look-greeter` | packaging | low | daily-drive | `look-nix-catalog` |
| 18 | `look-nix-retire-registry` | packaging | medium | daily-drive | `look-nix-catalog`, `look-stylix-source`, `look-toolkit-live`, `look-leaves-daemons`, `look-leaves-launchers`, `look-leaves-terminal`, `look-greeter` |
| 19 | `look-picker` | scootbar | medium | daily-drive | `look-apply-cli` |
| 20 | `look-authoring` | core | medium | daily-drive | `look-apply-cli` |
| 21 | `look-install` | core | low | daily-drive | `look-authoring` |
| 22 | `look-install-sh` | packaging | medium | daily-drive | `look-apply-cli` |
| 23 | `look-share` (later) | core | low | daily-drive | `look-install` |
| 24 | `look-auto-switch` (later) | core | low | daily-drive | `look-toolkit-live` |

The epic is [`live-looks`](../../docs/backlog/core/live-looks.md); each
ticket names the evidence it must record. "Later" entries carry milestone
`later`: they are filed so the design has a home, not scheduled.

## 11. Risks and non-goals

- **Running GTK and Qt apps do not take a new palette.** Polarity, theme
  name, icons, cursor and font reach running GTK apps through dconf and the
  portal; `gtk.css` colors and the libadwaita palette reach the next launch,
  and Qt's style variant needs a restart. The reply says so. A theme-name
  flip that might recolor running GTK 3 and plain GTK 4 apps
  (`gtksettings.c:1713-1722`) is inferred, untested, and does nothing for
  libadwaita: a research step in `look-toolkit-live`, not a promise.
- **Running terminals keep their palette** (foot cannot re-read its config).
- **Third-party processes cannot be transacted** (§4.5).
- **A look is never code** only while both the key allowlist and the value
  grammar hold, and each renderer quotes; a loader that trusts the tool, or
  a renderer that skips quoting, reopens it.
- **A split desktop under Stylix targets** (D14) is deliberate and named in
  the reply.
- **The login screen stays static** (§6.4).
- **No per-window looks, no per-output colors, no high-contrast look in v1**
  (§3.5, §7.6).
- **Two sources of truth during migration:** until
  `look-nix-retire-registry`, the Nix registry and the look directories
  coexist; `look-shipped` adds a check that they agree.
- **Fonts** are the real catalog cost once looks differ (D13), not images
  (the shipped wallpapers total about 2.1 MB).

## 12. Decisions needed

Each with the recommendation this design is written against. D1, D6 and D8
are recommended defaults the agents can take without the maintainer; the
rest want the maintainer's call.

- **D1 (default, no call needed). Where shipped looks live.** A top-level
  `looks/<name>/` with its own CI path filter; example configs stay under
  `docs/examples/`. Today a change under `docs/` runs no CI beyond the
  ReGreet gate (`ci.yml:236-246`).
- **D2. Where look selection lives in Nix.** `programs.scoot.look` and
  `programs.scoot.looks` (top level), read by the profile, the bar module
  and the greeter, instead of `programs.scoot.desktop.look`. Breaking, clean.
- **D3. May a look carry module lists?** No. Colors, geometry (minus the
  bar's `Bar` keys), fonts, opacity, wallpaper, cursor and allowlisted
  per-module visual keys (glyph strings included, under the glyph grammar);
  module lists, `[button.*]`/`[exec.*]`/`[push.*]` and every `on-*` stay
  user layout.
- **D4. The default catalog.** Every shipped look plus `stylix` when Stylix
  is on.
- **D5. Stylix's role.** A synthesized `stylix` look and the default when on
  (§5), the apps never reading Stylix, with the visible changes of §5.5.
- **D6 (default, no call needed). A central look registry.** None.
- **D7. Gap in a look.** **Not in v1** (the review's counter, adopted): gap
  is layout, a change reconfigures every window (a cost in every client's
  re-render, which a compositor-side benchmark would not show) and
  re-clamps the ring. Adding it later is additive; removing it would be
  breaking. The shipped examples' gaps (18/20/22) stay in their layout
  configs.
- **D8 (default, no call needed). The look-cycle keybinding.**
  `Mod+Shift+T` → `scoot look next`, paired with visible feedback.
- **D9. The shell-apps plan (PR #579).** Its theming follows §9 of this
  design (one token list, the apps never reading Stylix, no per-app Stylix
  options, no colored per-app example configs), which that plan has
  adopted; its five asks are decided in §9. The maintainer confirms the
  precedence once for both plans.
- **D10. Per display or per user.** Persisted per user for `--tty` and
  `--nested` (the webtop deployment and the "try it first" path are
  daily-drive sessions whose swaps must survive a restart); private per
  display only for `--headless`, or any backend given `--private-look`,
  seeded from the user's choice (§4.2). The default path is deterministic,
  so no environment variable has to reach systemd units.
- **D11. Who owns dconf and the GTK/Qt files.** The look tool owns the dconf
  keys of §4.7 (home-manager stops writing `color-scheme` from an eval-time
  look, `theme-home.nix:628-631`), except a key the user sets in
  `dconf.settings` themselves, which home-manager keeps and `sync` leaves
  alone. `gtk.css` and `qt6ct.conf` stay scoot's stable indirection files
  unless `gtk.enable`/`qt.enable` own those paths (today's rule).
  **Mechanics, two layers of the same rule ("never overwrite a value
  someone else set"):**
  - *Nix:* once scoot's modules stop defining any
    `org/gnome/desktop/interface` key, every key left in the merged
    `config.dconf.settings."org/gnome/desktop/interface"` was set by the user
    or another module. The home-manager module writes those key *names*
    (not values) into the catalog's metadata file beside `look-default`, and
    `sync` and `apply` skip them. This reads the merged option at
    evaluation without a cycle, because scoot no longer writes to it.
  - *Everywhere:* `<state>/look.json` records the value the tool last wrote
    for each key. Before writing, the tool reads the current value; if it is
    neither unset nor what the tool last wrote, someone else changed it
    (a settings app, a hand `gsettings set`), so the tool leaves it and the
    reply names it. **With no record yet** for a key (the first `sync` or
    `apply` after migration) and no claim on it in the Nix key-name list,
    the tool adopts the key: it writes it and starts the record, whatever
    the current value is. **This is the one exception to "never overwrite a
    value someone else set" (F3):** on a non-Nix box there is no Nix list,
    so the first `apply` overwrites a `color-scheme` the user chose in a
    settings app. The opt-out is `scoot look apply --no-toolkit` (and a
    persistent `toolkit = false` in `<state>/look.json`, set by `scoot look
    toolkit off`), which skips every dconf write; the docs say so where the
    first apply is described. On a home-manager machine before
    `look-nix-catalog`, home-manager still owns `color-scheme` and restores
    its own value at the next activation; the dogfooding settings (§10:
    `desktop.look = null`) stop home-manager writing it at all.
  - *Why adoption is needed (R1):* the migration switch normally clears
    the old value itself. Home-manager records each generation's managed
    dconf keys and runs `dconf reset` on every key the new generation stops
    managing (`modules/misc/dconf.nix:20,120,156` at `efa3ccb4`, the rev
    `scripts/scootbar-stylix-test.sh:35` pins), and `sync` runs after that
    `dconfSettings` step, so the profile's old eval-time `color-scheme`
    arrives unset. But that cleanup skips an activation that has no
    old-generation state ("Can't do cleanup if we don't know the old
    state", `dconf.nix:143`), which would leave the stale `prefer-dark` in
    place; without adoption the "someone else set it" rule would then lock
    the key forever.
  - *No regression in between (N3, R2):* `look-nix-catalog` stops
    home-manager's eval-time `color-scheme` write only by moving that one
    write into `sync` (run after `dconfSettings`), from the active look's
    `polarity`, so a dark look never loses GTK's dark-mode signal before
    `look-toolkit-live` adds the other keys and the swap-time writes. Until
    then `scoot look apply` does the same one-key `color-scheme` write as
    `sync` (from `look-apply-cli` on, under the same rules), so a live swap
    across polarity (to music-desk and back) never leaves dark/light stale
    until the next activation.
- **D12. Git install in v1?** No: path install in v1, git and `lookPacks`
  later (`look-share`).
- **D13. How a look carries fonts.** Three carriers, in order:
  `[fonts.nix]` attribute paths resolved against `pkgs` at evaluation (the
  build inputs of the render derivation, which a sandboxed build needs);
  `[fonts.files]` bundled in the look (license required); else the family
  name resolved by `fc-match` on a non-Nix box at every apply and at
  session start, with the render cache keyed by the resolved file's path and
  mtime, so a font package update re-renders instead of leaving the bar
  pointing at a vanished file.
- **D15. Do the shipped looks carry the examples' shape and opacity?**
  Recommend yes: the looks are what the previews show and what non-Nix
  users already run, and a look must behave the same on both paths. The
  cost (rounded corners per frame, an ARGB bar) is the look author's choice
  and measured by `look-layer-scoot` and `look-layer-scootbar`; every Nix
  profile user sees the change once (§5.5), with a two-key opt-out. The
  alternative keeps every costly part opt-in (`docs/examples/*/scoot.toml`'s
  "nothing here is a default" rule) by leaving `[shape]` and `[opacity]`
  out of the shipped looks, so Nix users see no change and non-Nix users
  lose them on migration instead. **Condition (from the re-review):** the
  shapes ship only if `look-layer-scoot`'s benchmark shows the per-frame
  cost of rounded corners acceptable on the default CPU (pixman) renderer
  at 1, 10 and 50 windows; if not, the shipped looks keep `[shape] radius`
  at 0 and the docs state the trade-off. **Ordering (R3):** `look-shipped`
  runs before that benchmark, so it lifts `[shape] radius = 0` and
  `[opacity] bar = 1.0` with the examples' values beside them as comments,
  and `look-layer-scoot` raises them to the examples' values in the same
  PR as its pixman benchmark, only if the benchmark passes.
- **D14. Stylix targets vs an explicit look.** Ownership by path (§5.5):
  paths an upstream module (and so a Stylix target) owns stay Stylix's; the
  reply names those apps.

## 13. Deferred from the review

Doc-fixable items were all addressed above. These are deferred, each with
its reason:

- **A full high-contrast look and the portal `contrast` key:** a non-goal
  for v1 (§11); `look-toolkit-live` lists the key as a follow-up.
- **Promoting `urgent ≥ 3.0` to a refusal:** waits for the maintainer's call
  on retuning music-desk (2.88), surfaced by `look-shipped`.
- **The theme-name flip for running GTK apps:** research inside
  `look-toolkit-live`; not designed further until measured.
- **A live OSC push into running foot terminals:** research inside
  `look-leaves-terminal` (it can interleave with a program's own output).
- **Moving the cursor rebuild off the loop thread:** designed as a
  conditional in §4.10, decided by `look-layer-scoot`'s release-build
  measurement rather than in advance.

## 14. What this could not verify

- **Third-party rows in §2** were read from source and man pages at the
  pinned versions, never run live. Inferred and unverified: GTK 4's
  `settings.ini` shadowed by dconf under portal-gtk, the Qt widget palette
  override by adwaita-qt, ReGreet, whether an in-place edit or a
  home-manager relink fires qt6ct's directory watcher, fontconfig with
  epoch-mtime store files, and the GTK theme-name flip.
- **Injection vectors** (§3.4) were checked in the pinned sources' man pages
  and code, not exploited.
- **Stylix reads beyond the stub** (base01/02/04/06/07/09-0F,
  `fonts.monospace`, `sizes.popups`/`applications`/`terminal`, `opacity`,
  `polarity`): Stylix is not a flake input, so there is no pinned source.
  `look-stylix-source` hand-checks each against a named rev. The
  shell-apps plan quotes Stylix's mako and fuzzel targets at `fb28acd`; those
  were not re-read here. Whether Stylix's gtk/foot/mako targets write
  through the upstream modules (D14's premise) is from `theming.md:38-44`
  and the upstream-ownership guards, not from Stylix source.
- **PR #572's cursor numbers** are its own debug-build measurements.
- **The corrupt-image path through `apply-config`** (old pixels or blank) was
  not traced; `scootbg check-image` before a swap makes it moot for looks.
- **The shell-apps plan** was read at `cdadd787` (its theming section) and
  through its report at `302c48f6` (PR #579); the re-alignment to this
  contract is that PR's to make, and was not re-read here after it.
