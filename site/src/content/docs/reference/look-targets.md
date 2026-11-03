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
with targets configured (hand-written or Nix-declared) renders fresh
instead of pointing at the Nix catalog render (built without the
templates) and says so. An in-tree example (kitty, all sixteen
terminal colors) lives in `docs/examples/look-targets/kitty-colors/`,
beside the six-app starter pack below.

## target.toml

| Option | Type / default | Reload |
|---|---|---|
| `name` | string, required. Matches the directory name (`[a-z0-9-]{1,32}`). | Validated on every apply; a mismatch refuses. |
| `output` | string, required. Relative to the config home (`kitty/colors.conf`). ASCII only (a Unicode name refuses); no `..`, no absolute path, no hidden segments; must stay under the config home through symlinks. | Rendered to `targets/<output>` in the active look on every apply. Two targets naming the same output refuse, as do two where one output is a directory on the way to the other (`x` and `x/y`); both compared case-insensitively. |
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

> **Symptom:** `validate` says two targets write the same output, or a file where another needs a directory.
>
> Both `target.toml` files name the same `output` (the compare is
> case-insensitive, so `Kitty/x` and `kitty/x` collide), or one names `x`
> and the other `x/y`. Rename one: the
> second write would silently win, and no wiring could name both.

> **Symptom:** `validate` says an output escapes the config home or a symlink.
>
> The output names `..`, an absolute path, or a directory that is a
> symlink out of the config home. Point it inside the home; through a
> symlinked directory refuses even when the spelling looks relative.

## Starter pack

Seven maintained targets ship in-tree in `docs/examples/look-targets/<name>/`:
the kitty example above plus the six below (each with its `target.toml` and template, each rendering in a test so the
example cannot rot). Add the ones you want with
[`scoot look target add`](#add-a-pack) (or copy a directory into
`~/.config/scoot/look-targets/<name>/` by hand), then add the wiring line
to the app's own config, once, by hand. Keys you set in your own config after the
include win per key. All six need a look with a `[terminal]` table;
against a look without one `validate` names the missing token. None of
the six declares a `[reload]`: the middle
column says how each app picks up the re-render.

| Target | What it themes | Picks up a swap |
|---|---|---|
| `alacritty` | palette, all 16 terminal colors, mono font | live: Alacritty reloads on file change (`live_config_reload`, on by default), imports included |
| `wezterm` | palette, all 16 terminal colors, mono font and size | live: WezTerm reloads on file change (`automatically_reload_config`, on by default) |
| `ghostty` | palette, all 16 terminal colors, mono font | by keybind: the `reload_config` action (`ctrl+shift+,` by default); Ghostty has no external reload command |
| `tmux` | status line, window list, pane borders, messages | by hand: run `tmux source-file <the rendered file>` (or restart the server); the path is machine-specific |
| `zathura` | page, status bar, input bar, completion, index, search highlight, font | restart: `R` reloads the document, not the config |
| `rofi` | window, input bar, list, selected and urgent rows, messages, font | next launch: rofi reads its theme on every start |

Wire each one at its rendered path (the look state dir's `look` link plus
`targets/<output>`):

```text title="~/.config/alacritty/alacritty.toml"
import = ["~/.local/state/scoot/look/targets/alacritty/colors.toml"]
```

```text title="~/.config/wezterm/wezterm.lua"
local scoot_colors = dofile(os.getenv("HOME") .. "/.local/state/scoot/look/targets/wezterm/colors.lua")
scoot_colors.apply_to_config(config)
```

```text title="~/.config/ghostty/config.ghostty"
config-file = "~/.local/state/scoot/look/targets/ghostty/theme"
```

```text title="~/.tmux.conf"
source-file ~/.local/state/scoot/look/targets/tmux/theme.conf
```

```sh
ln -s ~/.local/state/scoot/look/targets/zathura/theme ~/.config/zathura/scoot-theme
```

```text title="~/.config/zathura/zathurarc"
include scoot-theme
```

```text title="~/.config/rofi/config.rasi"
@import "~/.local/state/scoot/look/targets/rofi/colors.rasi"
```

zathura's `include` resolves relative to the file doing the including,
so the link above keeps the wiring portable; an absolute `include` of
the state path works too. tmux reads `~/.tmux.conf` when its server
starts (`$XDG_CONFIG_HOME/tmux/tmux.conf` on tmux 3.2+); `:source-file`
applies a re-render to the running server.

Nix users declare the same targets with
[`programs.scoot.lookTargets`](../desktop/look.md): each entry lands
as `scoot/look-targets/<name>/{target.toml,template}` in the config
home. A sync with any target configured (declared or hand-written)
renders fresh instead of pointing at the catalog render, which was
built without the templates.

## Add a pack

A pack is a directory of targets someone else wrote: a directory holding
target directories (the starter pack above is one), or a single target
directory. `target add` copies it into your own `look-targets/`. A pack
is not part of a look and not part of `look install`: installed looks
still cannot carry templates, outputs or reload commands.

```sh
scoot look target add ./pack            # print the review, add nothing
scoot look target add ./pack --trust    # print the review, then add
```

Targets render files into your desktop's config and may run a command on
every swap, so the add is gated like `look install --trust`. Without
`--trust` it prints, for each target, what it writes, its `RELOAD` command
on a line of its own (or `reload: none`), and the whole template, then
exits 1 having written nothing. With `--trust` the same review prints
first and the targets land. Nothing runs at add time; the files are read,
checked and copied, and the bytes you read are the bytes written.

| Check | What refuses |
|---|---|
| Source shape | a symlink anywhere (the pack, a target directory, `target.toml` or a template), a hard link, a look directory (it has `look.toml`: use `look install`), a file beside the targets other than `README*`, `LICENSE*` or `COPYING*`, a file in a target beyond `target.toml` and its template, a `template` naming a subdirectory |
| Limits | more than 32 targets, more than 4 MiB in total, a template over 64 KiB or not UTF-8 |
| Each target | the same checks `look validate` runs: name matches its directory, `output` stays under the config home, reload shape, template syntax, and every token spelled by at least one look in your catalog (a token only some looks set refuses later, per look, as usual) |
| Collisions | a target name that already exists in `look-targets/` (a Nix-declared one included: it is never written through), an `output` that an existing target or another target in the pack already writes, or that is a file where another needs a directory (`x` against `x/y`), compared case-insensitively |

On failure the add is all or nothing: every target is checked before the
first one lands, and if one cannot land the ones already moved are
removed again (a crash mid-add is not covered). The review shows letters,
digits and ordinary punctuation as they are and escapes everything else
(controls, zero-width and bidi characters, odd spaces, combining marks) as
`\u{1b}`, so a template or file name cannot rewrite the terminal or
imitate the `RELOAD` line. Each reload argument is quoted when it holds
anything beyond plain path characters, so `echo "a b"` and `echo a b`
read differently. `--json` prints the review as a document after the add
succeeds (`added` is `true`), or with `added` set to `false` when you
omitted `--trust`; a failed add prints only the error. The template and
reload arguments are raw there.

> **Symptom:** `target add` prints the review and exits 1 with "nothing added".
>
> That is the gate working: you ran it without `--trust`. Read the
> templates and the `RELOAD` lines; if you trust the pack, rerun with
> `--trust`.

> **Symptom:** `target add` says a target named `<name>` already exists.
>
> `look-targets/<name>` is already there (yours, or declared in Nix).
> Remove or rename it, or rename the target in the pack; nothing was
> added.

> **Symptom:** `target add` says a target writes an output an existing
> target already writes, or a file where another needs a directory.
>
> Two targets may not write the same file (the second write would
> silently win), and one may not write `x` while another writes `x/y`
> (the render could not make `x` a directory). Change one `output`, or
> remove the existing target.

> **Symptom:** `target add` refuses a symlink, a hard link or an unexpected file.
>
> A pack is foreign input and is read as plain files only. Re-export it
> with `cp -R` or `tar` (macOS `tar` adds `._*` files; set
> `COPYFILE_DISABLE=1`), without links.

## Not here yet

Browsers (no live reload for Firefox without an extension, and the
Chrome policy needs a root or managed path), look-supplied token data and
per-app overrides for the targets you enabled, and keeping look sources
and template sources in separate imports. Each is a later slice; the
ticket lists them.
