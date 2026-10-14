---
title: "install.sh follow-ups from the #565 re-review"
status: "open"
area: "core"
priority: "low"
blocked: null
---

# install.sh follow-ups from the #565 re-review

Filed 2026-10-12. Serves **daily-drive**: the any-Linux install path. None of these is a wrong-binary or data-loss path; each fails safe.

## The gap

An independent re-review of PR #565 (APPROVE, no blockers) left four small items in `site/public/install.sh` and `site/scripts/test-install.sh`:

1. `test-install.sh` lines ~112, 133, 152 use `A && B || fail`, which shellcheck 0.9.0 flags (SC2015), so the suite's own shellcheck gate is red on that version. CI's pinned shellcheck (0.11-era) is clean. Make the gate version-independent (`-S warning` or targeted disables).
2. `pkg_for`'s default branch has no `foot` mapping, so an unknown distro (Void, Gentoo) prints `install with your package manager: ` with no package.
3. An explicit `--version` / `--bar-version` is interpolated into `grep` unescaped, so `--version '.*'` matches. Fail-closed (a 404 later, never a wrong binary). Use `grep -F`.
4. Two concurrent installs at one prefix: the manifest is written only in the EXIT trap, so the second run can refuse with "was not installed by this script" while the first is mid-install. Self-healing on retry; the message misleads. Either write a lock/in-progress marker or say "another install may be running".

Also a coverage gap: the fake `systemctl` is always present, so the "no user manager" note branch is never exercised.

## What to do

One small PR: fix 1-3, decide on 4 (a marker file is enough), add the missing test branch. Re-run `cd site && pnpm verify` and `test-install.sh` in an `ubuntu:24.04` and a `fedora:latest` container.

## Not in this ticket

Checksums for the session, unit and look files (same TLS trust root as `SHA256SUMS`, so no gain), and `.deb`/`.rpm` on the tagged releases.
