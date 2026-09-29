---
title: "Testing and CI: harnesses, fuzz targets, path-filtered CI and a bench script"
status: "open"
area: "scootbar"
priority: "medium"
blocked: "skeleton-layer-surface"
---

# Testing and CI

Filed 2026-09-29. Serves **daily-drive** (a bar people depend on) and the
engineering bar in `CLAUDE.md`: tests land with each piece, not batched.
Modeled on scootbg (`crates/scootbg/tests`, `crates/scootbg/fuzz`,
`scripts/scootbg-bench`, the `scootbg` path filter in `.github/workflows/ci.yml`).

## Layers

- **Pure drawing** (no Wayland): snapshot tests of canvas output for rects,
  rounded rects, glyph runs at 1x and a fractional scale.
- **Module harness**: fake events in, assert on the `View` and the returned
  `Action`; every module ships tests through it.
- **Integration** against real compositors: headless scoot (screenshot over
  IPC, pixel assertions, hotplug with `--outputs N`) and headless sway, as
  scootbg's suite does. Tests skip without a compositor binary unless
  `SCOOTBAR_REQUIRE_SCOOT=1` makes that a failure.
- **Fuzz targets**: the config parser, the `exec`/`push` JSON, the clock format
  string, the TZif reader, and the D-Bus message parser if hand-rolled. Regression
  inputs kept in-tree, as scootbg's `regressions/`.

## CI

A `scootbar` filter in the workflow's classify job so a bar-only PR skips
unrelated jobs; `cargo nextest run` and `cargo test` for the package
(`CLAUDE.md` explains why both); clippy `-D warnings` and `fmt --check`; the
Cargo-feature matrix (default, `--no-default-features`, each module alone) so
the smallest build stays buildable; an `ldd` assertion that the default build
links only libc, libm and libgcc_s (no libEGL or libgbm); the macOS `cargo
check` of the stub.

## Benchmark

`scripts/scootbar-bench`, reusing `scripts/scootbg-bench`'s Python runner
(procs, sizing, report), for the [release gate](lightest.md).

## Done when

A bar-only PR runs exactly the jobs it needs, every module has harness tests,
and the four fuzz targets run for a fixed budget without findings.
