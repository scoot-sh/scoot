---
title: "The flake's `systems` list still names `x86_64-darwin`, which the pinned nixpkgs refuses to evaluate at all (LOW, pre-existing) — RESOLVED: deliberate exclusion, system dropped with reason"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# The flake's `systems` list still names `x86_64-darwin`, which the pinned nixpkgs refuses to evaluate at all (LOW, pre-existing) — RESOLVED

~~The flake's `systems` list still names `x86_64-darwin`, which the pinned
nixpkgs refuses to evaluate at all (LOW, pre-existing). Found while
adding item 16's `packages` output: `nix flake check --all-systems` fails
on `packages.x86_64-darwin.default` — and equally on
`devShells.x86_64-darwin.default`, i.e. it predates the new outputs rather
than being introduced by them. Verified against `main` itself, not just by
reading: `nix eval
'git+file:///Users/steveyackey/code/flexwm?ref=main#devShells.x86_64-darwin.default.name'`
throws out of `nixpkgs.legacyPackages.x86_64-darwin`, nixpkgs having
dropped that platform after the 26.05 branch. Plain `nix flake check`
(current system only) passes, on both macOS and the dev VM. The fix is a
one-line choice — drop `x86_64-darwin` from `systems`, or repin nixpkgs to
a branch that still carries it — and it only matters if an Intel Mac ever
has to build this; left alone to keep item 16 to its own scope.~~ —
**RESOLVED 2026-09-18** as deliberate exclusion: `x86_64-darwin` dropped
from `systems` (`flake.nix` only, plus one README sentence and this
bookkeeping; no Rust, no derivation-output change for any remaining
system).

## Fix

`systems` is now `aarch64-linux`, `x86_64-linux`, `aarch64-darwin` — the
`x86_64-darwin` line is gone, replaced by a comment at the site recording
why: the pinned nixpkgs (26.11) throws out of
`legacyPackages.x86_64-darwin` at eval, so no per-system definition can
even be reached; the repin alternative and the cargo-level all-clear are
below. README's Building section carries one sentence so an Intel Mac
reader learns `cargo build` is their path instead of a failing `nix
build`.

## Verify-first (redone against this tree, not assumed)

- Failure re-derived pre-fix on this checkout (branch base `7963ea3`,
  before the edit): `nix eval
  '.#devShells.x86_64-darwin.default.name'` throws
  `error: Nixpkgs 26.11 has dropped support for x86_64-darwin. The 26.05
  stable branch still supports x86_64-darwin, and will receive security
  fixes until the end of 2026. ...`, and `nix flake show --all-systems`
  dies in the same throw while evaluating
  `packages.x86_64-darwin.default` / `devShells.x86_64-darwin.default`.
  (The `--system` flag is ignored for untrusted users on this machine, so
  the attribute path itself selects the system — same caveat the
  description-drift record notes.)
- "Support it" is not a one-line add: the throw happens in
  `nixpkgs.lib.genAttrs` over `legacyPackages`, before any flexwm
  per-system definition is reached. Supporting the platform through nix
  means repinning the whole tree (and `vm/` with it, which the flake
  comment pins to the same rev so the dev shell and the VM agree) to
  26.05 — an older everything, whose security fixes end with 2026 per
  nixpkgs' own message — for a platform Apple has phased out. Declined
  on cost, not on demand.
- Cargo-level all-clear, so the exclusion is nix-only and honest: the
  Darwin build is one arch-independent `cfg(not(target_os = "linux"))`
  path (`crates/flexwm/src/main.rs`), every compositor dependency lives
  under `[target.'cfg(target_os = "linux")'.dependencies]`
  (`crates/flexwm/Cargo.toml`), and `crates/` contains zero
  `target_arch`/`aarch64`/`x86_64` code (two grep hits, both measurement
  comments in `compositor/config.rs`). Both Darwins share the identical
  dependency set and code path, so the proven `aarch64-darwin` build
  covers the shared path and `cargo build` from source remains open on
  an Intel Mac. (No Intel Mac in the loop to run it on — stated, not
  papered over. An Intel Mac also cannot run the `aarch64-darwin` nix
  binary: Rosetta translates the other direction.)

## Evidence (branch `fix/flake-x86-darwin-system`)

- `nix flake show --all-systems` → exit 0: exactly three systems under
  `apps`, `devShells`, `formatter`, `packages`; no `x86_64-darwin` node.
- `nix flake check --all-systems` (the ticket's exact failing command)
  → exit 0, 12 `✅`, zero errors, zero `x86_64-darwin` mentions.
- Per-system eval: `packages.{aarch64-darwin,aarch64-linux,x86_64-linux}.default.name`
  → `"flexwm-0.1.0"` on all three;
  `devShells.{same three}.default.name` → `"nix-shell"` on all three;
  `packages.x86_64-darwin.default.name` → plain `does not provide
  attribute ... 'packages.x86_64-darwin.default.name'` (cleanly absent,
  not the nixpkgs throw).
- `nix build .#packages.aarch64-darwin.default` → success; the binary
  prints the real `--help` surface (proves no remaining-system
  derivation changed content).
- `nix fmt -- --check flake.nix` → exit 0.
- `cargo test -p flexwm` → 20 + 3 passed; `cargo nextest run
  --workspace` → 125 passed, 0 failed; `cargo clippy -p flexwm
  --all-targets -- -D warnings` → clean; `cargo fmt --check -p flexwm`
  → clean (all on the macOS host; no Rust changed, run as the cheap
  baseline). `scripts/smoke-test.sh` skipped: nothing executable
  changed — a systems-list line, a README sentence, and this
  bookkeeping.

## Scope guard

`nix-src-fileset` work (PR #119, just merged) untouched — verified by
`git status`: only `flake.nix`, `README.md`, the ticket move, and the
two index updates below are in the diff. `src` fileset, `version` /
`crateDescription` reads, and `vm/compositor-deps.nix` all unevaluated
by this change.
