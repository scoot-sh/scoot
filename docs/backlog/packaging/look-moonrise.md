---
title: "Fourth example look: moonrise (chill), from an Unsplash illustration"
status: "open"
area: "packaging"
priority: "medium"
blocked: null
---

# Fourth example look: moonrise (chill), from an Unsplash illustration

Filed 2026-10-04. Serves **daily-driving**: one more complete desktop look
for the `programs.scoot.desktop.look` registry (dark and calm, beside the
warm vinyl-sunset, the light music-desk and the loud radial-burst), so a
user who lives in scoot all day has a night-sky option.

## The gap

Three looks ship in `docs/examples/` (`vinyl-sunset`, `music-desk`,
`radial-burst`), each registered in `nix/modules/desktop.nix` with eval
pins in `nix/tests.nix` and a row in `docs/nix.md`'s look table. The
maintainer asked for a fourth, chill one from the Unsplash illustration
"Silhouetted trees under moon and stars" by saatvik 5554
(https://unsplash.com/illustrations/silhouetted-trees-under-moon-and-stars-jwBJOj6gakI):
black tree silhouettes, a coral/amber disc, a slate-navy to dusty-rose sky.

## What to do

- Ship the illustration as `docs/assets/wallpapers/moonrise.png` (Unsplash
  License, credited in `NOTICE` and the look README like the other two
  shipped wallpapers), optimized without visible loss.
- Add `docs/examples/moonrise/` built the same way as the other three:
  `scoot.toml`, `bar.toml` with command modules, `foot.ini`,
  `starship.toml`, Helix theme + config, btop theme + config, lazygit,
  `regreet.css`, README with a file table, and a reproducible IPC
  screenshot preview. Chill = calm contrast, the disc amber as accent,
  navy/mauve surfaces, cream text at WCAG AA for body text.
- Register `moonrise` in `nix/modules/desktop.nix` (enum + palettes +
  wallpaper), eval pins in `nix/tests.nix`, a row in `docs/nix.md`'s table.
- Keep hunks in the shared nix files small: `f435`
  (`feat/nix-desktop-idle-lock`) and `wurl` (`feat/wallpaper-from-url`)
  touch the same files.

## Not in this ticket

- No new theming machinery: the look fills the existing `looks` schema
  (role-named colors so the coming registry absorbs it without rework).
  The `desktop-theme-look` child owns fonts/cursor/GTK/dark-mode.
- No rename: "moonrise" stands unless the maintainer picks the report's
  alternative.
