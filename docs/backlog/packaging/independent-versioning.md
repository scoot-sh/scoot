---
title: "Independent versions per shipped binary, with the coupling between them made explicit"
status: "open"
area: "packaging"
priority: "medium"
blocked: null
---

# Independent versioning

Filed 2026-09-29. Serves **daily-drive**: the bar, the launcher and the
notification daemon must be able to release without a compositor release, and
the other way round.

## Where things stand

- One `[workspace.package] version = "0.1.0"` inherited by every crate. The
  flake reads that single value for every package (`flake.nix`, `version =
  ... .workspace.package.version`).
- No git tags exist. `scoot --version` and `scootctl --version` print the
  version plus the IPC protocol number (`cli-version-flag-done.md`).
- `scoot-ipc`'s protocol number already versions the one wire contract.

## What to build

- **Shipped binaries version on their own**: `scoot`, `scootctl`, `scootbg`,
  `scootbar`, later `scootnotify` and `scootlaunch`. Library crates
  (`scoot-core`, `scoot-ipc`, `scootbg-mem`, `scootui`) are internal: version
  with their consumer or set `publish = false`.
- **Make the couplings explicit contracts**, because independent versions make
  them visible:
  - `scoot` and `scootctl`: the IPC protocol number (exists).
  - `scoot` and `scootbg`: scoot runs `scootbg apply-config` with its
    `[wallpaper]` section, and the flake builds the two from one revision for
    that reason. Decide the compatibility statement (a minimum version, or a
    small handshake in `apply-config`) and check what a mismatch does today
    before promising anything.
  - `scootbar` and scoot: standard protocols only, plus an optional IPC
    feature guarded by the IPC protocol number.
- **Decide the split**: fully independent for the shell components (they speak
  standard protocols), and either independent or lockstep for the tightly coupled
  trio `scoot`/`scootctl`/`scootbg`. Recommend lockstep for that trio at first;
  the shell components are where independence pays.
- **What SemVer means for a binary**: name the public interface (CLI flags,
  config schema, control-socket protocol) and what breaks it, e.g. removing a
  config key is a major change. Consider a `version` key in the config schema.
- **Input is the commit history**: `CLAUDE.md` requires Conventional Commits with the
  package(s) as the scope, so the bump per package is computed from the commits
  that name it (`feat` minor, `fix` patch, `!` or `BREAKING CHANGE:` major). The
  script that bumps and tags reads that; commits with no package scope
  (`ci`, `docs`, `nix`, `backlog`) never bump a version.
- **Mechanics**: per-crate `version`, tags `<package>-vX.Y.Z`, a per-package
  changelog, and a small script (like `scripts/backlog`) that bumps, tags and
  checks. Prefer that over adopting a release tool until it is clearly needed.
- **Nix**: the flake reads each package's version from that crate's
  `Cargo.toml`, not the workspace; `nix/tests.nix` and the `--version` lines in
  `.github/workflows/nix-build.yml` follow.
- **CI**: the existing path classification decides which packages changed, so a
  bar-only change does not imply a compositor release.

## Done when

Each binary reports its own version, a tag names exactly one package, the flake
builds each with its own version, and the compatibility statement per pair is
documented and enforced where a mismatch would misbehave.
