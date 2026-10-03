---
title: "A Stylix target for the compositor: window borders, background, cursor, wallpaper"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M6"
resolved: "2026-10-03"
---

# A Stylix target for the compositor: window borders, background, cursor, wallpaper

Filed 2026-10-03. Serves **daily-drive**: the maintainer themes their whole
scoot desktop with Stylix (light polarity, from their wallpaper) and today
sets the workspace pill and the bar's highlights to the scheme's blue by
hand, while the compositor's window rings, background, cursor and wallpaper
get nothing from the scheme at all.

## The gap

`nix/modules/scootbar.nix` has Stylix defaults
(`programs.scootbar.stylix.enable`, six `colors` tokens from
`config.lib.stylix.colors.withHashtag`, font and size), pinned by
`nix/scootbar-tests.nix`, but the compositor's modules
(`nix/modules/home.nix`, `nix/modules/nixos.nix`) have none. Scoot's config
already has the keys a target would set (`docs/configuration.md`
`[appearance]`: `focus_ring_active_color`, `focus_ring_inactive_color`,
`background_color`, `cursor_color`, `cursor_theme`, `cursor_size`; and the
`[wallpaper]` table, `image = ...`, drawn by scootbg) — they just keep their
built-in defaults under a Stylix setup.

## What to do

1. `programs.scoot.stylix.enable` (and the NixOS equivalent if the NixOS
   module renders the config too), built exactly the way `scootbar.nix`
   does it: same detection (`config.lib ? stylix && config.stylix.enable`),
   on by default when Stylix is in use, every value at `lib.mkDefault` so a
   value the user wrote wins, no Stylix input on this flake, nothing
   changes without Stylix.
2. The mapping, confirmed against Stylix's own window-manager targets
   (sway, hyprland, niri, river) at the rev `scootbar-tests.nix` names:
   focused ring, unfocused ring, background, cursor (`stylix.cursor`
   name/size/package), wallpaper (`stylix.image`, `stylix.imageScalingMode`
   against what scootbg supports).
3. The bar accent: change scootbar's Stylix `accent`/`hover` default from
   `base0A` to whatever the compositor's focused ring uses, so the bar's
   highlights and the window ring match out of the box.
4. Tests: extend the nix eval tests the way `scootbar-tests.nix` stubs
   Stylix (precedence user > Stylix > built-in default; nothing without
   Stylix; evaluates with no Stylix option defined).
5. Docs: `docs/nix.md` (the new option, what it sets, the mapping table
   with base16 names) and wherever `docs/configuration.md` mentions Nix
   or Stylix.

## Not in this ticket

Scheme generation, polarity handling beyond what the base16 slots already
encode, GTK/Qt app theming (Stylix's own targets do that), and any change
to scoot's runtime.

## Resolution (2026-10-03, PR #407)

Branch `feat/nix-scoot-stylix`, head `aebd4f21a` at resolve time (plus the
resolve commit itself).

What landed, item by item:

1. `programs.scoot.stylix.enable` in `nix/modules/home.nix` (on by
   default; same detection, `mkDefault`-per-leaf, no Stylix input, inert
   without Stylix). No NixOS equivalent: `nix/modules/nixos.nix` renders
   no config file, so there is nothing to theme there.
2. The mapping, confirmed against nix-community/stylix at `fb28acd`
   (the rev `nix/scootbar-tests.nix` names): sway
   `modules/sway/hm.nix` (focused `base0D`, every unfocused border
   `base03`, background `base00`), hyprland `modules/hyprland/hm.nix`
   (`col.active_border = base0D`, `col.inactive_border = base03`,
   `misc.background_color = base00`), river `modules/river/hm.nix`
   (same three slots). Correction to the brief: there is **no niri
   target** at that rev, nor on current master, so there was nothing to
   check it against — and all three existing targets agree, confirming
   the coordinator's `base0D`/`base03`/`base00` expectation. Cursor from
   `stylix.cursor` (`stylix/cursor.nix`: name/size/package, all or
   none); the package is Stylix's own cursor target's job
   (`stylix/hm/cursor.nix` sets `home.pointerCursor`, which installs it
   and puts `share/icons` on the lookup path scoot searches), so the
   module only names theme and size. `cursor_color` has no Stylix
   convention and is untouched. Wallpaper `image`/`mode` from
   `stylix.image`/`stylix.imageScalingMode` (`stylix/palette.nix`:
   `stretch`/`fill`/`fit`/`center`/`tile`, default `fill`), gated on
   `image != null`, mapping 1:1 onto scootbg's five modes. A user
   `wallpaper.color` next to Stylix's `image` stays refused fail-safe
   (documented, not worked around).
3. The bar accent: `accent`/`hover` `base0A` -> `base0D` in
   `nix/modules/scootbar.nix`, with the comment, `docs/nix.md`,
   `nix/scootbar-tests.nix` and `scripts/scootbar-stylix-test.sh`
   updated. Only the Stylix default moved; the bar's own default stays
   yellow.
4. Tests in `nix/tests.nix` (Stylix stub; themed-settings equality,
   per-leaf user-wins for all 7 leaves, tile pass-through, both
   switches off, no-cursor, no-image, cursor-package-never-installed,
   rendered-TOML content) and the `scootbar-tests.nix` accent move.
5. Docs: `docs/nix.md` (new `Stylix` section with the mapping table,
   the `stylix.enable` row, the accent change stated plainly) and
   `docs/configuration.md` (`[appearance]` and `[wallpaper]` notes).

Evidence (dev VM, aarch64-linux, Nix 2.34.8, tree at `aebd4f21a` via
`git archive`, extracted to `/tmp/stylix-check2`):

- `nix build /tmp/stylix-check2#checks.aarch64-linux.scoot-modules
  /tmp/stylix-check2#checks.aarch64-linux.scootbar-modules` -> exit 0,
  including the new `ok: Stylix defaults render (appearance, cursor,
  wallpaper + command)` content check and the real `scootbar daemon
  --check` over the re-themed files. Store paths:
  `/nix/store/pln170lzb1ap9yl4af4hb1il875j8cic-scoot-modules-check`,
  `/nix/store/j9x2y1yj6cj4h26fk06hvbl0bcivvlzq-scootbar-modules`.
- Fail-before, same machine: pre-change `home.nix` + new `tests.nix`
  fails at eval; pre-change `scootbar.nix` + new `scootbar-tests.nix`
  fails naming exactly the two accent pins.
- Light-palette render (`/tmp/stylix-render.nix`, kept on the VM):
  `scoot-config.toml` carries `focus_ring_active_color = "#1d4ed8"`,
  `background_color = "#f5f0e6"`, `cursor_theme =
  "Bibata-Modern-Classic"`, `cursor_size = 24`, `[wallpaper]` with the
  Stylix image, `mode = "fill"` and the injected scootbg `command`;
  `bar.toml` carries `accent = hover = "#1d4ed8"`.
- `nixfmt --check` clean on all tracked `.nix` (one new block
  reformatted). Full `nix flake check` and
  `scripts/scootbar-stylix-test.sh` (needs flake fetches + the palette
  generator build) are for CI, which is running on PR #407.
