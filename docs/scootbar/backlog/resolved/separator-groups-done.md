---
title: "Separators between groups of modules, not every pair"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M6"
resolved: "2026-10-07"
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

## Resolution (2026-10-07)

Shipped the ticket's first shape: a `"|"` entry in the
`left`/`center`/`right` lists (config file, `--left`/`--center`/`--right`
flags and per-output `[output."NAME"]` lists alike) marks where a separator
goes. A section list with no marks draws every gap, exactly as before, so
an old config looks byte-identical; a list with any marks draws only where
a mark stands. A mark needs a module on both sides of it in its own section
(leading, trailing and doubled marks are refused, naming the list); it is
never started, takes no space and is not counted in the 32-module most, so
an unstarted or empty module beside one never moves the line. The choice is
the ticket's own first suggestion, kept per-section (an unmarked list keeps
the old behavior even beside a marked one) and trivially reversible (drop
the marks and the old behavior is back).

Pinned by layout tests (marks travel with their section, stray marks
refused per section, only modules count toward the most), the render's
member flags (`members_mark_only_the_marked_gaps`, including across an
unstarted module), paint tests (only the marked gap draws;
`separators_draw_only_where_a_mark_stands`, and the mark beside an empty
module still marks the visible gap), config/flag parsing tests and a
`bar-separated-groups-1x` snapshot. Docs in the same change: the Layout and
Spacing sections of the configure page, the `cli.md` reference, and the
daemon help and `--help --json` texts.
