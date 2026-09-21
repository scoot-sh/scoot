---
title: "Docs gaps found converting a real NixOS config to the flake"
status: "resolved"
area: "meta"
priority: "medium"
blocked: null
---

# Docs gaps found converting a real NixOS config to the flake — DONE

Filed as gh issue #178 (six numbered parts from actually doing the
conversion). Docs-only: `CHANGELOG.md` + `docs/nix.md`, no code, no wire,
no behavior change.

## Per-part disposition

1. **CHANGELOG rename entry, session-identity half — CHANGED.** The
   2026-09-18 entry now carries the three strings plus the repo move (part
   2, same edit). Audit correction to the ticket's framing: none of the
   three "Was" values ever existed in-tree — verified with `git log -S`
   (each hits only the ticket-filing commit `2b14661`): pre-rename scoot
   exported no `XDG_CURRENT_DESKTOP` at all (`session_env.rs` was born
   post-rename in `ce69ffe`, unconditionally `scoot`), and
   `DesktopNames=`/`-portals.conf` were born post-rename with the new name
   (`ee5b246`, `ce69ffe`). So the entry frames them as what a converter's
   *own* files named, not as tree history. The "Is" halves are sourced:
   `crates/scoot/src/compositor/session_env.rs:59` (`DESKTOP_NAME =
   "scoot"`), `nix/modules/nixos.nix:38` (`DesktopNames=scoot`),
   `resources/scoot-portals.conf`.
2. **Repo move — CHANGED (CHANGELOG only).** `yackey-labs/flexwm` →
   `scoot-sh/scoot` is now in the 2026-09-18 entry, sourced from the
   resolved rename record (`rename-flex-family-done.md:51`) with the
   redirect-makes-stale-origin-silently-work warning from its §1. No README
   or `docs/nix.md` line: both already reference only the new URL (README
   Install snippet, `nix.md` consumer snippet) — verified zero
   `yackey-labs` hits outside the historical records, so there was nothing
   proportionate to add.
3. **`docs/nix.md` migration section — CHANGED.** New "Migrating from a
   hand-rolled packaging" section with the ticket's three renames. Every
   claim sourced: old binary name from the pre-rename flake (`pname =
   "flexwm"`, renamed in `7171e9e`); the no-existence-check proven by eval
   (`"${d}/bin/flexwm"` interpolates to a store path for a derivation that
   was never built); the fail-safe-looks-like-success warning cites the
   loader rule it rests on (`docs/configuration.md#failure-semantics`: no
   file at the default path is silent); the module split cites
   `flake.nix`'s `homeModules`/`nixosModules` wiring and the HM module's
   wholesale `xdg.configFile` ownership (`nix/modules/home.nix:169`).
4. **Packaged-renderer honesty — CHANGED (reference note only).** The
   ticket's premise is stale: filed pre-#198, when the flake package
   panicked on `gles` and built no scanout tier. Post-#198 (PR #198, gh
   #177) `packages.scoot` force-links libEGL (RUNPATH, host Mesa ICDs) and
   `packages.scoot-gpu` carries `gpu-scanout` — and `docs/nix.md`'s "GPU
   tiers from the flake" section already says exactly that (verified
   against `flake.nix:124-170` and `:257-301`), so the section itself is
   untouched. What was genuinely missing: the live-defaults reference
   showed `backend = "pixman"` and `[tty] gpu` with no pointer to that
   section. Added a two-key note there (gles needs OS drivers + loud
   startup error without them; scanout tier needs `scoot-gpu`; `[tty] gpu`
   names a host device path and is packaging-independent).
5. **End-to-end session example — CHANGED.** Worked example in the NixOS
   module section using the NEW surface (`session.command` +
   `sessionScript`, per #171) — not the hand-rolled `sessionPackages`
   workaround the issue had to keep. Verified by eval, not by booting: a
   scratch expression through the same `evalModules` harness as
   `nix/tests.nix` renders the example's exact settings — files
   `scoot/config.toml` + `scoot/session.sh` (executable, shebang + body) +
   portals file, `session.command` verbatim with the absolute script path,
   `providedSessions = [ "scoot" ]`, all module assertions holding. Booting
   a full session is out of scope for a docs PR (stated in the ticket).
6. **Smaller — ONE ALREADY-RIGHT, ONE CHANGED.** (a) The roadmap row: the
   issue's "row 6 says planned" is stale — PR #198 (`1418f01`) already
   fixed `docs/roadmap/README.md` row 6 to "done on paper" (verified in
   the diff). No double-fix. (b) `scoot msg reload` one-liner in the
   home-manager section: added, request name verified against the IPC
   surface (`scootctl` parses `reload` → `Request::Reload`,
   `scoot msg` aliases through the same parser; both spellings documented
   in `docs/configuration.md#reloading-the-config`).

## Verification

- Example eval: scratch `/tmp/e2e-verify.nix` (same-module `evalModules`,
  not part of the repo), `nix eval --impure --file` — output shows the
  three files, executable script text, verbatim command,
  `providedSessions = [ "scoot" ]`, assertions green.
- No-existence-check: `nix eval --impure --expr` interpolating
  `"${d}/bin/flexwm"` for an unbuilt derivation prints the store path.
- Existing gate unbroken: `nix build
  '.#checks.aarch64-darwin.scoot-modules'` exit 0 on this tree (no `.nix`
  touched — docs-only, confirmed by `git status`).
- Links: every `## ` header in `docs/nix.md` has its TOC anchor and vice
  versa, including the new section; the three new cross-doc links
  (`configuration.md#failure-semantics`,
  `configuration.md#reloading-the-config`, `#gpu-tiers-from-the-flake`)
  target headers that exist.
- Skill applied (`.claude/skills/update-docs/SKILL.md`): one audience per
  addition (the converting Nix consumer), no defensive constructions, no
  measurements in the user doc (numbers live here and in the PR body),
  additions only so nothing documented was relocated or lost.
