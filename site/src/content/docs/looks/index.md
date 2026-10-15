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
scoot look current                    # the active look and shadowed keys
scoot look validate ./my-look         # check a directory without touching anything
scoot look build ./my-look --out /tmp/my-look   # render without applying
```

## Pick and apply

`list` shows the catalog in search order: your own looks, installed
looks, home-manager looks, system looks, then the shipped ones. A name
that exists twice resolves to the earliest; `list` marks the later one
shadowed.

`apply NAME` renders the look, validates it, records the choice, repoints
the state link, and reloads each running app's look layer in order: the
compositor first, then every bar of the session. Apps that are not running
are skipped, never an error. With nothing running, only the link moves,
and the next session starts on the new look. A shipped look needs its
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
| GTK `gtk.css` colors | Next launch of each app; running apps keep their palette |
| Qt style variant and palette | Next launch of each app; under the Adwaita style QWidget apps keep Adwaita's fixed palette (see below) |
| Launcher and notification configs | Next launch of each app |
| Running terminals | Keep their palette; new terminals use the look's |
| Cursor size in already-running apps | Next launch |

A swap never applies pending edits to your own files: the reloads are
scoped to the look layer, under a snapshot of your files from their last
successful load. Half-written files wait for your own `scoot msg reload`
(or `scootbar msg reload`).

## Files and flags

The choice is per-session state: `<state>/look.json` (the record) plus a
`look` link to one complete rendered look, under one lock. Renders live
in the cache (`$XDG_CACHE_HOME/scoot/rendered/<name>-<hash>/`), pruned to
the active, the previous and the four most recent. An unwritable state dir
refuses loudly.

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
shims (one `@import` of the state link's `gtk/gtk.css`, so new GTK apps
take the new palette with no rewrite per swap), the two `settings.ini`
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
  one someone else changed since, or one that cannot be read is
  left alone. Either way the record clears, so turning back `on` adopts
  fresh on the next apply. `scoot look toolkit` shows the state.
- A private look state (a headless or `--private-look` session) never
  runs the step: its swaps must not move the real desktop.

## Dogfooding on a home-manager machine

Until the Nix catalog lands, the eval-time merges write look and Stylix
values into your own `config.toml` and `bar.toml`, and those shadow every
look. For the duration, set:

```nix title="home.nix"
programs.scoot.desktop.look = null;
programs.scoot.stylix.enable = false;
programs.scootbar.stylix.enable = false;
```

`scoot look current` lists anything still shadowing. Cost: with
`desktop.look = null` the profile's theme slot is inert too, so GTK, Qt,
fontconfig and foot lose their theming while dogfooding; it returns with
the Nix catalog.

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
