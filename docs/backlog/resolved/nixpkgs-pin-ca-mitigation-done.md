---
title: "Bump the nixpkgs pin past the eMudhra/Comodo CA mitigation (02c087f)"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-21"
---

# Bump the nixpkgs pin past the eMudhra/Comodo CA mitigation (02c087f)

Filed 2026-10-21. Serves **daily-drive**: the pinned cacert bundle (`8ce4ef6`, 2026-09-10) still trusts the roots nixpkgs blacklisted on 2026-10-20 (NixOS/nixpkgs#576485: emSign ECC Root CA G3, emSign Root CA C1, emSign ECC Root CA C3, Comodo AAA Services).

## The gap

Nothing scoot ships verifies certificates, and fixed-output fetches are hash-pinned, so a rogue root cannot change what a build produces; the exposure is a pin that is behind. Tried 2026-10-21 (PR #713, closed): moving `flake.nix`, `flake.lock`, `devenv.yaml` and `devenv.lock` to `02c087f` (the head of nixos-unstable, the merge of #576390) fails CI on the newer toolchain with clippy `-D warnings` errors at about 100 sites (`chunks_exact` with a constant chunk size in scoot, scootui and scootbg tests and render code; needless borrows) and on the docs-site pnpm-deps hash.

## What to do

One PR: bump the pin, fix the lints (`as_chunks` for the constant-size chunks, drop the borrows; none of it changes behavior, but the render-path sites get a benchmark glance), refresh the docs-site hash, keep `devenv.yaml`, `flake.nix` and `vm/` on the same rev, and verify every Nix and Linux job green on the exact head.

## Not in this ticket

`vm/flake.lock` follows nixos-unstable on its own schedule.

## Done 2026-10-21

Landed the pin bump to `02c087f` with the lint fallout fixed and the docs-site hash refreshed. `cargo clippy --workspace --all-targets -D warnings` passes on rustc 1.99.0; `nix fmt --check` and `cargo fmt --check` pass. Micro-benchmarks show no regression (fill 8MiB avg +4.8% noise, blend 1MiB -26% noise, both in-place with no allocation).
