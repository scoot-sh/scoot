---
title: "packaging: deb/rpm/arch workflows fail on main (spec version lag, deb smoke fonts)"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-20"
---

# packaging: deb, rpm and arch workflows fail on main

Filed 2026-10-19 from the independent review of PR #682 (dispatched on its branch; the same failures reproduce on main). Serves **daily-drive**.

## What to do

- **Version lag:** the workspace is 0.3.1 but all four rpm specs and all eight PKGBUILDs say 0.2.1, so `rpm.yml` and `arch.yml` fail their version gate before building. Bump them in the release PR (`packaging/rpm/README.md` documents it); the v0.3.2 release PR does this.
- **deb smoke:** `deb.yml` builds, lints and installs all four packages and runs `--check` on the bar and launcher, then fails in the shared smoke test applying the `vinyl-sunset` look, which needs Nerd fonts that bookworm-slim plus `fonts-dejavu-core` lack. Install the fonts in the proof containers or gate the look stage on their presence.
- Then dispatch all three workflows and record a green run per package in this entry.

## Green runs (PR #696, head 382e29af)

- arch: https://github.com/scoot-sh/scoot/actions/runs/39383628499 (success)
- rpm: https://github.com/scoot-sh/scoot/actions/runs/39383631150 (success; remaining warnings are the accepted set: 5x no-manual-page-for-binary plus no-documentation on scootbg/scootlaunch)
- deb: https://github.com/scoot-sh/scoot/actions/runs/39383633489 (success; look stage skipped via SMOKE_SKIP_LOOKS on both arches)

Fixes, all in PR #696: Arch scoot/scoot-git PKGBUILDs pin bfd (`-C link-arg=-fuse-ld=bfd`) because Rust 1.99's default lld leaves the static libgbm stub unpulled; mesa stays an optdepend (no DT_NEEDED, so no depends change). scootlaunch %description reworded off bare 'systemd' ('service scope'), no new rpmlint filter. Smoke looks gated behind SMOKE_SKIP_LOOKS in the installed-binary proofs (no distro package provides the Nerd families; /usr/bin/fc-match shadows the stub).

## Docs (same PR)

No user-visible docs change; say so in the PR.
