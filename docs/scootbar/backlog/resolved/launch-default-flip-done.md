---
title: "scootlaunch: make it the desktop profile's default launcher and picker"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M7"
resolved: "2026-10-21"
---

# scootlaunch: flip the default

Filed 2026-10-12. Serves **daily-drive**. The
[shared criteria](../../../dev/design/shell-apps.md#nix-modules-and-the-desktop-profile-switch) apply, against
fuzzel's numbers. Needs the maintainer's ruling.

## Docs (same PR)

Part of this ticket's work, not a follow-up: update `site/src/content/docs/scootlaunch/index.md and README.md (What works / Not yet)` with the launcher as the default, how to opt out, and the breaking marker. Task-first, type/default/example/reload per option, `> **Symptom:**` boxes, snippet gates green (`site/OUTLINE.md`). A change with no user-visible effect says so in the PR instead.

## Resolution

Landed 2026-10-21 (`feat(nix)!: launcher defaults to scootlaunch`): daemon
default flipped, fuzzel one line away, all pickers flip with the daemon,
condition-1 breaks documented plus a "Migrating from fuzzel" section,
install/compose docs updated, tests re-pinned with escape-hatch and
no-overlay coverage. README unchanged (the compositor is untouched).
Follow-ups: [first-open outlier](../launch-first-open-outlier.md),
[xdpw chooser live proof](../launch-xdpw-chooser-live-proof.md).
