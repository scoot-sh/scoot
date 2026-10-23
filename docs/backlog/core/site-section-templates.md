---
title: "Site: scootlaunch/scootnotify break the per-app page template"
status: "open"
area: "core"
priority: "medium"
blocked: null
---

# Site: scootlaunch/scootnotify break the per-app page template

Filed 2026-10-22 (W48 docs-site UX pass). Serves **daily-drive**: a
reader who learns the overview → configure → reference →
troubleshooting shape once should find it in every app section.

## The gap

Every app section follows overview → configure → reference →
troubleshooting (`site/OUTLINE.md`, `site/astro.config.mjs` sidebar),
except:

- `scootlaunch/` has overview, panel, CLI reference — no configure
  page (panel keys/config/theming live only in `panel.md`) and no
  troubleshooting page.
- `scootnotify/` has overview, configure, CLI reference — no
  troubleshooting page.

Evidence: `ls site/src/content/docs/scootlaunch site/src/content/docs/scootnotify`
vs any of `scoot scootbar scootbg msg` (each has `troubleshooting.md`).

## What to do

Add `scootlaunch/troubleshooting.md` (panel won't open, ranking
surprises, icon misses, dmenu/run failures with diagnosing commands),
`scootnotify/troubleshooting.md` (no popups, popups under fullscreen,
DND stuck, history/restore surprises — much of it already exists in
`desktop/index.md#notifications` and the symptom index; link both
ways), and either a `scootlaunch/configure.md` split out of `panel.md`
or a conscious decision that panel+configure stays one page. Register
each in the sidebar and the `customSets` in `astro.config.mjs` so the
llms gate stays green (`pnpm verify`).

## Not in this ticket

Rewriting `panel.md` or the notify configure content — this is
structure plus symptom entries, not reference prose.
