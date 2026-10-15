---
title: "The look format: the scoot-look crate, look.toml schema, value grammar and pure validation"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-14"
---

# The look format: the scoot-look crate, look.toml schema, value grammar and pure validation

Filed 2026-10-12, child of [`live-looks`](live-looks.md). Serves **both**:
every other child reads looks through this crate.

## The gap

There is no look format: the registry is Nix attrsets
(`nix/modules/desktop.nix:1931-2147`), every per-app mapping is Nix code
(`theme-look.nix:49-59`, `theme-home.nix`, the app modules), and nothing
bounds what a look's strings may contain once they are written into
line-based configs where keys run code (mako `on-button-*=exec`,
`doc/mako.5.scd:44-54,100-114`; fuzzel `launch-prefix`/`terminal`,
`doc/fuzzel.ini.5.scd:246,267`; GTK 3 `gtk-modules` loading an absolute
path, `gtk/gtkmodules.c:180-181`; all at the pinned nixpkgs).

## What to do

A new crate `crates/scoot-look`, **pure** (no Wayland, no fontconfig, no
image decoder, no process spawning; builds on macOS), per the design doc's
section 3:

- The `look.toml` schema v1 (section 3.2): the thirteen palette roles with
  their defaults, `[terminal]`, `[fonts]` with `popup-size`, `app-size`,
  `terminal-size` and the `files`/`nix` carriers, `[opacity]`, `[shape]`
  (no `gap` in v1), `[cursor]` (`accent`, optional `body`), the
  `[wallpaper]` unit, `[icons]`, `[night]`, `[apps.<app>]`.
- `extends` resolution (cycle-checked, depth at most 8) over *source*
  looks only, and never to an installed look (origin by directory, section
  3.1).
- The value grammar of section 3.5: names, identifiers (starting with a
  letter or digit, so never `.` or `..`; ASCII-only by decision), Nix
  attribute paths for `[fonts.nix]`, display text, glyph strings, paths,
  `https://` wallpaper URLs with a required 64-hex `sha256` (host a DNS
  name with a dot: no userinfo, IP literal, port, `localhost` or
  `.localhost`/`.local`/`.internal`/`.home.arpa` name; the host is
  lowercased and one trailing dot stripped *before* those checks, so
  `LOCALHOST.` and `foo.local.` are refused; and a host whose last label is
  all digits or starts with `0x` is refused, because curl and the WHATWG URL
  parser read `127.1` and `0x7f.0.0.1` as IPv4 (F2); IDNs and punycode are
  fine given the hash pin), and numbers,
  each with its charset and length cap; `[cursor] accent` takes `#rrggbb` or
  `"none"`; NUL and control characters refused everywhere; a 64 KiB file
  cap.
- Contrast (section 3.5): the four refusing pairs, `dim` and `urgent`
  reported; refusal or warning by origin (shipped/installed vs the user's
  own).
- The per-app key allowlists in one place (scoot, scootbar, the shell apps'
  fragments as their plan publishes them), shared by the validator and every
  loader.
- Pure renderers for the first-party fragments (`scoot.toml`, `bar.toml`)
  taking resolved inputs (font files, the absolute wallpaper path, the
  target binary's features: `popup-radius` only with scootbar's `popup`
  feature, `crates/scootbar/src/config.rs:396-401`).
- A small `scoot-look` binary target with no Smithay in its closure (for Nix
  builds and the bar-only path).

## Evidence to record

Unit tests per rule (each refusal named), property tests on the parser
(malformed TOML, deep `extends` chains, huge files), and grammar tests that
feed a newline, `=`, `[`, a quote and NUL into every string token. The
crate's dependency list and the `scoot` binary size before and after.

## Not in this ticket

The shipped looks (`look-shipped`), reading the layer in any app, the CLI,
Nix, the third-party renderers.

The design this implements against: [live looks](../../../dev/research/live-looks-design.md).

## Resolution (PR #608, branch `look-format`; review-fix pass)

Review findings 1-8 fixed in the `scoot-look` crate, each reproduced
with a failing test first (see the PR body for the finding-by-finding
record):

- Dependency list: `serde` (workspace) plus `toml = "1"` only. `toml`
  adds no new package to the tree: `scoot` and `scootbar` already pin
  it (`Cargo.lock`: `toml 1.1.6+spec-1.1.0`), and no other crate
  depends on `scoot-look`, so no dependency cycle is possible.
- The `scoot` binary is unchanged: the pass touches only
  `crates/scoot-look` and the site reference page
  (`site/src/content/docs/reference/look-format.md`); nothing else
  in the workspace reads the crate yet.
- Rendered fragments verified against the real parsers where
  reachable: an override-heavy `bar.toml` passes `scootbar daemon
  --check` (`ok`, exit 0) on Linux. `scoot` has no `--check` and its
  look-layer loader is a later ticket, so the `scoot.toml` fragment
  is validated by reparse plus key-by-key comparison against
  `AppearanceConfig` and the wallpaper `Table`.
