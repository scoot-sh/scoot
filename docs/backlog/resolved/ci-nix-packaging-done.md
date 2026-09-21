---
title: "CI never exercises the Nix packaging — RESOLVED"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# CI never exercises the Nix packaging — RESOLVED

## What it said

Filed as gh issue #173. `.github/workflows/ci.yml` ran every step
*through* the flake (`nix develop --command ...`) but never exercised the
flake's own outputs: `packages.<system>.scoot`/`.scootctl` (derivation,
`buildInputs`, `cargoLock.outputHashes`, fileset `src` scoping),
`nixosModules.*` / `homeManagerModules.*` (eval errors invisible),
`checks.*.scoot-modules` (`nix/tests.nix` guarded only when run by hand).

Suggested: `nix flake check -L` in the Linux job after the dev shell is
warm (cheap on a warm store); `nix build .#scoot` fuller but pays a full
Smithay build — judgment call whether every-PR or main-only.

## Resolution

Three workflow changes, zero `.rs`, zero packaging changes:

1. **Cheap half taken unconditionally, both jobs** (`ci.yml`): `nix flake
   check -L` in the Linux job directly after `cargo fmt` (the first step
   that realises the dev shell, so the store is warm; placed before the
   Smithay builds so a broken package fails fast), and the same command in
   the macOS job after `cargo check`. The macOS half is load-bearing, not
   symmetry: `nix flake check` omits incompatible systems per runner, so
   the Linux job never evaluates `packages.aarch64-darwin.*`, the Darwin
   `apps`, or the Darwin module checks (verified in the logs on both
   sides). No new runner prerequisites: both jobs already install nix via
   `nix-installer-action` and already require flakes for `nix develop`.
2. **Full build is main-only, separate workflow** (`nix-build.yml`, `on:
   push: branches: [main]` plus `workflow_dispatch`): `nix build .#scoot`
   and `.#scootctl` each with a deterministic `--out-link`, then
   `./result-*/bin/scoot{,ctl} --version` (both print with no session, so
   the smoke needs no seat/socket/GPU). Separate file rather than a gated
   job in `ci.yml` so the PR signal stays clean and the cost split is
   visible. Not every-PR: measured **5m10s wall cold** for both packages
   on the 4-vCPU dev VM (same core count as the runners — representative,
   not a lower bound), with zero reuse possible (the deliberate
   no-store-cache decision documented in `ci.yml`; the cargo `target/`
   cache cannot help a store build), against a failure mode that only
   fires on packaging-adjacent changes — which the every-PR `flake check`
   already half-covers at eval. Not nightly: that leaves main red for up
   to a day, strictly worse than one run per merge. gpu-scanout excluded
   (gh #177's ticket); the Darwin packaged artifact stays local-only
   (macOS CI is check-only by decision).
3. **Format pin from #175 closed here** (`ci.yml`, Linux job only —
   formatting is system-independent, a second run would prove nothing
   twice): `git ls-files -z '*.nix' | xargs -0 nix fmt -- --check`.
   `git ls-files` so a new `.nix` file is covered with no listing to
   maintain; explicit paths because `nix fmt -- --check .` is a
   deprecated directory form and bare `nix fmt` hits the pre-existing
   empty-stdin quirk (`<stdin>:1:1`, recorded in two earlier tickets,
   still present, worked around rather than fixed). Resolves to the
   flake's own `formatter` (`pkgs.nixfmt`). All seven tracked `.nix`
   files check clean — no `compositor-deps.nix`-style residuals, so no
   fix-or-scope was needed.

Bug-bash sized to the change: the workflow has no `continue-on-error`
anywhere (verified by grep — count 0), so every new step fails its job;
no step gating or secret touches anything added.

## Verification record

All commands Mac-side in `/Users/steveyackey/code/flexwm` except where
noted "dev VM" (`ssh -p 2222 dev@localhost`, tree via the `/mnt/scoot` 9p
mount). Branch `backlog/ci-nix-packaging`, uncommitted working tree
unless noted.

- `git ls-files -z '*.nix' | xargs -0 nix fmt -- --check` (Mac,
  Determinate Nix 3.22.3): exit 0, 0.665s wall. The exact CI string.
- `nix flake check -L` (Mac): exit 0, `✅` on every Darwin output;
  `warning: ... omitted these incompatible systems: aarch64-linux,
  x86_64-linux`. Warm wall ≈0.5s (`time nix flake check --no-build`:
  0.527s wall).
- `nix flake check -L` (dev VM, aarch64-linux, Nix 2.34.8, cold store):
  exit 0, `all checks passed!`, 44.2s wall (≈3.4s CPU — the rest is
  first-run downloads; the checks derivation alone built in 34.3s wall).
  On CI the same downloads overlap the dev-shell realise, so the marginal
  cost sits beside the ~35s realise `ci.yml` already budgets.
- `nix build /mnt/scoot#scoot /mnt/scoot#scootctl --print-out-paths`
  (dev VM, cold store): exit 0, **5m10.380s wall**, both outputs realised
  (`scoot` 5658968 bytes, `scootctl` 529440 bytes — the latter byte-count
  matching gh #172's pre-fix `ls`, i.e. the split this guards). The
  workflow's exact `--out-link` + `--version` form re-run against the
  realised paths: exit 0, `scoot 0.1.0 (ipc protocol 3)` from both
  binaries. (The `Git tree ... is dirty` warnings in that log are the
  branch's own uncommitted workflow edits; CI trees are clean.)
- Cargo suite: untouched, none run — stated, not skipped (zero `.rs`,
  zero packaging files; `git status` at commit shows only the two
  workflow files, README, and this move).

### The guard fires (decision 4)

Fixture A — `package = lib.mkOption` → `lib.mkOptioTypo` in
`nix/modules/home.nix` (pure-module eval error; `cargo build -p scoot
-p scootctl` exit 0 throughout): `nix flake check -L` exit 1 on the Mac
(`error: attribute 'mkOptioTypo' missing ... Did you mean
mkOptionType?`) and exit 1 on the dev VM (same error via `/mnt/scoot`).
Reverted byte-identical; both sides green again.

Fixture B — `pkgs: [` → `pkgs:\n[` in `vm/compositor-deps.nix`
(unformatted file): the fmt CI string exit 1 naming exactly
`vm/compositor-deps.nix: not formatted`. Reverted; exit 0.

Negative finding, recorded so nobody re-proves it: breaking the flake
*wrapper* import (`imports = [ ./nix/modules/home.nix ]` →
`home-MISSING.nix` in `flake.nix`) does **not** go red under `nix flake
check` on Darwin — module-typed outputs are unchecked/lazy there
(`homeManagerModules` is even listed as `unknown flake output`), and the
`checks.*.scoot-modules` derivation that *does* run reads
`./modules/home.nix` directly, not through the wrapper. So the
check-guard covers the pure modules, the packages, `apps`, `devShells`,
`formatter` and the checks derivation — not wrapper-level wiring on a
foreign system. Fixture A is the honest proof; the wrapper gap is stated,
not hidden.

## Left out (with why)

- `gpu-scanout` package build (#177) and any compositor/client code:
  explicitly out of scope; expect zero non-workflow, non-doc files.
- Darwin packaged-artifact build: macOS CI stays check-only per the
  standing decision; `nix build` on a Mac remains the documented local
  path.
- Bare-`nix fmt` empty-stdin quirk: pre-existing, routed around with
  explicit paths, not fixed.
- No separate gh comment: the PR body carries `Fixes #173`.
