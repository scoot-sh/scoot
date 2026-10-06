---
title: "Drop compatibility aliases for scoot's own old names"
status: "open"
area: "packaging"
priority: "medium"
blocked: null
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
