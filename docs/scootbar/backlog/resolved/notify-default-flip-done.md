---
title: "scootnotify: make it the desktop profile's default notification daemon"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M7"
resolved: "2026-10-21"
---

# scootnotify: flip the default

Filed 2026-10-12. Serves **daily-drive**. Resolved 2026-10-21 by the
notify-default flip PR (`feat(nix)!: notifications default to
scootnotify`, branch `feat/notify-default-scootnotify`): the
maintainer ruled "Let's make the flip!" (sole user, so breaking ships
with no alias but is marked).

What landed: `desktop.notifications.daemon` defaults to `"scootnotify"`
on the home-manager and NixOS sides, the profile/keys/bar/look slots
follow it, the flake wrappers fill the package from the flake's own
build, and `daemon = "mako"` stays one line away (evaluated-output
diff proves the mako shape byte-identical to main). Docs, migration
notes (`Migrating from mako`), install.sh wording and the CHANGELOG
breaking entry in the same PR. Evidence: raw eval outputs and red
proofs in the PR body; every check green on the exact head.

Deferred, not dropped: the honest M2 headless measurement
([scootnotify vs mako A/B](../notify-ab-bench.md)) — no numbers are
published or claimed.

## Docs (same PR)

Part of this ticket's work, not a follow-up: update `site/src/content/docs/scootnotify/index.md and README.md (What works / Not yet)` with scootnotify as the default, how to opt out, and the breaking marker. Task-first, type/default/example/reload per option, `> **Symptom:**` boxes, snippet gates green (`site/OUTLINE.md`). A change with no user-visible effect says so in the PR instead.
