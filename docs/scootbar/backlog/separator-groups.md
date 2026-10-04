---
title: "Separators between groups of modules, not every pair"
status: "open"
area: "scootbar"
priority: "low"
blocked: null
milestone: "M6"
---

# Separators between groups of modules, not every pair

Filed 2026-10-03, from the same showcase shoot. Serves **daily-drive**
(looks): `[bar] separator` draws a line between every neighbouring pair of
modules in a section. The maintainer's own bar groups related modules with
no line inside a group (load next to CPU) and lines between groups; today
that cannot be expressed.

## What to do

Decide a shape and keep it small: for example a `"|"` entry in the
`left`/`center`/`right` lists that marks where a separator goes (and
`separator` then draws only there), or a per-module `separator-after`.
Whatever the shape, a list with no markers behaves exactly as today. Pin it
with the layout tests and a snapshot.

## Not in this ticket

Group backgrounds (a pill behind a group), which is a bigger theming entry.
