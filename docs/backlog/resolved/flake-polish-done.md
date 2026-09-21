---
title: "Flake polish: homeModules alias, Darwin default package, nixfmt drift — RESOLVED"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Flake polish: homeModules alias, Darwin default package, nixfmt drift — RESOLVED

## What it said

Filed as gh issue #175. Four small, independent items found while
reviewing the flake (each a one- or two-line change, filed together):

1. No `homeModules` alias: home-manager's current output name is
   `homeModules.*`, `homeManagerModules.*` is legacy. Fix: one line,
   `homeModules = self.homeManagerModules;`, keeping both working
   (check the reverse-compat direction — which spelling consumers pin).
2. Darwin home-manager default installs a binary not named `scoot`: the
   wrapper defaults `package` to per-system `.default` (`scootctl` on
   `aarch64-darwin`) while the option reads "The scoot package to
   install". If the macOS module use is config-management (per
   `docs/nix.md`), `package = null` is the honest Darwin default.
   Decide + record.
3. `vm/compositor-deps.nix` isn't `nix fmt`-clean (cosmetic
   `pkgs:\n[` → `pkgs: [`), so `nix fmt` rewrites a tracked file on a
   clean checkout. Format it — and check whether anything in CI pins
   formatting (see #173; decide whether the pin belongs here or there).
4. `nixfmt-rfc-style` is now an alias of `pkgs.nixfmt` (evaluation
   warning). Switch the flake's `formatter` (and any other reference)
   to the un-aliased name.

## Resolution

All four fixed together, verified each live (all commands Mac-side in
`/Users/steveyackey/code/flexwm`, Determinate Nix 3.22.3 / 2.35.2,
uncommitted working tree of branch `backlog/flake-polish`).

### 1. `homeModules = self.homeManagerModules;`

Reverse-compat direction, as the ticket asked: existing consumers pin
the *legacy* spelling in their own flake inputs
(`inputs.scoot.homeManagerModules.scoot`), while home-manager's docs
and `nix flake show`'s schema know the *current* spelling. So the alias
points new → old: the canonical `homeModules.*` resolves to the same
set the legacy name always gave, and nothing already pinned moves.

- Before: `nix flake show` had no `homeModules` section at all (a
  consumer following home-manager docs gets "flake does not provide
  attribute `homeModules`"); this nix tags the legacy name
  `unknown flake output` in show and lists it under
  `The following flake outputs are unchecked` in
  `nix flake check --no-build`.
- After: `nix flake show` renders a `homeModules` section with both
  children typed `Home Manager module`; `nix flake check` reports
  `✅ homeModules.scoot`; the legacy path evaluates byte-identically
  (Darwin probe below returns the same JSON through either spelling).
- Residual, by design: the `unknown`/`unchecked` notice still names the
  *legacy* spelling only. It cannot be silenced while keeping the alias
  (it is nix's nudge toward the current spelling), and removing the
  legacy name would break existing consumers — the exact tradeoff the
  ticket asked to check. So the "warning is gone" claim is scoped: the
  current spelling is warning-free; the legacy one keeps nix's nudge.

### 2. Darwin HM default is now `null` (files-only)

The ticket's reasoning held on inspection: `docs/nix.md` frames the
macOS use as config management ("the config you deploy to a Linux box
next to the machine that edits it"), the pure module already supports
files-only (`default = null`, "asserts nothing about the package"),
and silently installing a binary named differently than the option
description says is exactly the kind of surprise this project avoids.
The NixOS wrapper is untouched (NixOS is Linux-only; its default stays
the compositor).

```nix
programs.scoot.package = nixpkgs.lib.mkDefault (
  if pkgs.stdenv.hostPlatform.isDarwin then null else self.packages.${pkgs.system}.default
);
```

(`mkDefault null` is well-defined here: the pure module's own default
is also null, and an explicit user setting — priority 100 over
`mkDefault`'s 1000 — still wins, proven below.)

Darwin probe (hermetic `lib.evalModules` of the flake wrapper for
`aarch64-darwin`, `enable = true`, before → after):

- Before: `{"files":["scoot/config.toml","xdg-desktop-portal/scoot-portals.conf"],"installed":["scootctl"],"package":"scootctl"}`
- After (`homeModules.scoot`): `{"files":["scoot/config.toml","xdg-desktop-portal/scoot-portals.conf"],"installed":[],"package":null}`
- Same JSON through legacy `homeManagerModules.scoot` (alias works).
- Linux unchanged (`homeModules.scoot` on `x86_64-linux`):
  `{"installed":["scoot"],"package":"scoot"}`.
- Explicit override still wins on Darwin
  (`package = <flake scootctl>`): `{"installed":["scootctl"],"package":"scootctl"}`.

`docs/nix.md` updated to match: the example imports
`homeModules.scoot` (legacy spelling noted as still resolving), the
`package` row reads "flake's own build (Linux), `null` (macOS)", and
the snippet shows the Darwin `scootctl` override. The
`nix/modules/home.nix` wrapper comment names both spellings plus the
Darwin-null rule.

### 3. `vm/compositor-deps.nix` formatted; pin question answered

- Before: `<formatter>/bin/nixfmt --check vm/compositor-deps.nix` →
  `vm/compositor-deps.nix: not formatted` (exit 1).
- After: exactly the cosmetic diff the issue predicted
  (`pkgs:\n[` → `pkgs: [`), and `nixfmt --check` over all seven
  tracked `.nix` files (`flake.nix`, both modules, `nix/tests.nix`,
  both `vm/` files) reports clean — including the edited `flake.nix`,
  whose first draft nixfmt rewrote to a one-line `if/then/else`
  (taken as-is).
- No-op proof, scoped honestly: bare `nix fmt` in this tree errors
  with `unexpected end of input / expecting expression` (nixfmt reading
  empty stdin — no file list passed), a pre-existing quirk already
  recorded in `flake-consumer-and-home-manager-done.md` and
  `hm-session-script-path-done.md`, untouched by this change
  (`git status` confirms it modified no files). The equivalent proof
  that *is* runnable — the formatter binary over every tracked `.nix`
  file — is clean.
- Pin: nothing in CI pins Nix formatting (`.github/workflows/ci.yml`
  has only `cargo fmt --all --check`, lines 119–121). The pin belongs
  with #173 (`ci-nix-packaging.md`, still open — a workflow change is
  explicitly out of scope here), not with this ticket. Drift risk
  stands, noted there, not fixed by scope creep.

### 4. `formatter = forEach (pkgs: pkgs.nixfmt);`

No other `nixfmt-rfc-style` references in code (remaining hits are
this record's history and the flake-polish ticket text itself).

- Before: `nix build ".#formatter.aarch64-darwin"` →
  `evaluation warning: nixfmt-rfc-style is now the same as pkgs.nixfmt which should be used instead.`
- After: same build prints only the uncommitted-tree notice; the
  evaluation warning is gone. `cmp` of the two store binaries
  (`nixfmt-rfc-style` pre-fix vs `nixfmt` post-fix) reports identical —
  the alias was pure, so formatting behavior cannot have changed
  underneath (consistent with item 3's check staying green).

## Verification record

- `nix flake check` (full, Mac): `✅ homeModules.scoot`,
  `✅ checks.aarch64-darwin.scoot-modules`, all outputs green. Only
  warnings: uncommitted tree (pre-commit working state),
  incompatible-systems omission (aarch64-linux/x86_64-linux cannot
  evaluate on Darwin hardware), legacy `homeManagerModules` unchecked
  (item 1 residual, by design). Linux-side check coverage: the
  system-dependent branch (item 2's Linux default) is eval-only and was
  evaluated for `x86_64-linux` nixpkgs on this Mac (probe above);
  per-system builds remain CI's job under #173.
- `nix build ".#checks.aarch64-darwin.scoot-modules" --rebuild`: all 8
  content checks pass against the edited tree (empty-minimal,
  binds round-trip, relocated config + script pairing, portals,
  shebang/exe, .desktop, wrong-type renders).
- Zero `.rs` touched (`git status`: `flake.nix`,
  `nix/modules/home.nix`, `vm/compositor-deps.nix`, `docs/nix.md`,
  this move). Cargo suite not applicable — stated, not skipped.
- Benchmark: n/a — eval-time Nix change; nothing runs per-event or
  per-frame. The flake's own comment already records this
  ("no benchmark applies ... it runs once per `nix flake check`").

## Left out (with why)

- Format pin in CI (#173) and session-command option (#171): separate
  open tickets, explicitly out of scope; item 3's pin analysis is
  recorded above instead of implemented.
- `gpu-scanout` package (#177) and any CI workflow change: untouched.
- Bare-`nix fmt` empty-stdin quirk: pre-existing, documented in two
  earlier records, not this ticket's complaint (item 3 is about a
  tracked file being dirty, which is fixed).
