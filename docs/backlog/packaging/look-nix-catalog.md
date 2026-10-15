---
title: "Nix: the programs.scoot.looks catalog, look as the declared default, the switch rule"
status: "open"
area: "packaging"
priority: "high"
blocked: null
---

# Nix: the programs.scoot.looks catalog, look as the declared default, the switch rule

Filed 2026-10-12, child of [`live-looks`](../core/live-looks.md). Serves
**daily-drive**: the Nix path is how the maintainer runs the desktop.

## The gap

`programs.scoot.desktop.look` is an enum (`desktop.nix:195-204`) merged into
`config.toml` and `bar.toml` at module priorities (`home.nix:532-539`,
`scootbar.nix:374-378`), so a swap can never change a key the look set, and a
`home-manager switch` has no rule for a choice made at runtime.

## What to do

Per the design doc's sections 3.1, 5.2 and 6:

- `programs.scoot.looks` (names and source paths, read with
  `builtins.fromTOML`; no derivations, no IFD) and `programs.scoot.look`
  (D2), with the profile's default moved from `mkDefault` to
  `mkOverride 1200 "ginger-night"` (`home.nix:503-505`,
  `nixos.nix:2235-2237`), leaving priority 1000 for the Stylix source and
  1500 to the option's `default = null`. Breaking (`feat(nix)!`), the
  maintainer's config updated in step. The changelog states the visible
  change for every profile user under D15: the shipped looks' shape and
  opacity (rounded windows, a translucent padded bar), which the Nix path
  never applied before, and the two-key opt-out.
- Each look rendered by a derivation running the `scoot-look` binary with
  its `[fonts.nix]` packages as inputs (D13); the catalog linked as
  `~/.local/share/scoot/looks/<name>` (sources) and
  `~/.local/share/scoot/rendered/<name>` (renders); the declared default in
  `~/.config/scoot/look-default`.
- Remove the eval-time merges of look values into `programs.scoot.settings`
  and `programs.scootbar.settings`.
- The activation step `scoot look sync` (the rule of section 6.3), ordered
  after `linkGeneration` and `dconfSettings`.
- **`color-scheme` moves into `sync`, in this ticket** (D11, review N3):
  home-manager stops writing it from the eval-time look
  (`theme-home.nix:628-631`) only because `sync` now writes it from the
  active look's `polarity`, so a dark look never loses GTK's dark-mode
  signal before `look-toolkit-live` adds the other keys and the swap-time
  writes. The module records the names of every key left in the merged
  `config.dconf.settings."org/gnome/desktop/interface"` (set by the user or
  another module, since scoot no longer defines any) in the catalog's
  metadata file beside `look-default`, and `sync` skips those; `sync` also
  skips a key whose current value is neither unset nor what it last wrote
  (recorded in `<state>/look.json`), **except** that with no record yet and
  no claim in that list it adopts the key (R1). On the migration switch
  home-manager itself normally resets the old `color-scheme`, since it runs
  `dconf reset` on keys a generation stops managing
  (`modules/misc/dconf.nix:20,120,156` at `efa3ccb4`, the rev
  `scripts/scootbar-stylix-test.sh:35` pins) and `sync` runs after that
  step. That cleanup is skipped when there is no old-generation state
  (`dconf.nix:143`), and adoption covers that case. The same one-key write
  runs on `scoot look apply` (`look-apply-cli`).
- The NixOS-module-only path: the catalog under
  `/run/current-system/sw/share/scoot/`, `/etc/scoot/look-default`, and
  `scoot look sync` run by `resources/scoot-session` at login.
- Docs in the same PR: `site/.../scoot/theming.md` and
  `site/.../desktop/index.md`.

## Evidence to record

Module tests for each corner of section 6.3 (`look = null` and a rollback
included), for the priority table (Stylix + profile evaluates to `stylix`;
an explicit look wins; the profile alone gives ginger-night), a VM run of
`home-manager switch` keeping a live choice and following a changed default,
and the catalog's closure size.

## Not in this ticket

Stylix, the greeter, third-party leaves, retiring the registry,
`lookPacks` (later, `look-share`).

The design this implements against: [live looks](../../../dev/research/live-looks-design.md).
