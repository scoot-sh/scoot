---
title: "scootnotify: make it the desktop profile's default notification daemon"
status: "open"
area: "scootbar"
priority: "low"
blocked: "notify-packaging"
milestone: "M7"
---

# scootnotify: flip the default

Filed 2026-10-12. Serves **daily-drive**. The criteria are the
[shared ones](../../../dev/design/shell-apps.md#nix-modules-and-the-desktop-profile-switch):

- the gate passed;
- the ratchet is at or under mako's;
- two weeks of daily use on the M2 with no open high-priority bug;
- a real-login proof per look;
- mako stays one option away for a release.

Needs the maintainer's ruling.

## Docs (same PR)

Part of this ticket's work, not a follow-up: update `site/src/content/docs/scootnotify/index.md and README.md (What works / Not yet)` with scootnotify as the default, how to opt out, and the breaking marker. Task-first, type/default/example/reload per option, `> **Symptom:**` boxes, snippet gates green (`site/OUTLINE.md`). A change with no user-visible effect says so in the PR instead.
