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

## Docs (same PR)

No user-visible docs change; say so in the PR.

## Done (2026-10-20, PR #693)

All three green on the `ci/release-hardening` branch (versions synced
to 0.3.1 by `scripts/sync-packaging-versions` itself):

- deb (amd64 + arm64): https://github.com/scoot-sh/scoot/actions/runs/39380985545
- rpm (Fedora 43 + openSUSE Tumbleweed): https://github.com/scoot-sh/scoot/actions/runs/39380988577
- arch (all eight PKGBUILDs): https://github.com/scoot-sh/scoot/actions/runs/39396791467

Beyond the ticket's two items, the gate surfaced and the PR fixes:
Nerd fonts installed in all proof containers (the ticket's first
option); the looks stage gated off in deb/rpm proof containers
(`SMOKE_SKIP_LOOKS`: bookworm foot 1.13 and Fedora foot both refuse
the rendered `initial-color-theme`); a `rpmlint` spelling error on
`systemd` in the new scootlaunch description reworded; and a
pre-existing Arch link failure (`undefined symbol: gbm_*`, red since
#588) fixed by linking the stub object by path plus `!lto` in the
PKGBUILDs (makepkg's `-flto=auto` bytecode + lld + liblto_plugin).
