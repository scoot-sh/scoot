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
8. [Authoring and sharing](#8-authoring-and-sharing)
9. [The contract for scootlaunch, scootnotify and scootgreet](#9-the-contract-for-scootlaunch-scootnotify-and-scootgreet)
10. [Order of implementation](#10-order-of-implementation)
11. [Risks and non-goals](#11-risks-and-non-goals)
12. [Decisions needed](#12-decisions-needed)
13. [What this could not verify](#13-what-this-could-not-verify)

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
| scoot `[autostart]`, `[binds]`, `[[window_rule]]` | live (spawn delta, swapped binds) | unaffected by a look by design (§3.4) | `reload.rs:68-76,101-123` |
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

### 3.1 A look is a directory

```
looks/moonrise/
  look.toml          # the manifest: tokens, metadata, per-app overrides
  wallpaper.png      # optional, referenced from look.toml
  preview.png        # optional, generated by `scoot look preview --capture`
  apps/              # optional static leaves, trusted looks only (§8.3)
    foot.ini  helix.toml  btop.theme  starship.toml  regreet.css
```

The same directory serves Nix (copied into the store and rendered there)
and everything else (`scoot look install`, `install.sh`). The shipped looks
move from `docs/examples/<look>/` to a top-level `looks/<look>/` (Decision
D1), so a look change runs CI.

### 3.2 `look.toml`

```toml
schema = 1
name = "moonrise"                 # [a-z0-9-]{1,32}; the file name of the dir must match
title = "Moonrise"
description = "Slate navy, a disc-amber ring and the night-sky illustration."
polarity = "dark"                 # "dark" | "light": GTK/Qt variant, portal color-scheme
extends = "ginger-night"          # optional: inherit every token, override below
license = "MIT"                   # SPDX; required for anything shipped or installed
credits = ["Illustration: ..."]   # free text, shown by `scoot look show`
preview = "preview.png"

[palette]                         # #rrggbb; required roles marked *
background = "#2B3648"            # * the desktop field, the bar body
surface    = "#2B3648"            #   raised panels (popups, cards); default = background
foreground = "#F6EEDC"            # * body text
dim        = "#9C8B95"            # * secondary text, separators
accent     = "#FFA45C"            # * highlights, selection fill
accent-fg  = "#2B3648"            #   text on accent; default = background
hover      = "#FFD54A"            #   default = accent
urgent     = "#E87F6A"            # * alerts
border     = "#5E4B5B"            #   unfocused frames; default = dim
focus      = "#FF9A49"            #   the focused window ring; default = accent
selection  = "#FFA45C"            #   text selection; default = accent

[terminal]                        # optional: the 16 ANSI hues, hand-placed
colors = ["#1a1512", "#c96a45", ...]   # exactly 16
alpha = 0.80

[fonts]
ui   = "DroidSansM Nerd Font Propo"   # the bar face (module glyphs)
sans = "DejaVu Sans"                  # GTK/Qt/greeter UI text
mono = "FiraCode Nerd Font"           # terminals, editors
size = 10                              # points; bar pixels = (pt * 4 + 1) / 3

[shape]
radius = 14             # window corners, bar, popups
ring-width = 2
ring-inactive-width = 2
gap = 20                # optional; a change re-arranges every window (§4.6)
bar-opacity = 0.82

[cursor]
theme = "Vanilla-DMZ"
size = 24
color = "#FF9A49"       # the compositor's fallback cursor; default = focus

[wallpaper]
image = "wallpaper.png" # relative to the look dir, made absolute when rendered
mode = "fill"           # fill | fit | stretch | center | tile
fill = "#2B3648"        # letterbox color; default = background
transition = "none"     # none | fade | wipe | grow (§4.6 on 4K fades)

[icons]
theme = "Adwaita"

[night]
temperature = 3400

[apps.scootbar]         # per-app overrides, only where the derivation is wrong
"workspaces.inactive-color" = "#B595AD"
```

The roles are the six the bar already has (`theme.rs:16-32`, which
`appColors` already maps to apps: `theme-look.nix:49-59`) plus five the
shipped data proves are separate hues: the inactive ring is not `dim` on
any look (ginger-night: ring `#4E2913`, dim `#8F7A63`;
`desktop.nix:2108,2115`), the active ring is not the bar accent on four of
five (ginger-night `#FF9A30` vs `#E57F29`, `desktop.nix:2107,2113`), and the
GTK/Qt renderers already need surface, accent-fg and selection. Every
optional role has a default derived from a required one, so the five
shipped looks lift without inventing a color.

### 3.3 Derivation: tokens once, per-app leaves only when needed

One library, `scoot-look` (a new pure crate: no Wayland, no I/O beyond
reading the look directory, so it builds on macOS beside `scoot-core`),
owns the schema, `extends` resolution, validation and a pure
`render(look, app) -> fragment` per app. Every consumer reads its fragment;
none re-derives tokens.

| App | Fragment | From |
|---|---|---|
| scoot | `[appearance]` ring colors and widths, `background_color`, `corner_radius`, cursor triple; `[layout] gap`; `[wallpaper]` without `command` | `focus`, `border`, `background`, `shape.*`, `cursor.*`, `wallpaper.*` |
| scootbar | `[colors]` six tokens; `[bar]` `radius`, `popup-radius`, `opacity`, `font`, `font-size`; the per-module visual keys in the allowlist | palette, `shape.radius`, `shape.bar-opacity`, `fonts.ui` resolved to a file, `fonts.size` → px |
| scootbg | the `[wallpaper]` section, through scoot | `wallpaper.*` |
| scootlaunch, scootnotify | the same six bar tokens plus `surface`, `radius`, `fonts.ui` (§9) | palette |
| GTK 3/4 | `gtk.css` named colors and `:root` vars; `settings.ini` | the `appColors` mapping, moved from Nix into the crate |
| Qt (qt6ct) | color scheme, `qt6ct.conf` style by polarity | the same |
| mako, swaylock, wob, fuzzel, fcitx5 | their config formats | the mappings in `notifications-home.nix:216-232`, `idle-home.nix:31-46`, `audio-home.nix:36-75`, `fuzzel-theme.nix:15-21`, `inputmethod-home.nix:43-74`, moved into the crate |
| foot | `[colors-dark]` (or `[colors-light]` for a light look) from `[terminal]`; foot 1.28 has no `[colors]` section any more | the 16 hues; no `[terminal]` table means no foot leaf, never a synthesized palette (`theming.md:98-106`) |
| starship, Helix, btop, ReGreet | static leaves from `apps/` only | trusted looks (§8.3) |

The Nix generators that do this today (`theme-home.nix`, the five app
modules) move into the crate, so there is one implementation of each
mapping, called by `scoot look build` in a derivation for Nix and by
`scoot look apply` everywhere else.

### 3.4 Presentation only: the allowlist is a security boundary

A look may set colors, geometry, fonts, the wallpaper and the cursor. It
may never set a command, a bind, an autostart entry, a module list, an
`exec`/`button`/`push` table, `on-*` actions, `[wallpaper] command`,
`[tty]`, `[renderer]`, `[xwayland]` or `[virtual_input]`. The shipped
`bar.toml` files carry commands (`docs/examples/moonrise/bar.toml:58-64,
76-86`) and scoot's file carries `[autostart]` and `[binds]`; if a look
could set those, `scoot look install <git URL>` would run someone else's
code at the next swap. `validate` refuses a non-allowlisted key by name,
and each app's look-layer loader refuses it again (defense in depth: the
loader never trusts the tool).

The allowlist also excludes every restart-only key (`reload.rs:77-90`), so a
swap can never produce a "takes effect on restart" refusal.

Converting the examples therefore splits each into a look (`look.toml`)
and a layout (module lists, icons, buttons) that stays an example config
or a profile default. Whether a look may carry per-module *visual* keys
(workspace `pill-shape`, `inactive-color`, the module glyphs) is in the
allowlist; whether it may carry module *lists* is Decision D3.

### 3.5 Validation (`scoot look validate`, run before every swap)

Strict, all-or-nothing, and the same code in CI, `install`, `watch` and
`apply`:

- `schema` known; required roles present; every color parses as
  `#rrggbb` (no per-field fallback, unlike `config.rs:194-206`); `extends`
  resolves, no cycle, depth at most 8; names match `[a-z0-9-]{1,32}`.
- Contrast, the thresholds `nix/tests.nix:14907-14932` already enforces on
  `gtk.css`, applied to tokens for every app: foreground on background
  ≥ 4.5, accent-fg on accent ≥ 4.5, accent on background ≥ 3.0, plus
  focus on background ≥ 3.0 (the ring must read). A look below a bar is
  refused, with the pair and the ratio named.
- Wallpaper: exists, is a format scootbg decodes, decodes fully once (the
  result cached by path, size and mtime, so a 6000x4000 JPEG's ~450 ms is
  paid once, not per swap), no larger than scootbg's own limits.
- Fonts: each family resolves (fontconfig on a non-Nix box, the package
  on Nix); the bar face resolves to a file, as `scootbar.nix:62-95` does.
- `license` present for anything installed or shipped; `apps/` leaves
  present only on a trusted look (§8.3); every `[apps.*]` key in the
  allowlist.
- The rendered fragments are checked by the apps' own strict parsers:
  the compositor's config parser in a strict mode, `scootbar daemon
  --check`, and scootbg's strict section parse.

## 4. The runtime: one indirection, one reload protocol

### 4.1 The indirection

```
$XDG_STATE_HOME/scoot/look           -> symlink to the active rendered look
$XDG_STATE_HOME/scoot/look.json      -> { name, rendered, chosen-by, declared-default }
$XDG_STATE_HOME/scoot/rendered/<name>-<hash>/   (non-Nix render cache)
```

A rendered look is a directory of per-app fragments (`scoot.toml`,
`bar.toml`, `gtk/gtk.css`, `qt/scoot-look.conf`, `mako`, `swaylock`,
`fuzzel.ini`, `wob.ini`, `foot.ini`, ...). On Nix it is a store path; on
anything else it is under the render cache. Switching is
`symlink(tmp) + rename(tmp, look)`: one atomic `rename(2)`, so every reader
sees the old directory or the new one, never a mix.

Every consumer points at the link, never at a specific look:

- **scoot and scootbar** read the look layer through the link and merge it
  *under* the user's own file, per key (§5.2). Their config files stay
  the user's; a swap never rewrites them. A dangling or missing link is
  "no look layer" plus a warning at startup (startup never fails over a
  look, the rule at `config.rs:16-23`), and a refused reload at runtime.
- **Per-launch third-party apps** take the link's path in their command
  line or config: `swaylock -C $XDG_STATE_HOME/scoot/look/swaylock`,
  fuzzel `--config` the same way, `gtk.css` as one
  `@import url("file://.../look/gtk/gtk.css")`, mako through its
  `include=`, qt6ct's `color_scheme_path`. A new process reads the new
  look with no hook at all.

### 4.2 The reload protocol: each app's existing control-socket `reload`

**Decision: every first-party app that runs long implements a `reload`
request on its own control socket, re-reads its config and the look layer,
validates both whole, and answers applied/refused. `scoot look apply`
drives those requests in order and aggregates the replies.** This is
exactly what two of the three apps already do:

- scoot: `Request::Reload` on `scoot.sock`, `Response::Reloaded { applied,
  refused }` (`scoot-ipc/src/request.rs:115-122`, `reload.rs:240-271`).
- scootbar: `{"protocol":1,"type":"reload"}` on `scootbar-NAME.sock`,
  `{"type":"ok"}` or an error (`control/protocol.rs:1-17,52`,
  `respond.rs:145-148`).
- scootbg: `apply-config` on `scootbg-NAME.sock`, driven by scoot.

Rejected:

- **SIGHUP as the contract.** scootbar cannot catch it
  (`#![forbid(unsafe_code)]`, `daemon/mod.rs:27-35`), so a HUP kills the
  bar. And a signal has no reply channel, so the tool could not tell a
  refused look from an applied one (`reload.rs:241-247` logs for exactly
  that reason). scoot keeps its HUP as a second trigger.
- **inotify on the link in every app.** `sighup.rs:5-7` rejected watching
  the config because an editor fires it mid-keystroke. The link's atomic
  rename does not have that problem, but a watch per app loses the
  ordering and the aggregated reply, and makes each app decide on its own
  when to read. The dev loop (`scoot look watch`, §8.1) does watch, in the
  tool, debounced, and still drives the same `reload` requests.
- **A new `look.apply` message per app.** The apps need not know what a
  look is beyond reading one more layer; `reload` already means "re-read
  everything you read".
- **Rewriting each app's config file per swap.** Clobbers the user's
  edits and cannot be atomic across files.

### 4.3 Order and the swap transaction

`scoot look apply NAME`:

1. **Render and validate** the target completely (§3.5). Any failure
   refuses loudly; nothing has been touched.
2. **Repoint** the link (one `rename`), remembering the previous target.
3. **Reload, in order:** scoot (ring, background, cursor, gap, wallpaper
   handed to scootbg) → scootbar instances → scootnotify (once it exists)
   → the toolkit side (the dconf keys `color-scheme`, `gtk-theme`,
   `icon-theme`, `cursor-theme`, `cursor-size`, `font-name`; `makoctl
   reload`; fcitx5's `ReloadAddonConfig "classicui"`; `SIGUSR1` to the
   user's own Helix processes and `SIGUSR2` to btop, both their
   documented reload signals; the wob unit restart) → a `look` event to
   IPC subscribers. The compositor first, because it draws behind
   everything else. Every toolkit step is best-effort and reported (a
   missing `dconf` or no fcitx5 is a note, not a failure), because none
   of them can be transacted.
4. **Wait** for scoot's wallpaper run to finish (a reload option added for
   this; today it is fire-and-forget, `reload.rs:125-133`).
5. **On any first-party refusal** (only environmental by now: everything
   was validated): repoint back to the previous target, re-run the reloads
   that already ran, and exit non-zero naming the app and the reason.

"A half-applied look is impossible" is true per first-party app (each
swaps whole or not at all: `reload.rs:273-277`, `respond.rs:10-12`) and
across first-party apps through step 5. It is not true across third-party
processes, which cannot be transacted: a running GTK app that ignores the
portal keeps the old look until it restarts. That is stated, not hidden.

### 4.4 Failure contract (every app)

- An invalid look is refused before the swap, by name and reason.
- Last good is kept: the link never points at an unvalidated directory.
- Never wedge: startup with a broken or dangling link starts without the
  look layer; a reload refuses; nothing exits. A look can never cost the
  user their binds, autostart or session.
- Refusals are loud on both channels: the reply and the log.

### 4.5 Under session lock

A swap applies under lock, like a reload (`reload.rs:143-171`): the ring,
background, cursor and wallpaper are config-derived pixels and disclose
nothing. The tool needs no special case.

### 4.6 Performance

A swap is a rare, cold event. Nothing in it may land on a per-frame path.

- Rendering, font resolution and the wallpaper decode run in the tool's
  process (or at Nix build time), never on the compositor's loop thread.
- The compositor's own share is today's reload: a TOML parse, a diff and
  `apply()` when a ring, background or gap moved (`reload.rs:308-321`).
  `apply()` re-arranges every window (`shell.rs:375-405`), so the ring and
  background fields should request a redraw without `apply()`, as the
  cursor fields already do (`reload.rs:322-331`). A ticket measures it
  before and after.
- scootbar restarts every module on a colors-only reload
  (`modules/mod.rs:615-661`). A style-only reload should keep every
  module's state and only repaint.
- A 4K `fade` drops frames (`transitions.md:110-118`), so a look's default
  transition is `none`.
- Evidence the swap tickets record: frame times across a swap on the
  headless and `--tty` backends (no frame over budget beyond what
  `apply()` costs today), the swap's wall time, and the compositor's
  allocation count on the swap (zero new per-frame allocations after it).

## 5. Stylix: a source of a look, not a layer above it

The maintainer asked: "With all this do we keep stylix or create look
support with custom looks". **Both: looks are the system of record and the
runtime selector; Stylix becomes one source of a look.**

### 5.1 Why the current ladder cannot stay

Today a key is decided at evaluation time: user (100) > Stylix
(`mkDefault`, 1000) > look (`mkOptionDefault`, 1500) > plain default
(`scootbar.nix:7-20`; the same ladder in `home.nix:534,571-608,623-624`).
A Stylix value is baked into `config.toml` and `bar.toml` as an ordinary
setting, which the look layer sits *under* at runtime, so a live swap
could never change any key Stylix set. Keeping Stylix above the look makes
the swap a no-op for every Stylix user, and moving it below makes Stylix
meaningless. Neither works; Stylix has to stop being a layer.

### 5.2 The new ladder

**Build time (home-manager or NixOS evaluation):** what the user wrote in
`programs.scoot.settings`, `programs.scootbar.settings` and the per-slot
`settings` > the module's plain defaults. No look or Stylix value is
merged into those files any more. What evaluation decides about looks is
the catalog (`looks`), the declared default (`look`) and the trust set.

**Runtime (every load and every swap):** the user's own file, per key >
the active look's fragment, per key > the app's built-in default. The
look's own tokens resolve as: the look's `[apps.<app>]` override > the
derived value > the role default (§3.2).

**Where Stylix sits:** inside a look. When Stylix is present and on, the
catalog gains a synthesized look named `stylix`, and the declared default
becomes `stylix` at `mkDefault` (so an explicit `look = "moonrise"` wins).
Swapping away from it and back works like any other look.

### 5.3 One mapping, in one place

The base16 → token mapping is implemented once, in `scoot-look`
(`import-base16`), and used by both paths:

- `scoot look import-base16 scheme.yaml` for anyone with a base16 scheme
  file.
- Nix: one function (`nix/lib/look-from-stylix.nix`) serializes
  `config.lib.stylix.colors.withHashtag.base00..base0F`, the fonts, sizes,
  opacity, cursor and image into a base16-shaped JSON in the store, and a
  derivation runs `scoot look import-base16` on it. No module maps a
  base16 slot itself; the duplicated mappings in `home.nix:571-573` and
  `scootbar.nix:482-487` are deleted.

The mapping keeps today's choices, so a Stylix user sees the same colors
after migrating: `background = base00`, `foreground = base05`,
`accent = focus = hover = base0D`, `dim = border = base03`,
`urgent = base08` (`scootbar.nix:102-111,482-487`;
`docs/backlog/resolved/scoot-stylix-target-done.md:72-81`). It adds
`surface = base01` and `selection = base02`, the base16 styling guide's
meanings. `[terminal]` takes the 16 base16 slots in the conventional
terminal order. Fonts: `sansSerif` → `sans`, `monospace` → `mono`, the bar
face from `sansSerif` as today (`scootbar.nix:493`), sizes through the
existing pt → px rule (`scootbar.nix:97-100`). The wallpaper is
`stylix.image` and `imageScalingMode` when set (`home.nix:587,603-606`),
else a flat `base00`. The cursor is `stylix.cursor` when set
(`home.nix:583-584`).

### 5.4 What Stylix cannot supply, and how the look fills it

| Token | Stylix | Filled by |
|---|---|---|
| `hover` | no role | `base0D`, today's choice (`scootbar.nix:485`) |
| `focus` vs `accent`, `border` vs `dim` | one hue each | the same slot for both, as today |
| `polarity` | `stylix.polarity` exists upstream but is not read or stubbed here | computed from `base00`'s luminance; `stylix.polarity` once verified (§13) |
| `[cursor] color` | no convention (`theming.md:182-183`) | `focus` |
| `[shape]` radius, ring widths, gap | none | the compositor and bar defaults, or `[apps]`/`extends` |
| wallpaper when `stylix.image` is unset | none | flat `background` |
| night temperature | none | the default |
| starship, Helix, btop, ReGreet static leaves | Stylix's own targets theme those apps | left to Stylix's own targets (scoot does not write those files for the `stylix` look) |

### 5.5 What changes for existing Stylix users, and the migration

- **Stylix on, no explicit `look`:** nothing visible changes. The
  `stylix` look is the declared default; its colors, fonts, cursor and
  wallpaper equal what the per-leaf defaults produce today.
- **Stylix on and an explicit `look`:** this is the one behavior flip.
  Today Stylix wins where both define a key (`theming.md:38-44`); after
  this, the look wins because the user picked it. `stylix` stays in the
  catalog, one `scoot look apply stylix` away.
- **Options (breaking; the sole-user rule ships them clean, marked `!`,
  with the maintainer's own `~/nixos-config` updated in the same change):**
  - `programs.scootbar.stylix.enable` is removed: the bar has no Stylix
    path of its own.
  - `programs.scoot.stylix.enable` keeps its name and changes meaning:
    "synthesize the `stylix` look and make it the default when Stylix is
    on" (default true, as now).
  - `programs.scoot.stylix.wallpaper.enable` keeps its name: whether the
    `stylix` look carries `stylix.image`. The `color`-plus-`image` trap it
    fixed (`stylix-wallpaper-color-trap-done.md:27-60`) cannot recur,
    because a look carries one or the other by construction.
  - `programs.scoot.desktop.look` moves to `programs.scoot.look`, with
    `programs.scoot.looks` beside it (Decision D2), so a user with scoot
    and the bar but no desktop profile, who gets Stylix defaults today
    (`theming.md:158-165`), keeps a working path.
- **Published text this supersedes,** each updated by the ticket that
  changes it: `theming.md:38-44` (Stylix > look), `theming.md:158-189`
  (the Stylix table), `theming.md:212-219` (`look` "only ever gains
  values": the enum becomes a catalog, and `look = "auto"` becomes
  "the `stylix` look, or a look imported from your wallpaper"), and
  `site/.../scootbar/theming.md` on Stylix precedence.
- **The NixOS side:** the greeter's Stylix assertion
  (`nixos.nix:86-88,2314`) stays; the greeter takes its look from
  `greeter.look` (§6.4).

### 5.6 What the tests can and cannot vouch for

`nix/scootbar-tests.nix:9-15` says it plainly: Stylix is not this flake's
input, the stub declares only what the module reads, checked by hand
against `nix-community/stylix` at `fb28acd`, and "a rename there would
pass this file and miss on a real Stylix". The stub in `nix/tests.nix:754-807`
declares `stylix.enable`, `cursor`, `image`, `imageScalingMode`,
`fonts.sansSerif`, `fonts.sizes.desktop` and five palette slots (`base00`,
`03`, `05`, `08`, `0D`). The `stylix` look reads more (all sixteen slots,
`fonts.monospace`, `opacity`, `polarity`), so its ticket must check each
new read by hand against a named Stylix rev, extend the stub, record the
rev in the header the way `scootbar-tests.nix` does, and keep the
real-Stylix run `scripts/scootbar-stylix-test.sh` green.

## 6. Nix and non-Nix parity, and the home-manager rule

### 6.1 Options

```nix
programs.scoot = {
  looks = [ "ginger-night" "moonrise" ./my-looks/moonrise-green inputs.someone.scootLooks.dusk ];
  look = "moonrise";      # the declared default: what a fresh state starts on
};
```

- `looks`: names of shipped looks, paths to look directories, or
  derivations producing one. Default: every shipped look, plus `stylix`
  when Stylix is on (Decision D4 on the default catalog size).
- `look`: one name in the catalog, or null for no look layer at all.
- The flake exports `lookPacks.<name>` (a directory of looks) so a repo of
  looks is a flake input: `looks = inputs.mylooks.lookPacks.default`.

### 6.2 What evaluation produces

- Each look is rendered by a derivation running `scoot look build` (the
  same renderer as everywhere else; no import-from-derivation, since the
  modules only reference the output paths). The build validates (§3.5), so
  a broken look fails `home-manager switch` before anything changes.
- The catalog: `~/.local/share/scoot/looks/<name>` → the rendered store
  path, one `xdg.dataFile` link per look. Read-only, garbage-collection
  rooted by the home-manager generation.
- The declared default: `~/.config/scoot/look-default` (a store file
  holding the name).
- The live choice is **not** home-manager's: `$XDG_STATE_HOME/scoot/look`
  and `look.json` are mutable state, and the link targets the *catalog*
  entry (`~/.local/share/scoot/looks/moonrise`), never a raw store path,
  so the next generation's catalog re-resolves it.
- An activation step runs `scoot look sync`, which applies the rule below
  and, when a session is running, drives the reloads.

### 6.3 The rule on `home-manager switch` (and rollback)

`look.json` records the declared default that was in force when the live
choice was made. On every activation, in order:

1. **No state yet** → start on the declared default.
2. **The declared default changed** since the live choice was recorded →
   the declaration wins: switch to the new default. (The user just edited
   their config to say so.)
3. **The live choice is no longer in the catalog** → fall back to the
   declared default, and say so in the activation output.
4. **Otherwise** → keep the live choice; re-resolve it through the new
   catalog, so an updated look's new contents apply.

Corner cases:

- The same look name with changed contents: rule 4 applies the new contents
  (the user updated the look, so they get the update).
- `look = null` in the new generation: rule 2 removes the look layer.
- Rollback (`home-manager` activating an older generation) runs the same
  rule against the older catalog and default. "Rolled back together"
  (`site/.../desktop/index.md`'s promise for the profile) then means: the
  catalog, the declared default and every file roll back; a live choice
  survives a rollback only when the rolled-back generation still offers it
  and declares the same default. A swap made since the last switch is a
  preference, not configuration, and it is not in any generation.
- No session running: the sync updates the link and stops; the next
  session start reads it.

**Rejected: NixOS specialisations as the swap mechanism.** A
specialisation is a whole system activation, root-only, chosen at boot or
by `switch-to-configuration`, and builds one system per look: none of
that is a live, per-user swap. It stays a fine way to pin a *greeter* look
per boot entry, which nothing here prevents.

### 6.4 The greeter is static

The login screen runs before any user session and is owned by NixOS
(`nixos.nix:2420-2477`). It takes `programs.scoot.greeter.look` (default:
the declared `look`), rendered at system build time. A live swap never
touches it. A non-Nix "apply to the greeter" is an explicit, root-level
`sudo scoot look apply --greeter NAME` that writes the greeter's CSS and
background once (a low-priority ticket), never a side effect of a user's
swap.

### 6.5 Non-Nix

`install.sh --look NAME` installs the shipped looks into
`~/.local/share/scoot/looks/` and runs `scoot look apply NAME`, which
renders into the cache and points the link. It stops copying whole
`scoot.toml`/`bar.toml` files over an empty config dir; a user's existing
files are never touched, and the look layer applies under them. That also
makes `--look` work on an existing install, which today it cannot
(`install.sh:411-422`).

## 7. The tool and the integration surface

### 7.1 `scoot look` (a subcommand of the existing CLI)

A subcommand, not a new binary: the `scoot` binary is already the client on
every platform (`msg/index.md:40-41`), and a second small binary would
duplicate its socket client and its packaging. `scoot look` never needs the
compositor running except for `apply`'s reloads.

| Command | Does |
|---|---|
| `list [--json]` | the catalog, the active one marked |
| `current [--json]` | name, source, rendered path |
| `show NAME` | tokens, contrast ratios, license and credits |
| `apply NAME [--for SECONDS]` | the transaction in §4.3; `--for` reverts after the timer unless `apply --keep` arrives (the preview) |
| `next` / `prev` / `random` | over the catalog order |
| `validate DIR\|NAME` | §3.5; the CI check |
| `new NAME --from BASE` | a directory with `extends = BASE` and nothing else |
| `watch DIR` | re-validate and re-apply on save, debounced (§8.1) |
| `save NAME` | capture the running appearance as a new look (the user's own `[appearance]`/`[colors]` values become tokens) |
| `import-base16 FILE` | a look from a base16 scheme (§5.3) |
| `install PATH\|GIT-URL [--trust]` | copy into the catalog after `validate`; static leaves only with `--trust` |
| `export NAME` | a tarball of the directory |
| `build DIR --out OUT` | render without applying; what the Nix derivation runs |
| `sync` | the home-manager rule (§6.3) |

Exit codes follow `scoot msg`'s (`0` ok, non-zero with the reason), and
every command takes `--json` for agents, consistent with the generated CLI
pages (`site/.../reference/cli.md`).

### 7.2 IPC, for agents and for a Mac driving a VM

The `scoot` binary on a Mac is a client only and drives a compositor in a
VM over the socket (`msg/index.md:40-41`). A look swap that only touches
local files cannot be reached that way, and computer use wants it: a test
can pin a look, take `scoot msg screenshot`, and compare against a golden
image. So IPC gains:

- `Request::LookApply { name, for_ms }` and `Request::Looks` (list and
  current), with `Response::LookApplied { name, applied, refused }`.
- `EventKind::Look` on `Subscribe`: `{ name }` after a swap, for the bar,
  scootnotify's optional toast and agents.

The compositor does **not** run the swap on its loop thread: it spawns
`scoot look apply NAME --from-compositor` as a child (like the scootbg run,
`wallpaper.rs:219-243`), and answers the IPC request when the child exits,
the way `WaitIdle` already answers off the loop (`ipc.rs:65,725`). The
child drives the same `reload` request back over the socket on its own
connection. This is scoot-specific by the rule in `CLAUDE.md`: no Wayland
protocol selects a desktop theme, and the portal's Settings interface is
read-only for clients.

### 7.3 Bar, keybinding, launcher, notifications

- **A keybinding:** the default keymap gains `Mod+Shift+T` (or the
  maintainer's pick) bound to `spawn scoot look next`; binds are reloaded
  live (`reload.rs:68-76`).
- **A bar button** works today with no new code:
  `on-click = { exec = ["scoot", "look", "next"] }`
  (`config/bindings.rs:6-18,123-140`).
- **A bar picker popup** needs a built-in `looks` module implementing
  `Module::popup` the way `power` does (`modules/mod.rs:206-232`,
  `power/mod.rs:76-77`), because `[button.*]` cannot open a popup
  (`config/bindings.rs:86-97`). A popup holds at most 16 widgets
  (`site/.../scootbar/modules.md:354`), so a longer catalog pages. The
  bar reload a swap triggers closes any open popup
  (`surfaces.rs:260-266`), so the picker closes on apply; that is the
  expected behavior of a picker, and it is stated.
- **scootlaunch** (optional hook): a `Look:` entry list through its dmenu
  mode, `scoot look list | scootlaunch --dmenu | xargs scoot look apply`,
  shipped as a profile script.
- **scootnotify** (optional hook): subscribes to `EventKind::Look` and
  shows "Look: moonrise", with the look's own colors, since it reloads
  first.

### 7.4 Wallpaper and cursor coupling

A look carries its wallpaper (`[wallpaper]`) and its cursor (`[cursor]`
theme, size and fallback color). Both flow through the compositor's
existing reload paths (`reload.rs:411-455` and `125-133`). A user who sets
`[wallpaper]` or a cursor key in their own file keeps it across every swap
(§5.2).

### 7.5 Automatic switching (opt-in)

`scoot look auto --light music-desk --dark moonrise` with one of:
`--schedule 07:00/19:00`, `--sun` (sunrise and sunset from the night-light
slot's location settings), or `--follow-portal` (when something else sets
`org.freedesktop.appearance color-scheme`, for example a toolkit settings
app). Delivered as a systemd user timer (or a path unit for the portal),
never a new daemon: the swap is a one-shot process. Off by default.

### 7.6 Per-output looks

A non-goal for now: the compositor has one `Appearance`
(`reload.rs:371-373`) and the bar one `Theme` per process. A stretch
later: scootbg already supports per-output wallpapers
(`crates/scootbg/src/section.rs:11-31`), so a look could carry per-output
wallpapers without per-output colors.

## 8. Authoring and sharing

### 8.1 The loop

1. `scoot look new moonrise-green --from moonrise` writes a directory whose
   `look.toml` is `extends = "moonrise"` and `name`.
2. `scoot look watch ~/.local/share/scoot/looks/moonrise-green` validates on
   every save and applies when valid, like a dev server. Debounced on
   `close_write`/`moved_to` with a short quiet window, which answers
   `sighup.rs:5-7`'s mid-keystroke objection. An invalid save is refused
   loudly (stderr, and a notification when scootnotify runs) and the last
   good look stays on screen.
3. `scoot look apply moonrise --for 20` previews another look and reverts
   after 20 s.
4. A nested preview (`scoot --nested` with a scratch state dir and the
   look applied) is the zero-risk option for trying a look without
   touching the running desktop; it costs a nested session and comes after
   the basics.

### 8.2 Inheritance

`extends` names another look in the catalog. Every token is inherited;
`[apps.*]` overrides merge per key; `wallpaper`, `preview` and `apps/`
leaves are inherited unless the child names its own. Resolution is in the
crate, cycle-checked and depth-limited (§3.5).

### 8.3 Sharing, and trust

- A look pack is a git repo or a flake output (`lookPacks`) of look
  directories. There is no central registry (Decision D6).
- `scoot look install <git URL>` clones, validates, and copies into the
  catalog. It refuses `apps/` static leaves without `--trust` and prints
  them, because foot, Helix, starship and btop configs can run commands
  (starship custom modules, a foot `shell=`). Shipped looks are trusted;
  a token-only look needs no trust.
- `license` is required to install. The vinyl-sunset case
  (`theming.md:31-34`: an illustration that may not be redistributed)
  generalizes: a look may reference a wallpaper by URL plus `sha256`, which
  scootbg already fetches and caches (`site/.../scootbg/from-url.md`).

### 8.4 Tests

- **CI: every shipped look validates and renders** (`scoot look validate`
  over `looks/*`), plus the contrast bars, plus a Nix check that every
  catalog look builds. This makes `theming.md:208-209`'s claim true.
- **Golden screenshots per look** from the headless harness: start
  `scoot --headless` with fixed clients, `scoot msg look-apply NAME`,
  `scoot msg screenshot`, compare to a committed golden with a small
  tolerance; regenerate on purpose. CI-safe (headless, the CPU renderer).
- **Swap tests:** an atomicity test (a reader racing a swap sees the old or
  the new directory, never a mix), a refuse-and-keep test (a broken look
  leaves every app on the previous one), a rollback test (a first-party
  refusal after the repoint restores the previous link and reloads), and
  a `home-manager switch` rule test per corner case in §6.3.

## 9. The contract for scootlaunch, scootnotify and scootgreet

This is what each planned app must implement; what each app *does* is its
own plan's business (the coordinating plan was looked for on
`docs/plans-scootlaunch-scootnotify` and was not yet pushed when this was
written; §13).

1. **Read the look layer** through `$XDG_STATE_HOME/scoot/look/<app>.toml`,
   merged under the app's own config per key, with the allowlist enforced.
   No app reads Stylix, a base16 file or the Nix registry directly.
2. **Take the tokens from `scoot-look`**, not a local palette struct. The
   planned shared drawing crate (`docs/scootbar/backlog/extract-scootui.md`,
   whose plan already names "color tokens") builds its theme from the
   `scootbar` token set: background, surface, foreground, dim, accent,
   accent-fg, hover, urgent, plus `radius` and the `ui` face.
3. **A long-running app (scootnotify) implements `reload`** on its control
   socket with whole-file validation, keep-last-good and an
   applied/refused reply. **A per-launch app (scootlaunch)** reads the
   link at launch and needs nothing else.
4. **The greeter** reads a look at system build time only (§6.4).
5. **Optional hooks:** `EventKind::Look` for a toast; a `Look:` dmenu
   list.

## 10. Order of implementation

The first slice proves the architecture end to end on a non-Nix box with no
Nix changes: tokens, the crate, the compositor and bar look layers, and
`scoot look apply` driving their reloads with the wallpaper. Then the IPC
and tests (computer use), then Nix, then the third-party apps, then the
authoring and the extras.

| # | Slug | Area | Priority | Serves | Blocked by |
|---|---|---|---|---|---|
| 1 | `look-format` | core | high | both | |
| 2 | `look-layer-scoot` | core | high | both | `look-format` |
| 3 | `look-layer-scootbar` | scootbar | high | daily-drive | `look-format` |
| 4 | `look-apply-cli` | core | high | both | `look-layer-scoot`, `look-layer-scootbar` |
| 5 | `look-ipc` | ipc | high | computer use | `look-apply-cli` |
| 6 | `look-golden-screenshots` | testing | medium | computer use | `look-ipc` |
| 7 | `look-nix-catalog` | packaging | high | daily-drive | `look-apply-cli` |
| 8 | `look-import-base16` | core | medium | daily-drive | `look-format` |
| 9 | `look-stylix-source` | packaging | high | daily-drive | `look-nix-catalog`, `look-import-base16` |
| 10 | `look-toolkit-live` | packaging | medium | daily-drive | `look-apply-cli` |
| 11 | `look-app-leaves` | packaging | medium | daily-drive | `look-apply-cli` |
| 12 | `look-picker` | scootbar | medium | daily-drive | `look-apply-cli` |
| 13 | `look-authoring` | core | medium | daily-drive | `look-apply-cli` |
| 14 | `look-install-sh` | packaging | medium | daily-drive | `look-apply-cli` |
| 15 | `look-auto-switch` | core | low | daily-drive | `look-toolkit-live` |
| 16 | `look-greeter` | packaging | low | daily-drive | `look-nix-catalog` |

The epic is [`live-looks`](../../docs/backlog/core/live-looks.md); each
ticket names the evidence it must record.

## 11. Risks and non-goals

- **Running GTK and Qt apps do not take a new palette.** Polarity, the
  theme name, icons, the cursor and the font reach running GTK apps
  through dconf and the portal; the colors in `gtk.css` and the
  libadwaita palette reach the next launch, and Qt's style variant needs
  a restart. A swap says so in its reply. A trick that might recolor
  running GTK 3 and plain GTK 4 apps (alternating between two theme names
  so GTK re-reads the theme from disk, `gtksettings.c:1713-1722`) is
  inferred from the code, untested, and does nothing for libadwaita; it is
  a research step in `look-toolkit-live`, not a promise.
- **Running terminals keep their palette** (foot cannot re-read its
  config). New terminals get the new look.
- **Third-party processes cannot be transacted** (§4.3).
- **A look is never code.** The allowlist (§3.4) and `--trust` (§8.3) are
  the boundary; a loader that trusts the tool would reopen it.
- **The login screen stays static** (§6.4).
- **No per-window looks.** No per-output colors yet (§7.6).
- **Two sources of truth during migration.** Until `look-nix-catalog`
  lands, the Nix registry and the look directories coexist; `look-format`
  adds a check that the lifted `look.toml` tokens equal
  `desktop.nix:1931-2147`, so they cannot drift in the meantime.
- **Font files on a non-Nix box** resolve through fontconfig at render
  time; a look naming a face the box lacks is refused by `validate`,
  never rendered with a silent fallback.
- **Store paths and garbage collection:** the state link targets the
  catalog entry, which the home-manager generation roots; a link that
  dangles after a collection reads as "no look layer" with a warning,
  never a startup failure.

## 12. Decisions needed

Each with the recommendation this design is written against.

- **D1. Where shipped looks live.** Recommend a top-level `looks/<name>/`
  with the example configs staying under `docs/examples/`. Today a change
  under `docs/` runs no CI beyond the ReGreet gate
  (`ci.yml:236-246`), so a look that breaks a Nix check merges unchecked.
  A product-data directory gets a CI path filter like scootbg's.
- **D2. Where look selection lives in Nix.** Recommend
  `programs.scoot.look` and `programs.scoot.looks` (top level), read by the
  desktop profile, the bar module and the greeter, instead of
  `programs.scoot.desktop.look`. That keeps the users who have scoot and
  the bar with Stylix but no profile working. Breaking, done cleanly.
- **D3. May a look carry module lists?** Recommend no. A look carries
  colors, geometry, fonts, wallpaper, cursor and the per-module *visual*
  keys; module lists, icons that are commands, buttons and `exec` stay
  user layout. A look that rearranges the bar on every swap is surprising,
  and lists invite commands.
- **D4. The default catalog.** Recommend every shipped look plus `stylix`
  when Stylix is on: five small directories, one shared wallpaper each,
  only the active one's leaves read.
- **D5. Stylix's role (the maintainer's question).** Recommend §5:
  looks are the system of record and the runtime selector, Stylix a
  synthesized `stylix` look and the default when on; user > look > default
  at runtime; the per-module Stylix mappings removed; one behavior flip for
  Stylix-plus-explicit-look users.
- **D6. A central look registry.** Recommend none: git repos and flake
  `lookPacks` only. A registry is a moderation job.
- **D7. Gap in a look.** Recommend yes, optional (every shipped look sets
  `gap = 20`), measured by `look-layer-scoot`'s benchmark; if a swap that
  moves the gap costs a visible hitch, `gap` leaves the schema.
- **D8. The default keybinding** for `scoot look next`. Recommend
  `Mod+Shift+T`, checked against the current default keymap by the
  `look-picker` ticket.

## 13. What this could not verify

- **Third-party rows in §2** were read from source and man pages at the
  pinned versions, never run live. The rows marked *inferred* (GTK 4's
  `settings.ini` shadowed by dconf under portal-gtk, the Qt widget
  palette override by adwaita-qt, ReGreet) and these mechanics are
  unverified: whether editing a file in place under `~/.config/qt6ct`
  fires qt6ct's directory watcher, whether a home-manager relink does,
  whether fontconfig sees a changed conf when store files carry epoch
  mtimes, and the GTK theme-name flip. Each is a first step in the ticket
  that relies on it.
- **Stylix reads beyond the stub** (all sixteen slots,
  `fonts.monospace`, `opacity`, `polarity`): Stylix is not a flake input,
  so there is no pinned source here. `look-stylix-source` checks each by
  hand against a named rev (§5.6).
- **Whether a corrupt image sent through `apply-config` leaves the old
  pixels or a blank output** (`daemon/images.rs:462-484` marks
  `draw_failed`; which buffer stays attached was not traced). `validate`
  decoding the image up front makes it moot for looks.
- **Whether recreating the bar's layer surface on a geometry reload
  blinks** (`surfaces.rs:292-309`): not measured; `look-layer-scootbar`
  measures it.
- **The coordinating plan** for scootlaunch and scootnotify
  (`docs/plans-scootlaunch-scootnotify`) was not on the remote when this
  was written; §9 is the contract it should implement, and its PR should
  link back here.
