---
title: "The Nix package's `src = self` invalidates the whole build on any doc-only edit (LOW, non-blocking) — RESOLVED: `lib.fileset` scoped to Cargo.toml/Cargo.lock/crates"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# The Nix package's `src = self` invalidates the whole build on any doc-only edit (LOW, non-blocking) — RESOLVED

~~The Nix package's `src = self` invalidates the whole build on any
doc-only edit (LOW, non-blocking). Found by `flexwm-reviewer` reviewing
item 16: `src` is the whole flake tree — `CLAUDE.md`, `README.md`,
`ROADMAP.md`, `vm/`, `scripts/` included — none of which the compiler
reads, but a change to any of them still busts the derivation's cache and
forces a full ~3.5 minute rebuild. Demonstrated directly: appending one
newline to `README.md` changed the output path entirely. Since this repo
edits `ROADMAP.md` on essentially every PR, that's a real recurring cost
once this lands. Fix direction: a `lib.fileset` filter scoped to
`Cargo.toml`/`Cargo.lock`/`crates/` — but first confirm nothing the build
actually needs lives outside that set (a `build.rs`, an `include_str!` of
a root-level file, a license file read at build time). Two reads were
checked and are safe (`vm/compositor-deps.nix`'s import, and
`builtins.readFile ./Cargo.toml` for the version string, both of which
resolve against the flake tree rather than `src`), but that check should
be redone against whatever the tree looks like when this is picked up,
not assumed still true.~~ — **RESOLVED 2026-09-18** (`flake.nix` only;
no Rust, no build-input, no user-facing change).

## Fix

`src = self` is now a `pkgs.lib.fileset.toSource` scoped to exactly
`./Cargo.toml`, `./Cargo.lock`, `./crates` — the union the ticket named.
A comment at the site records why each outsider stays out (all four
`./` reads in the flake — `version`, `crateDescription`,
`cargoLock.lockFile`, `vm/compositor-deps.nix` — resolve at eval time
against the flake tree, not `src`).

## Verify-first (redone against this tree, not assumed)

- No `build.rs` outside `crates/` (`crates/flexwm/build.rs` only, inside
  the filter); no `include_str!`/`include_bytes!` of any root-level file
  (grep over `crates/` clean); no `.cargo/config`, no toolchain file;
  `license.workspace = "MIT"` is a string, not a file read.
- The ticket's two reads re-checked safe at this rev, plus the two it
  didn't name: `crateDescription`'s `readFile ./crates/flexwm/Cargo.toml`
  and `cargoLock.lockFile = ./Cargo.lock` are likewise eval-time flake-tree
  paths.
- The "too much" direction turned out worse than filed: the pre-fix `src`
  store path also contained `target/` (527 MB workdir, 1.0 GB in-store)
  and `.git` (41 MB) — untracked files riding along in the working-tree
  copy — for a `src` closure of **1,111,867,680 bytes**.

## Evidence (branch `fix/nix-src-fileset`)

- Before: `src` =
  `/nix/store/wv73ahwb49a5rsxwxqq0xqxjkdz1p19c-source` (closure
  1,111,867,680 bytes; `nix store ls` shows `target/`, `.git`, `docs/`,
  `vm/`, `scripts/` alongside `crates/`); `drvPath` =
  `/nix/store/8p0i8289ljxrh9krcyywc07nj8s3lppy-flexwm-0.1.0.drv`.
- Doc-edit invalidation reproduced pre-fix: one appended newline to
  `README.md` moved the drv to
  `/nix/store/bxm9dg9c6s13pxl2dzgdjqbv9y809y43-flexwm-0.1.0.drv`
  (reverted afterwards).
- After: `src` =
  `/nix/store/i8jpbr4wq85ld9xkfs3h3yk1slqvyjcs-source` — `nix store ls`
  shows exactly `Cargo.lock`, `Cargo.toml`, `crates/` — closure
  **3,325,216 bytes** (~334x smaller).
- Hash-stability proven post-fix: `README.md` newline + a docs edit + a
  `target/` touch together leave the drv at
  `/nix/store/sc1hg5rm72bx4yxni5l9bl7qx5bs434q-flexwm-0.1.0.drv`
  (all probes reverted afterwards; only `flake.nix` modified).
- `nix build .#packages.aarch64-darwin.default` → success:
  `/nix/store/374g32m5p2qrbnapa4f1y4in95hmi3hq-flexwm-0.1.0`; the binary
  prints the real `--help` surface and the honest Darwin `--headless`
  refusal (`the compositor only runs on Linux; flexwm msg works
  everywhere`).
- `nix flake show` → exit 0 (all four systems evaluate); `nix fmt
  -- --check flake.nix` → clean.
- `cargo test -p flexwm` → 20 + 3 passed; `cargo clippy -p flexwm
  --all-targets -- -D warnings` → clean; `cargo fmt --check -p flexwm` →
  clean (all on the macOS host; no Rust changed, run as the cheap
  baseline). `cargo nextest run --workspace` not run: no Rust changed and
  the full-suite pass is covered by the `nix build` proof for what this
  change touches. `scripts/smoke-test.sh` skipped: nothing
  executable changed — the built binary is byte-identical in content,
  only its `src` filter narrowed. No README change: no user-facing
  surface (`nix build`/`nix run` usage unchanged).
- Linux `nix build` not run: the one-system proof above is what the
  ticket asks for, and the Linux package differs only in `buildInputs`
  (same `src`). Stated, not papered over.

## Scope guard

Sibling ticket `packaging/flake-x86-darwin-system.md` verified still open
and untouched — one ticket at a time.
