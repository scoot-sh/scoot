---
title: "Desktop: display arrangement rules (kanshi-class) on the paved path"
status: "open"
area: "packaging"
priority: "medium"
blocked: null
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
