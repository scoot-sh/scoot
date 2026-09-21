---
title: "CI never exercises the Nix packaging (no `nix flake check`, no `nix build`)"
status: "open"
area: "packaging"
priority: "medium"
blocked: null
---

# CI never exercises the Nix packaging (no `nix flake check`, no `nix build`)

Filed as gh issue #173 (read it — the three unguarded surfaces and the
concrete suggestion; this entry tracks it).

`.github/workflows/ci.yml` runs everything *through* the flake (`nix
develop --command ...`) but never exercises the flake's own outputs:
`packages.<system>.scoot`/`.scootctl` (derivation, buildInputs,
`cargoLock.outputHashes`, fileset `src` scoping), `nixosModules.*` /
`homeManagerModules.*` (eval errors invisible), `checks.*.scoot-modules`
(`nix/tests.nix` guards only when run by hand).

Suggested (from the issue, verify costs before committing to the shape):
`nix flake check -L` in the Linux job after the dev shell is warm
(~instant on a warm store; eval + a python tomllib script). `nix build
.#scoot` is the fuller guard (the artifact users install) but pays a
full Smithay build the cargo cache can't help with — judgment call
whether that belongs on every PR or only on `main`; take the cheap half
regardless, decide the full build explicitly and record why.

Scope: workflow only (plus whatever the check half needs — no
compositor/client code expected). Prove it guards: a change that breaks
`nix flake check` while `cargo build` stays green must go red (the issue
names the class; construct one fixture or cite a past instance).
