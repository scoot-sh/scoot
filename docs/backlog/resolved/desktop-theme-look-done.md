---
title: "Desktop look: fonts, cursor, GTK/Qt theme and dark mode from the look choice"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-06"
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

## Resolution (2026-10-06)

Landed as `feat(nix): desktop look themes fonts, cursor, GTK/Qt, greeter
and app files` (PR #474 — this entry resolved in the PR): `desktop.theme`
filled from `desktop.look` (user > Stylix > look, per key) — the look's
cursor in the compositor config, GTK `settings.ini` + `GTK_THEME`, qt6ct
config + `QT_QPA_PLATFORMTHEME`/`QT_PLUGIN_PATH` (`Adwaita`/`Adwaita-Dark`,
capitalized exactly as the plugin registers them), the dark-mode
preference from the look's polarity, the fontconfig default (UI face for
sans-serif, terminal face for monospace), the look's UI face as the bar's
font file, and the look's own app files from the flake (foot, starship,
Helix, btop — static where hand-tuned, generated where mechanical). The
greeter wears the look too (backdrop pairing, dark setting, CSS, font).
Per-target opt-outs under `theme.targets.<name>.enable`; no daemon runs
for any of it. Picks: Vanilla-DMZ cursor (3.3 MiB vs Bibata's 322 MiB),
Adwaita icons (the toolkit default). Docs on the site's desktop page
(App theme) and theming page.

Evidence: `nix build .#checks.aarch64-linux.scoot-modules` green on the
Asahi M2 (eval pins for per-key precedence, Stylix-absent path, unknown
look, every target opt-out, greeter pairing; content checks for every
generated file), `nix build .#docs-site` green, and a real `scoot-test`
greetd login per look (GTK file chooser, qt6ct with `Adwaita-Dark`
selected and no warning, foot, bar font, themed greeter config;
screenshots `theme-*.png` beside the implementer report). Two bugs the
live proof caught and fixed: `--` inside the generated fontconfig XML
comment (illegal; fontconfig refused the file) and the missing
`QT_PLUGIN_PATH` (the platformtheme never loaded, style fell back).
