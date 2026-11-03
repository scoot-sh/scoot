---
title: "User and community look targets: render any app's theme from a template, like pywal templates and Stylix targets"
status: "open"
area: "core"
priority: "medium"
blocked: null
---

# User and community look targets: render any app from a template

Filed 2026-10-17, child of [`live-looks`](live-looks.md), after
`look-install`. Serves **daily-drive**: the apps people want themed
(Ghostty, kitty, Alacritty, WezTerm, tmux, zathura, rofi, browsers) are
too many for scoot to hard-code, and pywal and Stylix get their reach
from templates and a target list that others extend.

## The gap

Every themed app is a Rust renderer plus an allow-list entry in
`scoot-look` (foot, mako, wob, fcitx5, fuzzel, swaylock, GTK, Qt, and the
static `apps/` leaves for Helix, btop and starship). A new app means a
scoot release. A user cannot add Ghostty or kitty to their own setup, and
an author cannot ship one in a look without scoot knowing the format.

## What to do

A target is a template plus a line saying where it goes and how to tell
the app. Shape to settle in the ticket:

1. **Templates.** A directory of templates, user-owned
   (`~/.config/scoot/look-targets/<name>/`) or shipped in a pack. Each has
   a `target.toml` (output file, optional reload) and a template file with
   token placeholders (`{{ palette.background }}`, `{{ terminal.color4 }}`,
   `{{ fonts.mono }}`, hex with and without `#`, rgb, alpha). Pure
   substitution, no logic that can run code.
2. **Render on every swap**, into the look's state dir next to the
   built-in fragments, under the same atomic swap, cache key and rollback
   rules, with `look validate` and `look build` covering them. Wiring into
   the app's own config stays an include or a link, written once with the
   own-if-absent-or-marked rule (or by hand).
3. **Reload.** Many terminals reload on file change (Alacritty, WezTerm,
   Ghostty on a signal, kitty by `kitty @ set-colors`). A reload is a
   signal to the user's own processes or a command the **user** declares in
   their own config. Never a command taken from a look.
4. **Trust.** Installed looks (someone else's) may supply token data and
   per-app overrides for targets the user has enabled, never templates,
   output paths or reload commands. Templates from a pack the user adds are
   the user's choice, shown first at add time, like `--trust`.
5. **Nix.** `programs.scoot.lookTargets.<name>` takes a template, output
   and reload, parallel to Stylix targets; the declared look still wins.
6. **A starter pack**, maintained in-tree: kitty, Ghostty, Alacritty,
   WezTerm, tmux, zathura, rofi, plus Chrome/Chromium (the
   `BrowserThemeColor` policy sets one color) and Firefox
   (`userChrome.css`, behind an explicit opt-in). Browsers are the hard
   part: no live reload for Firefox without an extension, and the Chrome
   policy needs root or a managed path. Say in the docs what each
   achieves.
7. **Import.** `look import` from pywal-style color schemes and Base16 is
   already the source side (`import-base16`); keep targets and sources
   separate.

Edge cases to pin: a template naming an unknown token refuses at
validate; a missing app is a note, never an error; two targets writing the
same output refuse; output paths stay under the user's config dirs
(no `..`, no symlink escape); one failing target rolls the swap back like
a refused reload does.

## Not in this ticket

Sharing target packs from git (see [`look-share`](look-share.md)); Windows
or macOS app targets; anything that runs a look-supplied command.

## Resolution in progress — slice 1 (user-owned targets only)

Slice 1 ships the user-owned half: templates in
`~/.config/scoot/look-targets/<name>/{target.toml,<template>}`
(`$XDG_CONFIG_HOME` when set, else `~/.config`, via the shared
`catalog::config_home` rule), rendered on every `scoot look apply`/swap
into the look's state dir under `targets/<output>` next to the
built-in fragments, under the same atomic swap, cache key
(`RENDER_VERSION` 16) and rollback rules; `scoot look validate` and
`look build` cover them (build stages `targets/` under `--out`).

`target.toml` (`scoot-look/src/targets.rs`, strict `deny_unknown_fields`):

```toml
name = "kitty-colors"           # must match the directory name
output = "kitty/colors.conf"    # relative to the config home (grammar:
                                # no `..`, no absolute, no hidden segments)
template = "colors.tmpl"        # file inside the target directory

[reload]
argv = ["kitty", "@", "set-colors", "--all"]  # bare PATH name or absolute
timeout_secs = 10               # optional, 1 to 300, default 30
```

Tokens are the look's own spellings (`{{ palette.background }}`,
`{{ terminal.color4 }}` (`color0`–`color15`, only when the look has a
`[terminal]` table), `{{ fonts.mono }}`, plus `polarity`,
`wallpaper.*`, `icons.theme`, `night.temperature` and the rest of the
resolved tokens in `target_tokens`). Color filters re-spell `#rrggbb`:
`| hex` (identity), `| nohash`, `| rgb` (`R,G,B`),
`| rgba(<0 to 1>)`. Pure substitution with no logic, conditionals,
loops, includes or execution; a template needing a literal `{{` has no
escape in v1, and a stray `}}` stays literal (JSON nests `}}`).
Unknown tokens, unknown filters, filters on non-colors and unclosed
`{{` refuse at validate — so a typo never swaps a half-rendered file.

Reload runs the user's own argv directly (never through a shell: no
splitting, globs or `$VAR`), with the declared bounded wait. Tension
with the ticket's "one failing target rolls the swap back like a
refused reload" resolved explicitly: a *render* failure (unknown
token, unreadable template, duplicate or escaping output) refuses
*before* anything is recorded or repointed — the swap never started,
exactly like a refused scoped reload. A *reload command* failure
(missing binary, timeout, non-zero exit) is a note naming what happened
and never undoes the swap, which already stands by then.

Edge cases pinned by tests (`scootctl/src/look/apply/tests.rs`,
`scoot-look/src/targets/tests.rs`): unknown token refuses at validate
and records nothing; missing reload binary is a note; duplicate outputs
refuse; absolute/`..`/hidden outputs refuse; a symlinked output prefix
refuses; a symlinked template refuses (`O_NOFOLLOW` like every other
look input); one failing target records and repoints nothing; a
template edit re-renders instead of reusing the stale cache; validate,
build and apply render the same bytes (the build test asserts the
`--out` file equals the swapped one); a sync with targets configured
renders fresh instead of pointing at the Nix catalog render (which was
built without the user's templates). Installed looks can never supply
templates, outputs or reloads: a `look.toml` with `[targets]` or a
target-shaped `[apps.*]` table refuses at parse through the existing
allowlist rules (pinned in `scoot-look` tests). One in-tree example
target (kitty, all sixteen colors) lives under
`docs/examples/look-targets/kitty-colors/` and renders in a test, so
the docs example cannot rot. Docs: `site/.../reference/look-targets.md`
(task-first, type/default/example/reload per option, `Symptom` boxes)
plus a pointer in `look-format.md`.

Later slices (still open): look-supplied token data and per-app overrides for
user-enabled targets; keeping look sources and template sources in
separate imports; browsers. None of these run a look-supplied command, ever.

## Resolution in progress — slice 2 (Nix targets + starter pack)

`programs.scoot.lookTargets.<name>` (home-manager): `{ template
(path or text), output, reload = { argv, timeout_secs } }` generating one
directory symlink `look-targets/<name>` to a store directory of regular
files (`target.toml` plus the template file named `template`), which
reads like the on-disk target slice 1 reads (via `xdg.configFile`, one
`source` per target so hand-written siblings keep working; per-file
entries would land as symlinks the tool refuses via `O_NOFOLLOW`). Named
`lookTargets`, not the ticket's
`look.targets`: `programs.scoot.look` is the declared-default string
and one option cannot be both that value and a parent (verified against
`lib.evalModules` — the declarations collide), so the targets live
beside it with zero churn to every `look` reader. A bad name or output
(including non-ASCII/control characters) fails the evaluation naming the
entry; staying under the config home through symlinks (and the reload
program shape) stays runtime-checked. A sync with any target configured
(declared or
hand-written) renders fresh instead of pointing at the catalog render:
the catalog build (`scoot-look build`, one look's fragments) cannot
absorb per-user templates, so sync stays a pure repoint only in the
no-targets case.

Starter pack (non-browser), next to the kitty example
(`docs/examples/look-targets/<name>/`): Alacritty, WezTerm, Ghostty,
tmux, zathura and rofi — each from the real tokens, each with the right
output path, no `[reload]` anywhere (live reload, keybind, manual or
next-launch per app), each rendering in a test. Browsers stay a later
slice (no live reload for Firefox without an extension, Chrome policy
needs a managed path).

## Resolution in progress — slice 3 (trust for packs)

`scoot look target add PATH [--trust]` (`scootctl/src/look/target_add.rs`):
adds a pack (a directory of target directories, or one target directory)
to the user's own `look-targets/`. Chosen over the other two slices
because it is the only one that adds no execution path and no new schema:
a pack is a directory the user names on the command line, never something
a look carries, so installed looks still cannot supply templates, outputs
or reload commands (the parse refusals are untouched). The verb is
`target add`, not `look add`/`install`, so it cannot be read as installing
a look.

Without `--trust` it prints, per target, what it writes, the `RELOAD`
argv on its own line, and the whole template, then exits 1 having written
nothing (unlike `install`, the templates are the whole payload, so there
is nothing to keep without trust). With `--trust` the same review prints
first and the targets land. Nothing runs at add time.

Pinned by `target_add/tests.rs` and the docs snippet gate: a foreign-pack
gate (symlinked pack, target directory, manifest or template; hard link;
stray file in a target or beside targets, including macOS `._*`; a
`look.toml` directory; nested template path; target-count and total-size
caps), the same validation `look validate` runs plus every token checked
against the union of the catalog's looks, the bytes reviewed equal the
bytes written (each file read once through the `O_NOFOLLOW` reader),
all-or-nothing add (staged in a sibling of `look-targets/`, since a stray
directory inside it would make every later apply refuse), name collisions
(a Nix-declared symlink is never written through), case-insensitive output
collisions against existing targets and inside the pack, and a hostile
review test (ESC, CR, clear-line, BEL and a bidi override in a template
print escaped and cannot imitate the `RELOAD` line; a control byte in a
reload argument refuses at read). The whole `docs/examples/look-targets`
directory adds as one pack of seven. `RENDER_VERSION` is unchanged:
rendering is unchanged.
