---
title: "Drop compatibility aliases for scoot's own old names"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-06"
---

# Drop compatibility aliases for scoot's own old names

Filed 2026-10-06. Serves **daily-drive completeness** (a clean flake surface
with one spelling per output, so the documented install configs cannot rot
into two) and secondarily **computer use** (agents reading the flake get one
canonical import path).

## The gap

Standing rule `sole-user-no-compat`: the maintainer is scoot's only user, so
breaking changes ship clean, with no aliases, deprecation periods or
compatibility shims. The legacy flake output `homeManagerModules.scoot`,
kept as the defining attr behind the `homeModules.scoot` alias
(`flake.nix:805-848`), violates it. The maintainer's own config already
imports `scoot.homeModules.scoot` (`~/nixos-config` commit `c3da963`).

## What to do

Swap the lines so `homeModules` is the defining output and delete
`homeManagerModules` outright (breaking, `!` + `BREAKING CHANGE:` footer).
Update `nix/modules/home.nix:178`, `nix/tests.nix:916-920` (`hmDocsDesktop`),
and the site's `desktop/index.md:178` "legacy spelling" note. Sweep for the
rest (renamed options, old config keys, CLI/env aliases, old IPC spellings)
and list every candidate kept or removed with reasons. Pin the removal with
a test showing the old name now fails.

## Not in this ticket

Protocol-level backward compatibility with other software (old Wayland
versions, legacy DRM, X11/XWayland) and history records (`docs/roadmap/`,
resolved backlog, past `CHANGELOG.md` entries).

## Resolution (PR #475, 2026-10-06)

`homeManagerModules` was the only live shim and is gone: `homeModules` is
the defining output in `flake.nix`, the both-spellings test pin is now an
absence pin (`!(flake ? homeManagerModules)`), the site note and module
comment are updated, and CHANGELOG Unreleased lists the removal. Kept
deliberately: `desktop.greeter` alias (current passthrough), nixpkgs regreet
aliases (third-party), `scoot msg` (current client), scootbar serde renames
(canonical names), all history records. No downstream uses the old name
(webtop, scoot-iso main + PR #1, ~/nixos-config all clear), so no
downstream PRs. Evidence: `nix eval .#homeManagerModules` errors;
`checks.aarch64-darwin.scoot-modules`, `docs-site`, `nix fmt --check` green.
