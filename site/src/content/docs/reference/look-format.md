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

A rendered look is the output of a build: `scoot.toml`, `bar.toml`
and `render.json`. It is never edited and never a source for
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
| `terminal` | `colors`: exactly 16 `#rrggbb` hues, hand-placed. No table means no foot fragment, never a synthesized palette. |
| `fonts` | `ui`, `sans`, `mono`: family names. `size`: points, desktop text. `popup-size`: default = `size`. `app-size`: default = `12`. `terminal-size`: default = `size`. Sizes convert to pixels as `clamp(1, 256, (pt * 4 + 1) / 3)`. |
| `fonts.files` | Bundled files in `fonts/`: `ui`, `sans` or `mono` to a `.ttf` or `.otf` path. License required. |
| `fonts.nix` | Nixpkgs attribute paths: `ui`, `sans` or `mono` to one to eight `Name.segment` paths. Resolved only with `lib.attrByPath` against `pkgs`. Nix-only; a non-Nix box resolves families with `fc-match`. |
| `opacity` | `bar`, `popups`, `terminal`: 0.0 to 1.0, each default 1.0. |
| `shape` | `radius`: 0 to 64, default 0. `ring-width`: 0 to 64, required. `ring-inactive-width`: default = `ring-width`. No `gap` in v1: gap is layout, not presentation. |
| `cursor` | `theme`: name, required. `size`: 4 to 256, default 24. `accent`: `#rrggbb` or `"none"`, default = `focus`. `body`: optional `#rrggbb`; a look leaves it alone unless set. |
| `wallpaper` | One unit, never merged per key: `image` or `color`, never both. `image` is a path inside the look dir, or `https://` plus a required 64-hex `sha256`. `mode`: `fill`, `fit`, `stretch`, `center` or `tile`. `fill`: default = `background`. `transition`: `none`, `fade`, `wipe` or `grow`. |
| `icons` | `theme`: name. Default `Adwaita`. |
| `night` | `temperature`: 1000 to 10000. |
| `apps.<app>` | Per-app overrides. Each key is the app's own dotted config key, quoted. Only allowlisted keys parse: `scoot` takes its appearance, cursor and wallpaper keys; `scootbar` takes its six colors, bar style and per-module visual keys; `scootnotify`, `scootlaunch` and `scootgreet` take the thirteen color roles plus face, size, opacity and radius. Anything else refuses by name. |

```toml
[apps.scootbar]
"workspaces.inactive-color" = "#B595AD"
```

## extends

`extends` names a source look. Every token inherits; `[apps.*]`
and the font carriers merge per key; `[wallpaper]` swaps as one
unit. Resolution refuses unknown names, cycles, chains deeper than
8, and an installed base. A rendered look has no `extends`.

## Values

Every string is bounded, because look strings land in line-based
app configs where some keys run code. Names (`name`, `extends`):
`[a-z0-9-]{1,32}`. Identifiers (font families, theme names):
ASCII letters, digits, space and `-_.+`, 1 to 64 bytes, starting
with a letter or digit, so never `.` or `..`. ASCII-only in v1:
a look that needs a non-Latin family name carries the file in
`fonts.files`. Display text (`title` to 64 bytes, `description`,
`credits`, `license` to 200): no control characters and no `=`,
`[`, `]`, `"` or `\`. Glyph strings: 1 to 8 characters, no
control, format or ASCII punctuation characters. Paths: relative,
no `..`, no leading `/`; the file must sit inside the look dir.
NUL refuses everywhere. The whole file caps at 64 KiB.

Wallpaper URLs take `https://` only, to 2048 bytes, with a host
that is a DNS name with a dot: no userinfo, IP literal, port,
`localhost` or local name. The host lowercases and strips one
trailing dot before those checks. A public name can still resolve
to a private address, so someone else's look fetches at install
time, never at swap time.

## Contrast

Four pairs refuse below their bar: foreground on background at
4.5, accent fill text on accent at 4.5, accent on background at
3.0, focus on background at 3.0. `dim` and `urgent` report only.
Who a failure stops depends on who chose the look: shipped and
installed looks refuse, with the pair and the ratio named. A
look the user's own configuration produced warns and never
refuses.

## Rendered fragments

`scoot.toml` carries the compositor colors and widths, cursor,
corner radius and the wallpaper unit. `bar.toml` carries the six
bar colors, radius, `popup-radius` (only when the target binary
has the `popup` feature), opacity, font and font size.
`render.json` names the source, its hash and the resolved inputs
(font files, wallpaper path). The inputs come from the
environment: the tool resolves font files and the wallpaper's
absolute path, and the Nix build passes the font packages. The
fragments must pass the apps' own strict parsers; that check
lives in the caller.

## Validate and build

```sh
scoot-look validate ./moonrise --origin user
scoot-look build ./moonrise --out /tmp/moonrise --wallpaper-abs /tmp/moonrise/wallpaper.png
```

`validate` refuses malformed files, unknown keys, values outside
the grammar, non-allowlisted `[apps.*]` keys and contrast
shortfalls per origin. `build` writes `scoot.toml`, `bar.toml`
and `render.json` into the output dir. On Nix the same binary
runs inside the render derivation with the look's font packages
as build inputs. Exit nonzero names the field and the reason.

> **Symptom:** `build` says an image wallpaper needs its resolved absolute path.
>
> Pass `--wallpaper-abs` with the image's absolute path. A color
> wallpaper needs no flag.
