---
title: Look targets
description: "Theme any app from a template: target.toml, tokens, reload, wiring."
---

One look themes every app together, including apps scoot never
hard-codes. A target is a template plus a line saying where the
rendered file goes and how to tell the app. Make one per app you want
themed; `scoot look apply` renders each into the active look beside the
built-in fragments and runs your reload command.

## Make a target

Targets live in `~/.config/scoot/look-targets/<name>/` (`$XDG_CONFIG_HOME`
when set, else `~/.config`), one directory per target:

```sh
mkdir -p ~/.config/scoot/look-targets/kitty-colors
```

Write `target.toml` (what the target is) beside the template (what the
app reads):

```toml title="~/.config/scoot/look-targets/kitty-colors/target.toml"
name = "kitty-colors"
output = "kitty/colors.conf"
template = "colors.tmpl"

[reload]
argv = ["kitty", "@", "set-colors", "--all"]
timeout_secs = 10
```

```text title="~/.config/scoot/look-targets/kitty-colors/colors.tmpl"
background {{ palette.background }}
foreground {{ palette.foreground }}
color4 {{ terminal.color4 }}
```

Apply the look. The template renders into the active look under
`targets/kitty/colors.conf`, next to `scoot.toml` and `bar.toml`, and
the reload runs:

```sh
scoot look apply moonrise
```

The swap is atomic: every target renders before anything is recorded,
under the same swap, cache key and rollback as the built-in fragments.
`look validate` and `look build` cover targets the same way. A sync
with targets configured renders fresh instead of pointing at the Nix
catalog render (built without the user's templates) and says so. An
in-tree example (kitty, all sixteen terminal colors) lives in
`docs/examples/look-targets/kitty-colors/`.

## target.toml

| Option | Type / default | Reload |
|---|---|---|
| `name` | string, required. Matches the directory name (`[a-z0-9-]{1,32}`). | Validated on every apply; a mismatch refuses. |
| `output` | string, required. Relative to the config home (`kitty/colors.conf`). ASCII only (a Unicode name refuses); no `..`, no absolute path, no hidden segments; must stay under the config home through symlinks. | Rendered to `targets/<output>` in the active look on every apply. Two targets naming the same output refuse (compared case-insensitively). |
| `template` | string, required. The template file inside the target directory. | Read on every apply (symlinks refuse); an unreadable template refuses the swap. |
| `reload.argv` | array of 1 to 16 strings, optional. The program plus its arguments: a bare name on `PATH` or an absolute path. | Run directly after a successful swap, never through a shell (no splitting, no globs, no `$VAR`). |
| `reload.timeout_secs` | whole number, default `30`, 1 to 300. | The hung child and the processes it started are killed after the wait (a process that starts its own session is not reached); the swap stands either way. |

Unknown keys refuse: `target.toml` takes only the keys above. A reload
needs no entry to render; without `[reload]` the file still renders and
nothing runs.

Wire the app's own config at the rendered file once, with an include or
a link. Keys you set after the include win per key. For kitty:

```text title="~/.config/kitty/kitty.conf"
include ~/.local/state/scoot/look/targets/kitty/colors.conf
```

(`~/.local/state/scoot` is the default state dir; the `look` link
inside it points at the active render.) Leave a foreign file alone:
when the app's own module or another tool owns the path, keep that
ownership and let the look stay out of it.

## Template tokens

A token is `{{ dotted.path }}`, exactly the look's own spelling
(`palette.accent-fg`, `terminal.color4`, `fonts.mono`). Tokens a look
does not set (a missing `[terminal]` table, an unset `[effects]` key)
refuse instead of rendering empty. There are no conditionals, loops, or
includes: pure substitution, and templates are data — nothing in one
ever executes. A template needing a literal `{{` has no escape in v1;
a stray `}}` with no opening `{{` stays literal (JSON nests `}}`).

| Token | Renders |
|---|---|
| `palette.background`, `palette.foreground`, `palette.dim`, `palette.accent`, `palette.urgent`, `palette.surface`, `palette.hover`, `palette.focus`, `palette.border`, `palette.accent-fg`, `palette.selection`, `palette.selection-text`, `palette.match` | the look's `#rrggbb` roles |
| `terminal.color0` … `terminal.color15` | the look's sixteen hues, only when it has a `[terminal]` table |
| `fonts.ui`, `fonts.sans`, `fonts.mono` | family names |
| `fonts.size`, `fonts.popup-size`, `fonts.app-size`, `fonts.terminal-size` | points (`10`, never `10.0`) |
| `opacity.bar`, `opacity.popups`, `opacity.terminal` | `0` to `1` |
| `shape.radius`, `shape.ring-width`, `shape.ring-inactive-width` | whole numbers |
| `effects.shadow`, `effects.fill`, `effects.motion`, `effects.motion-duration-ms`, `effects.motion-bounce` | only when the look sets them |
| `cursor.theme`, `cursor.size`, `cursor.accent`, `cursor.body` | `body` only when the look sets it |
| `polarity` | `dark` or `light` |
| `wallpaper.color`, `wallpaper.mode`, `wallpaper.fill`, `wallpaper.transition` | only when the look sets them |
| `icons.theme` | the icon theme name |
| `night.temperature` | only when the look sets it |

Color filters re-spell a `#rrggbb` token: `{{ palette.accent }}`
(`#FF9A30`), `{{ palette.accent | hex }}` (same),
`{{ palette.accent | nohash }}` (`FF9A30`),
`{{ palette.accent | rgb }}` (`255,154,48`),
`{{ palette.accent | rgba(0.5) }}` (`rgba(255,154,48,0.5)`). A filter on
anything but a color refuses, as does an unknown filter or token.

The reload runs after the swap stands: a zero exit counts as reloaded,
anything else (missing binary, timeout, non-zero exit) is a note naming
what happened. A reload that writes past 64 KiB on stdout or stderr is
killed instead, and the note says its output was truncated. A failing
reload never undoes the swap. A failing render
does the opposite: unknown token, unreadable template, duplicate or
escaping output — the swap refuses before anything is recorded, the way
a refused reload does.

Installed looks (someone else's) can never supply templates, output
paths, or reload commands. A `look.toml` with a `[targets]` table or a
target-shaped `[apps.<app>]` table refuses at parse through the
allowlist, so only these user-owned directories are ever read.

> **Symptom:** `validate` names an unknown token.
>
> The template spells a token no look carries (`palette.backround`,
> `terminal.color4` against a look with no `[terminal]` table). Fix the
> spelling or pick a look that sets it; a typo never swaps.

> **Symptom:** the reload did not run and the reply says the app is not installed.
>
> The reload binary is missing from `PATH` (or the absolute path is
> wrong). Install the app or fix the argv; the rendered file is already
> in the active look, so a manual reload (or the next swap) picks it up.

> **Symptom:** `validate` says two targets write the same output.
>
> Both `target.toml` files name the same `output` (the compare is
> case-insensitive, so `Kitty/x` and `kitty/x` collide). Rename one: the
> second write would silently win, and no wiring could name both.

> **Symptom:** `validate` says an output escapes the config home or a symlink.
>
> The output names `..`, an absolute path, or a directory that is a
> symlink out of the config home. Point it inside the home; through a
> symlinked directory refuses even when the spelling looks relative.

## Not here yet

Trust for packs (`--trust` showing a pack's templates at add time), the
Nix shape (`programs.scoot.look.targets`), a starter pack beyond the
kitty example, browsers (no live reload for Firefox without an
extension, and Chrome's policy needs a managed path), and keeping look
sources and template sources in separate imports. Each is a later
slice; the ticket lists them.
