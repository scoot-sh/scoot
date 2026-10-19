---
title: "scootlaunch: make it the desktop profile's default launcher and picker"
status: "open"
area: "scootbar"
priority: "low"
blocked: "launch-packaging, launch-agent-json"
milestone: "M7"
---

# scootlaunch: flip the default

Filed 2026-10-12. Serves **daily-drive**. The
[shared criteria](../../../dev/design/shell-apps.md#nix-modules-and-the-desktop-profile-switch) apply, against
fuzzel's numbers. Needs the maintainer's ruling.

## Docs (same PR)

Part of this ticket's work, not a follow-up: update `site/src/content/docs/scootlaunch/index.md and README.md (What works / Not yet)` with the launcher as the default, how to opt out, and the breaking marker. Task-first, type/default/example/reload per option, `> **Symptom:**` boxes, snippet gates green (`site/OUTLINE.md`). A change with no user-visible effect says so in the PR instead.
