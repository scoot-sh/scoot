---
title: "Nix package: `packages.scootbar`, overlay, `nix run`, CI and eval tests, from the first milestone"
status: "open"
area: "scootbar"
priority: "high"
blocked: null
milestone: "M1"
---

# Nix package

Filed 2026-09-29. Serves **daily-drive**: a bar nobody can install with one
line is not iterating with users. It ships with the **first** milestone that
runs, not after config and modules; every later milestone then lands already
installable. The NixOS and home-manager modules follow in
[nix-modules-and-stylix](nix-modules-and-stylix.md).

Modeled on how scootbg is packaged in `flake.nix` (its own derivation, overlay
entry and eval tests in `nix/tests.nix`).

## What to build

- **`packages.scootbar`**: `rustPlatform.buildRustPackage` with
  `cargoBuildFlags = [ "-p" "scootbar" ]` so only the one binary is in
  `$out/bin`, `stripAllList`, `doCheck = false` (CI runs the tests), `meta`
  description read from `crates/scootbar/Cargo.toml` like `scootbgDescription`
  (no drift), `mainProgram = "scootbar"`. Linux only; the crate builds a stub
  elsewhere, so no Darwin package, as scootbg.
- **Source fileset**: `src` already unions `Cargo.toml`, `Cargo.lock` and
  `crates/`; add `scootbar/fuzz` to the exclusion beside scootbg's so the fuzz
  crate never enters the build (`nix-src-fileset-done.md`).
- **Overlay** `pkgs.scootbar` next to `pkgs.scoot` and `pkgs.scootbg`, and
  `nix run .#scootbar`.
- **No font in the package's closure.** scootbg's benchmark counts installed
  disk with its non-glibc closure, so a bundled font would show up as a loss.
  The bare binary takes a font from `--font` (a config key later) or a short
  fixed list of well-known directories, and otherwise **refuses to start with a
  message naming how to give it one**; the module and Stylix supply the path. On
  NixOS those directories are usually empty, so `nix run .#scootbar` needs
  `--font` or the demo output below.
- **A demo output, `packages.scootbar-demo`**: `scootbar` wrapped with a small font
  from nixpkgs as its default `--font`, so `nix run .#scootbar-demo` works on a
  clean NixOS box on day one. It is a separate output: the bare `scootbar` (and
  its measured closure) never carries the font, and the modules do not use it.
- **Cargo features as a variant**: `--no-default-features --features ...` is
  reachable by `.override`, tested to build.
- **CI**: a `scootbar` path filter; `nix flake check -L` covers the new output
  on every PR; `nix build .#scootbar` main-only with the others
  (`ci-nix-packaging-done.md`); an `ldd` assertion that the binary links only
  libc, libm and libgcc_s.
- **Eval tests** in `nix/tests.nix`: the overlay provides `pkgs.scootbar` on
  Linux and not on Darwin.
- **Docs**: a short section in `docs/nix.md` (what, why, the font rule); the
  README changes only if what scoot is changes.

## Done when

`nix run github:scoot-sh/scoot#scootbar` starts the bar from a clean checkout,
CI builds it, and the closure is measured and recorded for the
[resource ratchet](lightest.md).
