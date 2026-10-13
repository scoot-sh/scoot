---
title: "The look format: look.toml tokens, the scoot-look crate, validation, the five looks lifted"
status: "open"
area: "core"
priority: "high"
blocked: null
---

# The look format: look.toml tokens, the scoot-look crate, validation, the five looks lifted

Filed 2026-10-12, child of [`live-looks`](live-looks.md). Serves **both**:
every other child reads looks through this crate, and a validated format is
what lets an agent pin a look and trust the result.

## The gap

There is no look format. The registry is Nix attrsets with 3 compositor and
6 bar colors per look (`nix/modules/desktop.nix:1931-2147`; radial-burst
has no `hover`, 2032-2038). The example directories mix the look with a
layout and commands (`docs/examples/moonrise/bar.toml:19-21,58-86`). Nothing
compares the two. Every per-app mapping is Nix code
(`theme-look.nix:49-59`, `theme-home.nix`, `notifications-home.nix:216-232`,
`idle-home.nix:31-46`, `audio-home.nix:36-75`, `fuzzel-theme.nix:15-21`,
`inputmethod-home.nix:43-74`), so a non-Nix box has none of them. The AA
contrast check runs on generated `gtk.css` only (`nix/tests.nix:14907-14932`).

## What to do

- A new pure crate `crates/scoot-look` (no Wayland, no I/O beyond reading a
  look directory; builds on macOS): the `look.toml` schema v1 exactly as the
  design doc's §3.2 gives it (required roles, defaults for optional roles,
  `[terminal]`, `[fonts]`, `[shape]`, `[cursor]`, `[wallpaper]`, `[icons]`,
  `[night]`, `[apps.<app>]`), `extends` resolution (cycle-checked, depth at
  most 8), and strict validation (§3.5): hex colors, required roles, the
  contrast bars (foreground/background 4.5, accent-fg/accent 4.5,
  accent/background 3.0, focus/background 3.0), names, license, the
  per-app key allowlist of §3.4.
- Pure renderers for the first-party fragments: scoot (`[appearance]`,
  `[layout] gap`, `[wallpaper]` without `command`) and scootbar (`[colors]`,
  the `[bar]` style keys, the per-module visual allowlist). Paths made
  absolute against the look directory.
- Lift the five shipped looks into `look.toml` files with no invented color:
  the registry's values, the example `scoot.toml`/`bar.toml` style values
  (radius, ring widths, gap, opacity), foot's 16 hues into `[terminal]`.
  Place them per Decision D1 (recommended: top-level `looks/<name>/`, with a
  CI path filter like scootbg's, since `docs/*` runs no CI:
  `.github/workflows/ci.yml:236-246`).
- A check that each lifted `look.toml` equals the Nix registry entry until
  [`look-nix-catalog`](../packaging/look-nix-catalog.md) retires the
  registry, so the two cannot drift meanwhile.
- `cargo fuzz`-style property tests on the parser (malformed TOML, deep
  `extends` chains, huge files: cap the file size like scootbar's 64 KiB,
  `crates/scootbar/src/config.rs:77-79`).

## Evidence to record

Unit tests per rule (each validation refusal named), the five looks
validating, a byte-identical render check against today's generated values
for the 3+6 colors, and the parse cost of a look (cold, microseconds).

## Not in this ticket

Reading the layer in any app, the CLI, Nix, the third-party renderers
(those land with their tickets, in this crate).

The design doc this implements against: [live looks](../../../dev/research/live-looks-design.md).
