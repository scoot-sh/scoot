---
title: "install.sh follow-ups from the #565 re-review"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-29"
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

## Resolution

Items 2 and 3 were already fixed on main when this landed (`pkg_for` maps `foot` in every branch; `has_tag` matches with `grep -F`), so they needed no change. Item 1: the five remaining SC2015 sites in `test-install.sh` are brace-grouped, and the file is clean on shellcheck 0.9.0 and 0.11.0. Item 4: the refusal now says an install of the script still running at the prefix can cause it (no marker file); the gate asserts the hint. Coverage gap: a new case runs the installer on a PATH with no `systemctl` and asserts the "no systemd user manager" note.
