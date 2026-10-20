---
title: "packaging: deb/rpm/arch workflows fail on main (spec version lag, deb smoke fonts)"
status: "open"
area: "scootbar"
priority: "medium"
blocked: null
---

# packaging: deb, rpm and arch workflows fail on main

Filed 2026-10-19 from the independent review of PR #682 (dispatched on its branch; the same failures reproduce on main). Serves **daily-drive**.

## What to do

- **Version lag:** the workspace is 0.3.1 but all four rpm specs and all eight PKGBUILDs say 0.2.1, so `rpm.yml` and `arch.yml` fail their version gate before building. Bump them in the release PR (`packaging/rpm/README.md` documents it); the v0.3.2 release PR does this.
- **deb smoke:** `deb.yml` builds, lints and installs all four packages and runs `--check` on the bar and launcher, then fails in the shared smoke test applying the `vinyl-sunset` look, which needs Nerd fonts that bookworm-slim plus `fonts-dejavu-core` lack. Install the fonts in the proof containers or gate the look stage on their presence.
- Then dispatch all three workflows and record a green run per package in this entry.

## Docs (same PR)

No user-visible docs change; say so in the PR.
