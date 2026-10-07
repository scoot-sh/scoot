---
title: "Independent versions per shipped binary, with the coupling between them made explicit"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-07"
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

## Resolved 2026-10-07 (PR #495)

What landed, item by item:

- **Per-crate `version`**: `scoot`, `scootctl`, `scootbg` and `scootbar`
  each pin their own `version = "0.1.0"` (the value the workspace
  carried, so nothing bumps yet); `scoot-core`, `scoot-ipc` and
  `scootbg-mem` are `publish = false`. There is no `scootui` crate and
  no standalone `scootctl` binary anymore (`scoot msg` is the only
  client): `scootctl` is the client library inside the trio, and the
  ticket's "shipped `scootctl`" is read as that library.
- **The split**: the trio `scoot`/`scootctl`/`scootbg` in lockstep at
  first (the ticket's recommendation), `scootbar` independent, enforced
  by `scripts/version check` in CI.
- **The couplings, measured against the code**: `scoot`/`scoot msg`
  stay on the IPC protocol number (printed beside the version);
  `scoot`/`scootbg` already handshake every `apply-config` run
  (`version` first: protocol mismatch refused, version mismatch warned,
  pre-handshake daemon refused with the fix -- `crates/scootbg/src/apply.rs`),
  so no new handshake was added and the rule is "same protocol required,
  version mismatch tolerated with a warning"; `scootbar`/scoot stay on
  standard protocols plus the IPC-guarded option. Strict section parsing
  (`deny_unknown_fields`) keeps a newer-scoot/older-scootbg skew loud
  (exit 2, session carries on) rather than silent.
- **`scoot --version` names scoot's own version**: `scootctl` grew
  `version_string_for`, and `scoot`'s `main` passes its own
  `env!("CARGO_PKG_VERSION")` (the old shared helper would have printed
  scootctl's version once the two diverge; pinned by tests on both
  sides).
- **`scripts/version`**: `plan` (dry run: bump per package plus the tags
  it would create, never creating anything), `check` (trio lockstep,
  semver, own versions, libs private, no tag ahead of its manifest),
  `changelog <package>`. Rules: `feat` minor, `fix`/`perf` patch,
  `!`/`BREAKING CHANGE:` major; pre-1.0 a major lands as minor;
  `refactor`/`test`/`build`/`chore`/`docs`/`ci` with a scope never
  bump; unscoped, non-package-scoped (`ci`, `docs`, `nix`, `backlog`,
  `claude`, `deps`) and merge subjects never bump; internal-library
  scopes fan out to their consumers like CI does. Tests in
  `scripts/test_version.py` (21 tests: fixture histories covering bumps,
  no-bump, breaking, merge/squash, unscoped, no prior tag, plus every
  `check` failure mode), run in CI beside the check.
- **Nix**: the flake reads each package's version from its own crate's
  `Cargo.toml` (evaluated: `scootbar` at a probe `0.9.9` resolves while
  `scoot` stays `0.1.0`); `nix-build.yml` asserts each packaged binary
  prints its crate's version. `nix/tests.nix` needed no change (it never
  read the workspace version).
- **CI**: one new `version` job (manifest check plus tool tests, python
  only) with its path filter in the same PR; manifest moves run it via
  first-match arms; nothing always-run. A bar-only change still implies
  no compositor release.
- **Docs**: contributor page `docs/versioning.md` (SemVer per binary,
  the measured couplings, the tool, the maintainer's release steps);
  user-facing `site/src/content/docs/reference/versions.md` (what each
  `--version` prints, which pairs work, symptom boxes), linked from the
  reference hub and sidebar. No `version` key was added to the config
  schema: the schema's strictness already makes skew loud, and a key
  nothing reads would be decoration.

Evidence: `scripts/test_version.py` 21/21, `scripts/test_backlog.py`
30/30, `scripts/version check` ok, `scripts/version plan` dry run
against the real history (trio and bar each `0.1.0 -> 0.2.0`, tags
`scoot-v0.2.0` etc., nothing created), `cargo test -p scootctl` 49/49,
`scoot` bin cli tests 32/32, `nix flake check` green on darwin,
`nix flake show --all-systems` evals, `nix build .#docs-site` green,
`scripts/backlog check` shows only the 3 known pre-existing problems.
Full-workspace cargo builds and the Linux suites ran in CI (this tree
never touched Linux hardware: the dev VM is down).

Open decisions for the maintainer: the first release is by hand
(`docs/versioning.md`, "Cutting a release"); `release-artifacts.md`
is unblocked by this resolve and designs the tag-triggered workflow.

Deliberate deviations from the ticket, each justified above: no
`scootui` (does not exist), no `scootctl` binary (library in the
trio), no new `apply-config` handshake (one already exists and was
measured sufficient), no config-schema `version` key (strictness
already covers it), no mutating release subcommand (the tool reads
only; the maintainer tags).
