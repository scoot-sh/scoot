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
5. **Nix.** `programs.scoot.look.targets.<name>` takes a template, output
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
