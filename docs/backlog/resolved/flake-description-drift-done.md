---
title: "`flake.nix`'s top-level `description` and its package's `meta.description` are two independently hand-copied strings that can drift (NIT) — RESOLVED: per-system `meta.description` derived from the crate, top level names both"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# `flake.nix`'s top-level `description` and its package's `meta.description` are two independently hand-copied strings that can drift (NIT) — RESOLVED

~~`flake.nix`'s top-level `description` and its package's
`meta.description` are two independently hand-copied strings that can
drift (NIT). Found by `flexwm-reviewer` reviewing item 16. Also, on
Darwin, `meta.description` still advertises "a scrolling-tiling Wayland
compositor that runs without a GPU" when the Darwin build is actually
just the `flexwm msg` client (`README.md` explains this correctly, but
`nix search`/`nix flake show` metadata would not). Cheap to fix whenever
`flake.nix` is next touched for another reason (e.g. the `src` filesetting
above) — bundling it there avoids paying for a second full evaluation/
rebuild cycle just for a string.~~ — **RESOLVED 2026-09-18** (flake metadata
only; no Rust, no build-input, no derivation-output change).

## Fix

- Top-level `description` is now
  `flexwm: a scrolling-tiling Wayland compositor that runs without a GPU
  (on macOS, the remote-control client only)` — it names both, since it is
  shown without a system context.
- `meta.description` is now per system: on Linux it is read from
  `crates/flexwm/Cargo.toml`'s own `description` (the same
  `readFile`/`fromTOML` shape the package's `version` already uses, so the
  crate and the flake claim can't drift); on Darwin it is
  `Remote-control client (`flexwm msg`) for the flexwm scrolling-tiling
  Wayland compositor`, so `nix search`/`nix flake show` no longer advertise
  a compositor macOS never runs. "Compositor" wording kept throughout per
  the project rule.
- The two flake strings now differ *by design* (platform-covering vs
  per-system), so there is no longer one claim in two places to drift —
  except the top-level literal, which is irreducible (see constraints).

## Loader constraints found by verify-first (both re-derived, not assumed)

A fully single-sourced fix (one `let`-bound string for all three sites)
is impossible at the flake loader level, proven live:

1. `flake.nix` must be *syntactically* an attribute set — wrapping it in
   `let ... in { ... }` fails with `file '.../flake.nix' must be an
   attribute set` (minimal probe at `/tmp/flakeprobe`, same error as the
   real file).
2. `description` must be a string literal — a computed
   `"flexwm: " + (builtins.fromTOML ...) + " ..."` fails with
   `expected a string but got a thunk at .../flake.nix:9:3`.

So the top level stays one hand-written literal by loader fiat; everything
derivable (the Linux `meta.description`) is derived.

## Evidence (branch `fix/flake-description-drift`; all commands run against the
working tree as pushed — `flake.nix` byte-identical before and after the
record-keeping amend)

- `nix eval .#description` — fails: `description` is not an output
  attribute (correct query is `nix flake metadata`, below). Records the
  wrong first attempt, not a regression.
- `nix eval --system aarch64-linux .#packages.aarch64-linux.default.meta.description`
  → `"A scrolling-tiling Wayland compositor that runs without a GPU"`
  (byte-identical to `crates/flexwm/Cargo.toml`; the `--system` flag itself
  was ignored as untrusted, but the attribute path selects the Linux
  package set explicitly, so the `isDarwin` branch taken is still Linux's).
- `nix eval --system aarch64-darwin .#packages.aarch64-darwin.default.meta.description`
  → `"Remote-control client (`flexwm msg`) for the flexwm scrolling-tiling
  Wayland compositor"`.
- `nix flake metadata` → `Description: flexwm: a scrolling-tiling Wayland
  compositor that runs without a GPU (on macOS, the remote-control client
  only)`.
- `nix flake show` → exit 0 (current system; `--all-systems` would trip the
  known, separate `x86_64-darwin` evaluation failure, left to its own
  ticket).
- `nixfmt --check flake.nix` (via the flake's own `formatter` output) →
  clean; `cargo fmt --check -p flexwm` → clean.
- `cargo test -p flexwm` → 20 + 3 passed; `cargo clippy -p flexwm
  --all-targets -- -D warnings` → clean; `cargo nextest run --workspace` →
  125 passed, 0 failed (all on the macOS host; no Rust changed, run as the
  cheap baseline).
- `scripts/smoke-test.sh` skipped: nothing executable changed — flake
  metadata strings plus this bookkeeping. No README change: no user-facing
  behavior surface (metadata only; README's Building already states the
  Darwin split correctly).

## Scope guard

Sibling tickets `packaging/nix-src-fileset.md` and
`packaging/flake-x86-darwin-system.md` verified still open and untouched —
one ticket at a time. `nix build`/`nix flake check` (full build) not run:
a metadata-only change alters no derivation output, and `nix flake show`
plus per-system `meta.description` evaluation covers what changed.
