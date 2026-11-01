---
title: Look format reference
description: "The look.toml schema v1: tokens, defaults, validation and rendered fragments."
---

A look is a directory of plain files. `look.toml` holds the tokens.
`validate` and `build` read that directory with the `scoot-look`
binary. No step needs Nix: a plain binary install renders the same
sources with the same binary. A Nix install renders them at build
time with the same binary.

## A look is a directory

A source look is what an author writes:

```sh
ls ~/.local/share/scoot/user-looks/moonrise/
# look.toml  wallpaper.png  fonts/  preview.png  apps/
```

A rendered look is the output of a build: `scoot.toml`, `bar.toml`,
`launch.toml`, `notify.toml`, `greet.toml`,
`gtk/gtk-3.0.css`, `gtk/gtk-4.0.css`, `qt/scoot-look.conf`,
`foot.ini` (only when the look has `[terminal]`), `fuzzel.ini`,
`swaylock`, `render.json`, the
daemon leaves (`mako`, `wob.ini`, `fcitx5/theme.conf`, `nightlight` when
the look names a temperature) and the terminal static leaves
(`apps/helix.toml`, `apps/nvim.lua`, `apps/btop.theme`,
`apps/starship.toml`, each only when the look carries it). It is never edited and never a source for
anything. The two never share a path.

The catalog is the union of the source directories, in fixed order:
user looks, installed looks, home-manager, system, shipped. Origin
is by directory. Installed looks (someone else's) apply but never
serve as an `extends` base. A name that exists twice resolves to the
earliest in the order, and `list` marks the shadowed one.

## look.toml

`schema` is `1`. `name` matches the directory name. Every other
token is optional in one file and required after `extends`
resolves: a child with `extends = "base"` writes only its
overrides.

```toml
schema = 1
name = "moonrise"
title = "Moonrise"
description = "Slate navy."
polarity = "dark"
license = "MIT"

[palette]
background = "#2B3648"
foreground = "#F6EEDC"
dim = "#9C8B95"
accent = "#FFA45C"
urgent = "#E87F6A"

[fonts]
ui = "DroidSansM Nerd Font Propo"
sans = "DejaVu Sans"
mono = "FiraCode Nerd Font"
size = 10

[shape]
radius = 14
ring-width = 2

[cursor]
theme = "Mocu"
size = 24
accent = "#FF9A49"

[wallpaper]
color = "#2B3648"
```

| Section | Keys (type, default) |
|---|---|
| `palette` | Thirteen `#rrggbb` roles. Required: `background`, `foreground`, `dim`, `accent`, `urgent`. Defaults: `surface` = `background`; `hover` = `accent`; `focus` = `accent`; `border` = `dim`; `accent-fg` = `background`; `selection` = `accent`; `selection-text` = `accent-fg`; `match` = `hover`. |
| `terminal` | `colors`: exactly 16 `#rrggbb` hues, hand-placed. No table means no foot fragment, never a synthesized palette. Renders the themed fragment (`[main]` `initial-color-theme` plus `[colors-dark]`/`[colors-light]`, needs foot 1.26+); against an older foot the apply renders the legacy `[colors]` fragment instead, with a note. |
| `fonts` | `ui`, `sans`, `mono`: family names. `size`: points, desktop text. `popup-size`: default = `size`. `app-size`: default = `12`. `terminal-size`: default = `size`. Sizes convert to pixels as `clamp(1, 256, (pt * 4 + 1) / 3)`. A named family that is not installed falls back with a note (the look's other families first, then fontconfig's generic answer); only a box with no usable font file at all refuses. |
| `fonts.files` | Bundled files in `fonts/`: `ui`, `sans` or `mono` to a `.ttf` or `.otf` path. License required. A bundled face wins over the fallback chain for its key. |
| `fonts.nix` | Nixpkgs attribute paths: `ui`, `sans` or `mono` to one to eight `Name.segment` paths. Resolved only with `lib.attrByPath` against `pkgs`. Nix-only; a non-Nix box resolves families with `fc-match`. |
| `opacity` | `bar`, `popups`, `terminal`: 0.0 to 1.0, each default 1.0. |
| `shape` | `radius`: 0 to 64, default 0. `ring-width`: 0 to 64, required. `ring-inactive-width`: default = `ring-width`. No `gap` in v1: gap is layout, not presentation. |
| `effects` | Depth and motion tokens, all optional and unset by default. `shadow`: `none`, `soft`, `deep` or `glow`. `fill`: `solid` or `gradient`. `motion`: `none`, `ease-out` or `spring`. `motion-duration-ms`: 0 to 1000. `motion-bounce`: 0.0 to 1.0. `scootnotify` and `scootlaunch` read them; a look that sets none renders exactly the fragments it did before. See [notification depth](../scootnotify/configure.md#depth) and [motion](../scootnotify/configure.md#motion), and the [launcher's](../scootlaunch/panel.md#depth-and-motion). To set them for every look without editing one, use the `[effects]`, `[card]` and `[title]` tables of `notify.toml` and `launch.toml` ([notifier](../scootnotify/configure.md#effects-over-any-look), [launcher](../scootlaunch/panel.md#effects-over-any-look)). |
| `cursor` | `theme`: name, required. `size`: 4 to 256, default 24. `accent`: `#rrggbb` or `"none"`, default = `focus`. `body`: optional `#rrggbb`; a look leaves it alone unless set. |
| `wallpaper` | One unit, never merged per key: `image` or `color`, never both. `image` is a path inside the look dir, or `https://` plus a required 64-hex `sha256`. `mode`: `fill`, `fit`, `stretch`, `center` or `tile` (image only; a color wallpaper carries no `mode` or `fill`). `fill`: default = `background`. `transition`: `none`, `fade`, `wipe` or `grow` (default `none`; the five shipped looks set `fade`, which animates at scootbg's defaults: 500 ms, `ease-out`). |
| `icons` | `theme`: name. Default `Adwaita`. |
| `night` | `temperature`: 1000 to 10000. |
| `apps.<app>` | Per-app overrides. Each key is the app's own dotted config key, quoted. Only allowlisted keys parse, and each key's value is checked against its kind (see below). Anything else refuses by name. `name` must match the look directory name. |

```toml
[apps.scootbar]
"workspaces.inactive-color" = "#B595AD"
```

### Per-app override keys and values

`scoot` takes its appearance, cursor and wallpaper keys, plus
per-output and per-workspace wallpaper overrides of the form
`wallpaper.output.<name>.<leaf>`,
`wallpaper.workspace.<name>.<leaf>` and
`wallpaper.output.<name>.workspace.<name>.<leaf>`, where `<leaf>`
is one of `color`, `mode`, `fill`, `transition`.
Only one `.workspace.` layer parses: a second one under a
`wallpaper.workspace.` prefix refuses.
Output and workspace names hold 1 to 64 ASCII letters, digits,
spaces and `_.+-`. Anything else in the suffix refuses, as does any
key with a newline, `=`, `[` or quote. The renderer quotes every
key on top of that. No override names an `image`: only the
top-level `wallpaper.image`, resolved to an absolute path at build
time, does. Per-output and per-workspace images belong in the
user's own file.

`scootbar` takes its six colors, bar style and per-module visual
keys. The bar's `font` and `fallback-fonts` are not overrides: scootbar
reads file paths there, so only the tool's resolved `--ui-font-file`
supplies them, and a fragment without one carries neither key.
`scootnotify`, `scootlaunch` and `scootgreet` take the thirteen
color roles plus face, size, opacity and radius. `scootnotify` and
`scootlaunch` also take `shadow`, `fill`, `motion`, `motion-duration-ms`
and `motion-bounce` (the `[effects]` values, per app). `scootnotify` alone
takes `card.enter`, `card.exit`, `card.mark`, `card.shape`,
`card.app-accent`, `title.weight` and `title.spacing`; the others refuse
what they do not read, since they would report an unknown key on every
load.

Every allowlisted key has a value kind, checked at parse. There is
no key whose value gets only a length check.

| Key | Value |
|---|---|
| color keys (`appearance.*_color`, `colors.*`, `wallpaper.color`, `wallpaper.fill`, `workspaces.*-color`) | exactly `#rrggbb` |
| `appearance.focus_ring_width`, `appearance.focus_ring_inactive_width`, `appearance.corner_radius` | whole number, 0 to 64 |
| `appearance.cursor_size` | whole number, 4 to 256 |
| `appearance.cursor_theme`, shell `font` | identifier (see Values) |
| `wallpaper.image` | relative image path (see below); never a URL; never an override leaf |
| `wallpaper.mode` and its leaves | `fill`, `fit`, `stretch`, `center` or `tile` |
| `wallpaper.transition` and its leaves | `none`, `fade`, `wipe` or `grow` |
| `bar.radius`, `bar.popup-radius` | whole number, 0 to 512 at parse; `bar.radius` renders clamped to half the default bar height (14) |
| `bar.opacity`, shell `opacity` | number, 0 to 1 |
| `bar.font-size`, shell `font-size` | whole number, 1 to 256 |
| `bar.padding`, `bar.spacing`, `bar.separator` | whole number, 0 to 1024 |
| `workspaces.item-gap` | whole number, 0 to 1024 |
| `workspaces.display` | `numbers` or `dots` |
| `window-title.icon` | glyph string (see Values) |
| shell `radius` | whole number, 0 to 64 |
| `scootnotify`, `scootlaunch` `shadow` | `none`, `soft`, `deep` or `glow` |
| `scootnotify`, `scootlaunch` `fill` | `solid` or `gradient` |
| `scootnotify`, `scootlaunch` `motion` | `none`, `ease-out` or `spring` |
| `scootnotify`, `scootlaunch` `motion-duration-ms` | whole number, 0 to 1000 |
| `scootnotify`, `scootlaunch` `motion-bounce` | number, 0 to 1 |
| `scootnotify` `card.enter`, `card.exit` | `slide`, `fade`, `scale` or `drop` |
| `scootnotify` `card.mark` | `none`, `stripe`, `dot` or `badge` |
| `scootnotify` `card.shape` | `rounded` or `pill` |
| `scootnotify` `card.app-accent` | `true` or `false` |
| `scootnotify` `title.weight` | whole number, 100 to 900 |
| `scootnotify` `title.spacing` | whole number, -2 to 8 |

A URL wallpaper cannot be overridden per app: its `sha256` pin has
no place in an override, so per-app `image` values refuse URLs.

## extends

`extends` names a source look. Every token inherits; `[apps.*]`
and the font carriers merge per key; `[wallpaper]` swaps as one
unit. Resolution refuses unknown names, cycles, chains deeper than
8, and an installed base. A rendered look has no `extends`.

## Values

Every string is bounded, because look strings land in line-based
app configs where some keys run code. Names (`name`, `extends`):
`[a-z0-9-]{1,32}`, and `name` must match the look directory name.
Identifiers (font families, theme names):
ASCII letters, digits, space and `-_.+`, 1 to 64 bytes, starting
with a letter or digit, so never `.` or `..`. ASCII-only in v1:
a look that needs a non-Latin family name carries the file in
`fonts.files`. Display text (`title` to 64 bytes, `description`,
`credits`, `license` to 200): no control characters and no `=`,
`[`, `]`, `"` or `\`. Glyph strings (such as
`window-title.icon`): 1 to 8 characters, no
control, format or ASCII punctuation characters. Paths: relative,
normalized (no `.`, `..`, `//` or trailing `/`), no leading `/`;
the file must sit inside the look dir. `preview`, when set, is
exactly `preview.png`. A look bundling files in `fonts/` names
their `license`, whatever its origin.
NUL refuses everywhere. The whole file caps at 64 KiB.

Wallpaper URLs take `https://` only, to 2048 bytes: printable
ASCII with no spaces or control characters, and never `?`, `#`
or `%`. The host is a DNS name with a dot: no userinfo, IP
literal (dotted, decimal, octal, hex or IPv6), port,
`localhost` or local name. The host lowercases and strips one
trailing dot before those checks. A public name can still resolve
to a private address, so someone else's look fetches at install
time, never at swap time. A URL wallpaper requires a `sha256`
pin of exactly 64 hex digits.

## Contrast

Four pairs refuse below their bar: foreground on background at
4.5, accent fill text on accent at 4.5, accent on background at
3.0, focus on background at 3.0. Four more pairs report only: `dim`
on background at 3.0, `urgent` on background at 3.0, `dim` on
`surface` at 4.5, and `border` on background at 3.0. A warning
names the pair and the ratio to two decimals; bars compare on the
unrounded value, so a warning always means below the bar (a real 4.46
prints as 4.46, never as a passing-looking 4.5).

`dim` on `surface` is the text bar for the notification card's
app-name line and the launcher's secondary text, both of which
draw `dim` over `surface` (which defaults to `background`). With
`effects.fill = "gradient"` the card's fill shifts across its
height, so the check measures `dim` against the worse of the
gradient's two stops. A look-level `effects.fill` is what the
check reads; a per-app `fill` override is not checked.

`border` on background is the inactive focus ring (the compositor's
`focus_ring_inactive_color`), an outline that should read at
3.0 so an unfocused window has a visible edge.

Who a failure of a refusing pair stops depends on who chose the
look: shipped and installed looks refuse, with the pair and the
ratio named. A look the user's own configuration produced warns
and never refuses. The report-only pairs warn for everyone, so
existing looks keep installing and applying; they only gain
warnings.

## Rendered fragments

`scoot.toml` carries the compositor colors and widths, cursor,
corner radius and the wallpaper unit. `bar.toml` carries the six
bar colors, radius (clamped to half the default bar height, 14),
`popup-radius` (only when the target binary
has the `popup` feature), opacity and font size. `font` appears
only with `--ui-font-file`: scootbar reads a file path there, and
without one the bar falls back to its well-known font paths.
Per-app overrides merge into those tables: each section renders
once, with the override winning, so every fragment reparses.
`gtk/gtk-3.0.css` carries the look's selection as direct rules
(proven amber live; a `:root` block breaks its parser and
`@define-color` has no effect on it). `gtk/gtk-4.0.css` carries the look's accent and surfaces as the named
colors libadwaita paints with, plus the matching `:root` vars (plain
GTK 4 widgets keep the stock accent). `qt/scoot-look.conf` carries the
Qt color scheme.
`foot.ini` carries `[main]` with `initial-color-theme` from the look's
polarity, and the 16 `[terminal]` hues as `regular0-7` then
`bright0-7` in both `[colors-dark]` and `[colors-light]` (foot 1.28
has no `[colors]`), each with `alpha` from `opacity.terminal`. Both
themes hold the same hues, so new terminals match whatever theme is
in effect.
`fuzzel.ini` carries the seven launcher colors as a `[colors]`
table in fuzzel's `rrggbbaa` spelling; it sets no font, so the launcher
keeps the user's own. `swaylock` carries the fourteen locker colors as
`key=value` lines in swaylock's `rrggbb` spelling. Both are
next-launch: fuzzel reads its config on every start, swaylock on
every lock. Neither file is a whole config: each app replaces the
user's file when pointed at another (`--config`, `-C`), so the apps
layer these leaves over the user's own instead -- fuzzel as CLI flags
read out of the file at launch, swaylock as the file's lines first
with the user's lines after (later lines win). The user's own keys
always survive a swap; the look only fills the colors.

`launch.toml` carries the launcher panel's look: the thirteen shell-app
color roles as a `[colors]` table, the look's UI family as `font` and,
when the tool resolved one, the file it resolved to as `font-file` (see
below), its popup size as `font-size`, the popups opacity as `opacity`, the shape
radius as `radius`, and the icon theme as `[icons] theme`. `notify.toml`
carries the same thirteen roles and dotless keys for the notification
cards. An `[apps.scootlaunch]` (or `[apps.scootnotify]`) `colors.*` key
replaces the derived role, and a dotless key (`font`, `font-size`,
`opacity`, `radius`) replaces the top-level value, so each is emitted
once. `font-file` is the tool's own key, like the bar's `font`: the
file `scoot look apply` resolved `fonts.ui` to (`fc-match`'s answer, or the
look's `[fonts.files]` entry; the fallback file its note names when the
family is not installed, as for the bar's `font`), an absolute path the apps read without
walking any font directory. It is not an override (`[apps.*]` refuses it)
and it is left out when the app overrides `font`, or when no file was
resolved (no `fc-match`); the apps then look the family up by file name.
`scoot-look build` writes it from `--ui-font-file`, as for the bar.
A look that sets `[effects]` adds top-level `shadow`, `fill`,
`motion`, `motion-duration-ms` and `motion-bounce` keys to `notify.toml`
and `launch.toml`, and a `[card]` table for `card.enter` and `card.exit`
overrides to `notify.toml` only; `greet.toml` never carries them. The
notifier's other card keys (`card.mark`, `card.shape`, `card.app-accent`) join that `[card]`
table and `title.weight` and `title.spacing` a `[title]` table, each only
when the look sets them. scootlaunch reads its fragment at every open; scootnotify reads
its own at start and on `msg reload --look` (which `scoot look apply`
sends live). `greet.toml` carries the
login screen's look, read at system build time only. `scoot look
validate` and `scoot look build` (and the underlying `scoot-look
validate` / `scoot-look build`) render and check all three beside the
first-party fragments: a refused `[apps.scootlaunch]` key fails the
build the same way a bad bar key does.

`render.json` names the source, its hash and the resolved inputs
(font files, wallpaper path). The inputs come from the
environment: the tool resolves font files and the wallpaper's
absolute path, and the Nix build passes the font packages. The
fragments must pass the apps' own strict parsers; that check
lives in the caller. The daemon leaves need only the look's tokens:
`mako` the notification colors, `wob.ini` a complete OSD config
without a `timeout` line (the hide timeout is desktop configuration,
not a look token), `fcitx5/theme.conf` the classic UI theme,
`nightlight` one `temperature=` line. Every interpolated value is a
`#rrggbb` token the renderer re-checks, so no format ever receives a
raw newline, `=` or section header from a look.
The terminal static leaves are never rendered from tokens: each is
the author's `apps/` file copied verbatim (shape-checked: size,
text, well-formed TOML, btop theme lines, or a `colors_name`-setting
Lua colorscheme), inherited through
`extends` unless the child names its own. Installed looks (someone
else's) drop them without `--trust`, which prints each kept leaf
first; every other origin keeps them.

## Validate and build

```sh
scoot-look validate ./moonrise --origin user
scoot-look build ./moonrise --out /tmp/moonrise --wallpaper-abs /tmp/moonrise/wallpaper.png --ui-font-file /usr/share/fonts/TTF/DejaVuSans.ttf
```

`validate` refuses malformed files, unknown keys, values outside
the grammar, non-allowlisted `[apps.*]` keys, per-key value
mismatches, a `name` that does not match the directory, and
contrast shortfalls per origin. The `name` check is CLI-only: the
library exposes `check_name_matches_dir`, and the tool resolves
the directory through `canonicalize` first, so `validate .`
checks the real directory name. A Nix store path
(`<hash>-<name>`) strips its hash prefix; `--name NAME` checks
against that instead of the directory for callers that lay the
source down under another name. `--trust` keeps an installed
look's `apps/` leaves (like `scoot look validate --trust`).
`validate` skips contrast for a
look with `extends`: without the catalog only single-file looks
resolve here. `build` takes a single source without `extends`
and refuses one with it; resolve the catalog first. `--no-popup`
renders the bar fragment without the `popup` feature's keys (for
minimal bar builds); `--catalog DIR` resolves font files and the
wallpaper against that tree instead of the live directories. `build`
writes `scoot.toml`, `bar.toml`, `gtk/gtk-3.0.css`, `gtk/gtk-4.0.css`,
`qt/scoot-look.conf`, `foot.ini` (only when the look has
`[terminal]`), `fuzzel.ini`, `swaylock`, `launch.toml`, `notify.toml`,
`greet.toml`, `render.json`, the daemon leaves and the
terminal static leaves into the output dir
(never through a symlink, and never beside files it did not write). Both
read `look.toml` only through a regular file opened without blocking:
symlinks, directories, devices, FIFOs and sockets refuse. The static
leaves read the same way (an installed look's drop first, unless
`--trust`). On Nix the same binary
runs inside the render derivation with the look's font packages
as build inputs. Exit nonzero names the field and the reason.

## Import from base16

```sh
scoot-look import-base16 scheme.yaml --out ~/.local/share/scoot/user-looks/solarized-dark
scoot-look import-base16 scheme.yaml --extends ginger-night --out ~/.local/share/scoot/user-looks/solarized-green
scoot-look import-base16 scheme.yaml --name my-look --out /tmp/my-look
```

One mapping turns any base16 scheme (or the base24 superset) into a
source look. It is what Stylix gives a Nix user, for a plain install:
no Nix involved, plain files in and out. It reads base16 YAML and the
JSON form the Nix side serializes, with or without a leading `#`, any
case, `palette:`-nested or flat. A leading UTF-8 BOM strips first.
The input caps at 64 KiB. The YAML subset refuses tabs for indent,
lists, flow collections, aliases/anchors, block scalars and tags with
a named error.

| Token | Slot |
|---|---|
| `background`, `surface` | `base00` |
| `foreground` | `base05` |
| `dim`, `border` | `base03` |
| `accent`, `hover`, `focus` | `base0D` |
| `urgent` | `base08` |
| `accent-fg` | `base00` |
| `selection`, `selection-text` | `base02`, `base05` |
| `match` | `base0A` |
| `[terminal]` | ANSI 0-15 as `base00, base08, base0B, base0A, base0D, base0E, base0C, base05, base03, base08, base0B, base0A, base0D, base0E, base0C, base07` (base16's terminal order: foot `regular0-7` then `bright0-7`), only without `--extends` |

This matches the Nix modules for the slots they use today. Companion
data applies when present: `variant` (or `polarity`) sets dark or
light, else `base00` against `base05` decides. `fonts` maps `sansSerif`
to `sans` and `ui`, `monospace` to `mono`. `sizes` maps `desktop`,
`popups`, `applications` and `terminal` to the four point sizes.
`opacity` maps `popups` and `terminal`; the bar's stays unset. `cursor`
maps `name` and `size`. Without `--extends` the output is complete
(title, polarity, fonts, ring width, Mocu cursor, flat background
wallpaper, terminal hues). With it the output is a child: palette plus
companion fonts, sizes, opacity and cursor when given, inheriting
terminal, shape and wallpaper from the base. `--extends` accepts any
`[a-z0-9-]{1,32}` name and is never checked against a catalog here.
Generated text (`title`, `description`, `license`, `credits`) sanitizes
to the grammar and never refuses. The output always
re-parses; a contrast shortfall warns (to stderr) and never refuses,
since the look is the user's own choice, except a child skips the
contrast check here (the base is unknown without the catalog).
A missing or malformed slot refuses naming
it. `--name` wins over the scheme's slug; `--origin` defaults to
`user`. Without `--out` the `look.toml` goes to stdout (warnings still
go to stderr, so stdout parses as TOML). With `--out DIR` the directory
must match the look name or the command errors.

> **Symptom:** `build` says an image wallpaper needs its resolved absolute path.
>
> Pass `--wallpaper-abs` with the image's absolute path. A color
> wallpaper needs no flag.
