---
title: "Desktop: display arrangement rules (kanshi-class) on the paved path"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-06"
---

# Desktop: display arrangement rules (kanshi-class) on the paved path

Filed 2026-10-04, child of `desktop-paved-path` (from PR #430's review).
Serves **daily-drive**: a laptop moving between a desk monitor and no monitor
needs its scale and placement to follow.

## The gap

scoot's `[[outputs]]` config sets scale and position per connector name, and
`wlr-output-management-v1` is read-only (`docs/protocols.md`), so a profile
switcher like kanshi cannot apply rules and there is no "this monitor set
means this layout" on the paved path. The M2 hand-sets DP-1 to 1x in its own
config today.

## What to do

Decide between scoot-native profiles (match a set of connected outputs by
name, make and model, then apply) and making `wlr-output-management-v1`
writable so kanshi works; prefer the standard protocol unless a concrete
reason says otherwise (CLAUDE.md), and say which. Expose it as a desktop
profile slot with eval pins.

## Acceptance

On the M2, plug and unplug DP-1 and show each profile applied (scale,
position) with `scoot msg outputs` before and after.

## Not in this ticket

A graphical display settings app.

## Resolution (PR #482, 2026-10-06)

Chose **scoot-native profiles**: `desktop.displays.profiles` plus a
`scoot-displays` watcher (home-manager user unit bound to
`scoot-session.target`), with eval pins and a `## Displays` site
section. Stock kanshi cannot drive scoot — its `exec` hooks run only
after a `succeeded` reply, which scoot's read-only write half never
sends (proven live: kanshi matched the profile, got `failed`, ran
nothing) — and a writable protocol would be atomic-modeset surgery
for a packaging slot. Two premises corrected: `[[outputs]]` carries
scale and mode only (position is automatic), and make/model cannot
key a profile (scoot's IPC reports names only). Software proof on the
M2 (headless 2→1 outputs: scale 1.0→2.0, power off, solo 1.5, all via
`scoot msg outputs`); kanshi idle measured at ~0 wakeups over 83 s.
Physical DP-1 plug/unplug steps are in the PR body for the
maintainer (no scoot session drives the real outputs right now).
