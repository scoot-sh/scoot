---
title: Looks
description: "Pick a look, swap it live, and see what changes now and what changes on next launch."
---

List, inspect and swap the desktop's looks. A look is presentation only
(colors, fonts, opacity, shape, cursor, the wallpaper): one choice themes
the compositor, the bar and the wallpaper together. The format is
[documented](../reference/look-format.md); the five shipped looks are
listed under [Theming](../scoot/theming.md). No step here needs Nix: a
plain binary install with a `looks/` directory lists, previews, applies
and authors looks with these commands alone.

```sh
scoot look list                       # the catalog, in search order
scoot look show moonrise              # tokens, wallpaper, fonts, contrast
scoot look apply moonrise             # render, validate, swap, reload
scoot look next                       # step through the catalog (wraps)
scoot look apply moonrise --for 20    # preview 20 seconds, then revert
scoot look current                    # the active look and shadowed keys
scoot look validate ./my-look         # check a directory without touching anything
scoot look build ./my-look --out /tmp/my-look   # render without applying
scoot look sync                       # follow the Nix declared default (below)
```

## Cycle and preview

`next` and `prev` step through the catalog in `list` order, wrapping at
either end. `random` picks a random look other than the active one, and
stays when the catalog holds one look. `Super+Shift+T` steps to the next
look; it does nothing while the session is locked. Each step applies
through the same transaction as `apply`, with the same reloads and
rollback. Two quick presses can land on the same look: each press picks
its target before the swap takes its lock.

`apply NAME --for SECONDS` previews a look and reverts after the delay.
The revert records the generation it reverts from: a newer `apply` before
the timer fires cancels the revert, so a preview never undoes a newer
choice. A preview over a pending preview chains the reverts: each timer
restores the choice it replaced, and an older timer stands down once a
newer choice lands. The timer is a detached child of the applying CLI, so
closing the terminal never strands it, and a compositor restart does not
drop it either (only the IPC `for_ms` timer is dropped by a restart). A
swap running when the delay ends waits the reverter out (bounded retries
with the guard re-checked), instead of dropping the revert. `scoot look
apply --keep` keeps the preview (and cancels the timer); `apply NAME
--keep` applies for good and cancels any pending preview.

## Pick and apply

`list` shows the catalog in search order: your own looks, installed
looks, home-manager looks, system looks, then the shipped ones. A name
that exists twice resolves to the earliest; `list` marks the later one
shadowed.

`apply NAME` renders the look, validates it, records the choice, repoints
the state link, reloads each running app's look layer in order (the
compositor first, then every bar of the session), and signals the
third-party daemons (mako, fcitx5, the OSD and night-light units).
Apps that are not running are skipped, never an error. With nothing
running, only the link moves, and the next session starts on the new look. A shipped look needs its
Nerd Fonts installed; without them `apply` refuses and changes nothing.

A swap reports three lists: what reloaded, what was skipped, and every
shadowed key — a key in your own files that overrides a look token. A
swap that changes nothing it was asked to still answers applied; check
`current` when the screen disagrees with the reply.

On refusal nothing half-applies. An invalid look refuses before anything
is touched. A refused reload rolls the swap back: the record and the link
go back to the previous look, the reloads that may hold the new layer run
again against the restored choice, and the reply names the refusal. That
is a positive receipt, a partial one (the compositor applied some fields
and refused others), and a sent-but-unanswered one (a timeout, a
connection closed after the write). A peer that refused outright — an old
compositor or bar included — never accepted the new layer, so it is never
re-run and nothing ever retries as a plain reload. If a rollback reload
also refuses, the reply
says `ROLLED BACK WITH ERRORS` with both reasons; each app is on its old
or its new look, never between — except a first-ever apply with no
previous look: there is nothing to reload back to, so whatever may hold
the new look still shows it while no look is recorded, and the
reply names exactly that.

Every command takes `--json` (the reply as JSON on stdout, notes on
stderr). Exit status: 0 applied, 1 the run failed, 2 the invocation was
wrong.

## What is live and what is next-launch

| Change | When it shows |
|---|---|
| Compositor ring, background, corners, cursor | Live, on apply |
| Bar colors, radius, opacity, fonts | Live, on apply |
| Wallpaper image or color | Live, on apply |
| Dark mode, GTK theme, icons, cursor, app font (`color-scheme`, `gtk-theme`, `icon-theme`, `cursor-theme`, `cursor-size`, `font-name`) | Live, on apply (see below) |
| GTK `gtk.css` colors | Next launch: GTK 3 text selection plus libadwaita accent and surfaces follow; plain GTK 4 keeps its stock accent; running apps keep their palette |
| Qt style variant and palette | Next launch of each app; under the Adwaita style QWidget apps keep Adwaita's fixed palette (see below) |
| Notification colors (mako) | Live, on apply (D-Bus reload; `makoctl` fallback) |
| OSD colors (wob) | Live, on apply (the unit restarts) |
| Input-method candidate theme (fcitx5) | Live, on apply (D-Bus reload) |
| Night-light temperature | Live, on apply (the unit restarts) |
| Launcher, picker and lock colors (`fuzzel.ini`, `swaylock`) | Next launch of each app |
| Running terminals | Keep their palette; new terminals use the look's |
| Cursor size in already-running apps | Next launch |

A swap never applies pending edits to your own files: the reloads are
scoped to the look layer, under a snapshot of your files from their last
successful load. Half-written files wait for your own `scoot msg reload`
(or `scootbar msg reload`).

## Third-party daemons

Each swap also renders the third-party daemon configs from the look's
tokens (`mako`, `wob.ini`, `fcitx5/theme.conf`, `nightlight` beside
`scoot.toml` in the rendered look) and signals each daemon. A daemon
that is not installed or not running is a note, never an error; a
private look state (headless or `--private-look`) signals none of them.

- mako reads the fragment through an `include=` of the link's file
  (ahead of your own keys, which win) and a D-Bus `Reload` restyles
  on-screen notifications without starting a stopped mako
  (`makoctl reload` only where no `busctl` exists). On Nix the generated config carries the
  include; without Nix add
  `include=~/.local/state/scoot/look/mako` yourself.
- wob reads a complete `wob.ini` from `-c` and the unit restarts (it
  starts in milliseconds and holds no state). Without Nix run
  `wob -c ~/.local/state/scoot/look/wob.ini`.
- fcitx5 reads the `scoot-look` classic UI theme and reloads it over
  D-Bus (`ReloadAddonConfig "classicui"`). Without Nix link
  `~/.local/share/fcitx5/themes/scoot-look/theme.conf` at the
  fragment and keep `Theme=scoot-look` in `classicui.conf`.
- The night-light unit reads the temperature from the fragment at
  start (only while the look's own warmth is in effect; an explicit
  temperature or a disabled target uses your value instead) and restarts
  on swap. Without Nix read
  `<state>/look/nightlight` (`temperature=` in Kelvin) in your own
  warmer's startup.

Your own settings win over the look, and a theme target turned off keeps the look away from that app entirely: mako and fcitx5 keys you set override the fragment by file order, an explicit night temperature beats the look's, and the OSD hide timeout and the day temperature stay yours. The first apply on a Nix machine
seeds the declared default look, so a fresh install's daemons never
start against a missing fragment.

## Files and flags

The choice is per-session state: `<state>/look.json` (the record) plus a
`look` link to one complete rendered look, under one lock. Renders live
in the cache (`$XDG_CACHE_HOME/scoot/rendered/<name>-<hash>/`), pruned to
the active, the previous and the four most recent. An unwritable state dir
refuses loudly. The rendered look holds `fuzzel.ini` and `swaylock`
beside the first-party fragments: layer them over your own configs and
the next launch picks the look up, no Nix involved.

Your own configs stay the base. Neither file is ever passed to its app
as the whole config: fuzzel's `--config` and swaylock's `-C` each read
that file instead of your own, so your font, terminal, lock image and
other settings would be lost while the look is active. The look's
colors layer over your files instead. Fuzzel takes them as CLI flags
(flags win over the file, so only the seven colors change). Swaylock
reads the look's lines first and yours after in one merged file (later
lines win, so your keys win per key and the look fills the colors).
A Nix install wires this for the launcher, the pickers, the chooser and
the lock screen, falling back to the eval-time theme while no look is
applied. Without Nix, wire the same two mergers yourself:

```sh
# Fuzzel: your config stays; the look's seven colors arrive as flags.
flags=$(awk '/^\[colors\]/{s=1;next} /^\[/{s=0} s&&/=/ {k=$0;sub(/=.*/,"",k);v=$0;sub(/[^=]*=/,"",v); if((k=="background"||k=="text"||k=="prompt"||k=="match"||k=="selection"||k=="selection-text"||k=="border") && v~/^[0-9a-fA-F][0-9a-fA-F][0-9a-fA-F][0-9a-fA-F][0-9a-fA-F][0-9a-fA-F][0-9a-fA-F][0-9a-fA-F]$/) printf " --"k"-color="v}' ~/.local/state/scoot/look/fuzzel.ini)
fuzzel $flags

# Swaylock: the look's lines first, your config after, as one file.
cat ~/.local/state/scoot/look/swaylock ~/.config/swaylock/config > "$XDG_RUNTIME_DIR/scoot-swaylock-config"
swaylock -C "$XDG_RUNTIME_DIR/scoot-swaylock-config"
```

(`~/.local/state/scoot` is the default state dir; `--state` and
`$SCOOT_LOOK_STATE` move it, as above.)

- `--state DIR` takes the state (default: `$SCOOT_LOOK_STATE`, then
  `$XDG_STATE_HOME/scoot`, then `~/.local/state/scoot`).
- `--socket PATH` takes the compositor socket (default: `$SCOOT_SOCKET`,
  then `$XDG_RUNTIME_DIR/scoot.sock`).
- A headless session keeps a private copy of the choice: swaps inside it
  never touch the desktop. `--tty` and `--nested` sessions use the shared
  choice, so their swaps survive a restart.

## Dark mode and the toolkit step

`apply` renders the toolkit files once and writes the dconf keys, so a
swap reaches GTK and libadwaita at once with no resident daemon. GTK 4
and libadwaita follow through the portal; GTK 3 follows the theme,
icons, cursor and font through GSettings directly (`color-scheme` means
nothing to GTK 3).

The dconf keys, all under `org.gnome.desktop.interface`: `color-scheme`
(`prefer-dark`/`prefer-light` from the polarity), `gtk-theme`
(`Adwaita-dark`/`Adwaita`), `icon-theme`, `cursor-theme`, `cursor-size`
and `font-name`. The owned files, under `~/.config`: the two `gtk.css`
shims (one `@import` of the state link's per-version `gtk/gtk-3.0.css`
and `gtk/gtk-4.0.css`, so new GTK apps take the new palette with no
rewrite per swap), the two `settings.ini`
files, `qt6ct/qt6ct.conf` (through the link's scheme), and the
fontconfig default. The tool owns a key or file only while it holds the
record: it writes when the key is unset or holds what the tool last
wrote, adopts it on the first apply whatever it holds, and otherwise
leaves it alone and names it. A file the tool did not write (hand-made,
or linked into the Nix store by home-manager) is left alone and named.
A box without `dconf` gets a note, not a refusal.

Qt limits, both measured: under the Adwaita style the look's palette
never reaches QWidget apps (adwaita-qt replaces it with its fixed one —
a style honoring `custom_palette` paints the scheme), and an in-place
scheme change fires no watcher in a running app (qt6ct watches its own
directory for added and removed files only). New Qt processes read the
new palette; running ones keep theirs.

- `scoot look apply --no-toolkit` skips the step for one apply.
- `scoot look toolkit off` persists the skip in `look.json`. When the tool
  wrote what is still live, `off` restores each key from before the
  tool's first write and stops owning it; a key the tool never wrote,
  one someone else changed since, one home-manager owns (see below),
  or one that cannot be read is left alone. Either way the record
  clears, so turning back `on` adopts fresh on the next apply.
  `off` leaves the written files in place; only the keys are restored.
  `scoot look toolkit` shows the state.
- A private look state (a headless or `--private-look` session) never
  runs the step: its swaps must not move the real desktop.

## Terminal (foot)

`apply` renders the look's `[terminal]` hues into the rendered look's
`foot.ini`: `[main]` selects the look's polarity as foot's initial
theme, and both `[colors-dark]` and `[colors-light]` carry the same
hues, with `alpha` from `opacity.terminal`. A look without
`[terminal]` renders no fragment. New terminals follow the look once
your `foot.ini` includes the fragment (one-time setup):

```ini title="foot.ini"
include=~/.local/state/scoot/look/foot.ini
```

Both palettes hold the look's hues, so new terminals match whatever
theme is in effect, even with a user's own opposite
`initial-color-theme`. Running terminals keep their palette until
they restart: foot never re-reads its config, and its
`SIGUSR1`/`SIGUSR2` switch only toggles the two palettes loaded at
foot start, so a swap sends no signal. `validate` and `build` check
the fragment against foot's own parser; `build --out` writes
`foot.ini` beside the other fragments.

## The Nix catalog

`programs.scoot.looks` lists the catalog (default: every shipped
look, plus paths to your own source look directories); each look
renders at build time and installs as `~/.local/share/scoot/looks/<name>`
(sources) beside `~/.local/share/scoot/rendered/<name>` (renders).
`programs.scoot.look` is the declared default (`ginger-night` with the
profile on, `null` for no layer), written to
`~/.config/scoot/look-default`. Each activation runs `scoot look sync`
after the links and dconf land: with no state it starts on the
declared default; when the default changed the declaration wins; when
the live choice left the catalog it falls back and says so; otherwise
it keeps the live choice and re-resolves it, reloading only when the
render changed. A null default removes the layer, rolling back like
any other swap. With no session running, only the link moves.

The toolkit step above runs on `sync` too, from the active
look. Keys home-manager owns stay home-manager's: the catalog records
the `org.gnome.desktop.interface` names the merged `dconf.settings`
still carry in `look-nix-claims` beside the declared default, and
`sync` (and `apply`) leave those alone however the live value reads.
On a NixOS-module-only machine the catalog installs system-wide, the
default lands in `/etc/scoot/look-default`, and the session launcher
runs `scoot look sync` at login.

## Authoring

Write a directory with a `look.toml` (see the
[format](../reference/look-format.md)), then `scoot look validate DIR`
and `scoot look build DIR --out OUT`. `validate` resolves, contrast-checks
and runs the effectful checks: fonts through fontconfig (the family must
be installed; a `[fonts.files]` path must exist), the wallpaper
through `scootbg check-image`, the bar fragment through the installed
bar's own parser. Your own looks live in
`~/.local/share/scoot/user-looks/<name>/`.

> **Symptom:** `apply` answers applied and nothing changes.
> A key in your own files shadows the look. `scoot look current` names
> every shadowing key; remove or change yours to let the look through.

> **Symptom:** `apply` refuses with `the compositor predates scoot look:
> update scoot and restart the session`.
> The compositor is older than the look-scoped reload. Update scoot and
> restart the session; the swap rolled back and nothing retried as a plain
> reload.

> **Symptom:** `apply` refuses with `the running bar predates the
> look-scoped reload: restart it`.
> The bar is older than the look-scoped reload: it answered the
> `reload-look` request with an ordinary unknown-request error, applying
> nothing. Restart the bar and re-apply; the swap never retries as a plain
> reload.
