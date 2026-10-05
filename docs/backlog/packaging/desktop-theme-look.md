---
title: "Desktop look: fonts, cursor, GTK/Qt theme and dark mode from the look choice"
status: "open"
area: "packaging"
priority: "medium"
blocked: null
---

# Desktop look: fonts, cursor, GTK/Qt theme and dark mode from the look choice

Filed 2026-10-04, child 11 of `desktop-paved-path`. Serves
**daily-drive** (a `look` that themes the compositor and bar but leaves GTK
apps, cursor and dark mode mismatched is a half look).

## The gap

Stylix wiring exists for scoot + bar (`nix/modules/home.nix:136-172`,
`docs/nix.md:362-397`) and cursor via Stylix — but no GTK/Qt settings, no
dark-mode signal (`GTK_THEME` / `color-scheme` preference), no fontconfig
default from the look, and no non-Stylix fallback. The three example looks
hand-write `foot.ini`, `starship.toml`, Helix/btop themes and greeter CSS as
loose files with no flake path applying them.

## What to do

Fill the `desktop.theme` slot — derive the whole app theme from
`desktop.look`, with and without Stylix:

- Fonts: a default font package from the look + fontconfig default (say the
  face; must satisfy scootbar's `--font` file-path need —
  `docs/nix.md:766-805` — so the bar never falls back to its refusal).
- Cursor theme + size from the look when Stylix is absent (Stylix path
  exists; keep its precedence: user > Stylix > look > compositor default).
- GTK (settings.ini + `GTK_THEME`), Qt (`qt6ct`/platformtheme + color
  scheme), and dark-mode preference from the look (each look declares
  light/dark — `music-desk` light, `vinyl-sunset`/`radial-burst` dark).
- Apply each example look's app files (foot palette, starship, Helix, btop,
  greeter CSS + backdrop pairing) from the flake instead of copy-paste —
  generated from one palette definition per look where feasible (say what
  stays a static file and why).
- Edge cases: XWayland apps reading XSETTINGS/Xft.dpi (compositor publishes
  scale — `docs/protocols.md` XWayland section; theme must not fight it);
  flatpak apps seeing the theme (sandbox filesystem notes); look change
  applying without re-login for what can move live.

Acceptance: eval pins in `nix/tests.nix` (per-key precedence incl. the
Stylix-absent path; unknown look fails); real-login proof on the M2 (GTK +
Qt apps, cursor, dark-mode app, bar font all match the look; screenshots);
docs in `docs/nix.md`.

## Not in this ticket

Authoring new looks (three exist); a Stylix colorscheme generator (Stylix
upstream's job); icon themes beyond a default pick (say the pick).
