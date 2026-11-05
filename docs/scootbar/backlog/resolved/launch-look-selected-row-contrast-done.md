---
title: "looks: selected-row contrast in ginger-night"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-11-05"
---

# looks: selected-row contrast in ginger-night — RESOLVED, not a defect

Filed 2026-10-19. Serves **daily-drive** / **computer use** (pick one, say why).

## The gap

What is wrong or missing, with evidence (file paths, measured numbers).

## What to do

The proposed shape, and the edge cases to pin.

## Not in this ticket

What is deliberately out of scope.

Found reviewing #674 (visual). The selected row is hard to tell from the rest in ginger-night. Check the contrast of every look's launcher selection role against its row background and add a check to the look render tests.

## Docs (same PR)

Update `site/src/content/docs/scootlaunch/panel.md` if the change is user-visible; otherwise say so in the PR.

## Resolution (2026-11-05)

Not real on current main (`1d4c9a07a`), so no look token or painter changed.
The ticket was filed reviewing #674, before #709 (`fix(scootlaunch): readable
match highlight on the selected row`): the row's match highlight drew `hover`
(`#FFB14B`) on the `accent` fill (`#E57F29`), 1.57:1, which is what made the
selected row read as hard to tell apart. #709 resolves the selected-row ink
from measured contrast (`Theme::match_on_selection`), and every shipped look
resolves to the inverted chip.

Measured (WCAG ratio, from `looks/*/look.toml`, selection = `accent`,
selection-text = `background`):

| Look | text on fill |
|---|---|
| ginger-night | 6.80 |
| moonrise | 6.22 |
| music-desk | 6.71 |
| radial-burst | 6.51 |
| vinyl-sunset | 8.55 |

No shipped look sets `surface`, so the panel is `background` and the fill on
the panel is the same pair, the same ratio.

Every look clears the 4.5 text bar by a wide margin; ginger-night is not the
outlier. Screenshot of scootlaunch under the ginger-night colors on the dev
VM, headless scoot, `fi` typed:
[launch-look-selected-row-contrast-gn.png](launch-look-selected-row-contrast-gn.png).

What landed: `the_selected_row_clears_its_contrast_floor_in_every_shipped_look`
in `crates/scootlaunch/src/look/tests.rs`, pinning for each shipped look, as
the panel reads the rendered fragment, text on fill at least 4.5, fill on
panel at least 3, and the selected-row match at least 3. The look pipeline
already gates `accent-fg/accent` at 4.5 (`scoot-look` `contrast::check`), but
that is the drawn pair only while `selection` and `selection-text` are not
overridden apart, so the new test pins what is drawn.
