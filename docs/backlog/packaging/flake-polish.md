---
title: "Flake polish: homeModules alias, Darwin default package, nixfmt drift"
status: "open"
area: "packaging"
priority: "low"
blocked: null
---

# Flake polish: homeModules alias, Darwin default package, nixfmt drift

Filed as gh issue #175 (four independent one-or-two-line items, deliberately
filed together — read it; fix together, verify each).

1. **No `homeModules` alias.** home-manager's current output name is
   `homeModules.*`; `homeManagerModules.*` is legacy (`nix flake check`
   warns `unknown flake output 'homeManagerModules'`). One line keeps
   both: `homeModules = self.homeManagerModules;` (check the reverse-compat
   direction — which spelling do consumers pin? keep both working).
2. **Darwin home-manager default installs a binary not named `scoot`.**
   The wrapper defaults `package` to per-system `.default`, which on
   `aarch64-darwin` is `scootctl` — while the option reads "The scoot
   package to install". If the macOS module use is config-management
   (per `docs/nix.md`), `package = null` is the honest Darwin default
   (files-only setup is explicitly supported). Decide + record.
3. **`vm/compositor-deps.nix` isn't `nix fmt`-clean** (cosmetic
   `pkgs:\n[` → `pkgs: [`), so `nix fmt` rewrites a tracked file on a
   clean checkout. Format it — and check whether anything in CI pins
   formatting (see #173; if not, say whether this PR should add the
   pin or leave the drift risk standing).
4. **`nixfmt-rfc-style` is now an alias** (evaluation warning says it is
   `pkgs.nixfmt`). Switch the flake's `formatter` (and any other
   reference) to the un-aliased name — verify the warning is gone after.

Each item: verify live (`nix flake check` warning text before/after,
Darwin eval of the default, `nix fmt` clean-tree no-op, warning gone).
