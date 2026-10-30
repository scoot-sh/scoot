---
title: "Dark looks' inactive focus ring is below 3:1 on the background"
status: "open"
area: "core"
priority: "low"
blocked: null
---

# Dark looks' inactive focus ring is below 3:1 on the background

Filed 2026-10-30 from the review of PRs #824 and #825. Serves
**daily-drive**: whether an unfocused window's edge is visible against the
desktop is a readability call, and `border` also colors other surfaces (below).

## The gap

The validator's `border/background` pair (the inactive focus ring,
`crates/scoot-look/src/contrast.rs`) reports a warning, never a refusal, on
three shipped looks, and `music-desk`'s `urgent` also reports:

| look | pair | ratio | candidate |
|---|---|---|---|
| ginger-night | `border/background` | 1.5 | `#795B46` |
| moonrise | `border/background` | 1.5 | `#8C7C82` |
| vinyl-sunset | `border/background` | 2.5 | `#89556E` |
| music-desk | `urgent/background` | 2.9 | none picked |

The candidates are the review's suggestions for reaching 3:1, not measured
against the looks' other pairs. The warnings print on
`scootctl look sync` and `scootctl look apply`, and the four pairs are locked as
known warnings in `crates/scoot-look/tests/shipped.rs`
(`shipped_contrast_warnings_are_the_known_pairs`), so a retune shows up as a
test change.

Related, from the same reviews:

- The validator rounds ratios to one decimal, so a real 4.46 reports as 4.5
  and reads as clearing a 4.5 bar it misses. Report two decimals, or compare
  on the unrounded value and print the rounded one.
- One surviving mutation: the `border` pair's displayed `foreground` label is
  not pinned by any test (changing which color the warning names passes).
- The per-app terminal themes under `looks/moonrise/apps` and
  `looks/vinyl-sunset/apps`, and `docs/examples`, still carry the dim hexes
  from before #824's dim lift. Not re-audited here; diff them against the
  looks' manifests.

## What to do

A design decision first: retune the three dark rings to at least 3:1, or accept
them as deliberately muted and say so in the look docs. `border` is not only the
inactive ring: it colors scootui card borders and separators, so brightening it
changes those too. Check the notification card and launcher panel in each look
before landing a value. If retuned, update the lock in `shipped.rs`, the
per-app themes and examples above, and re-run `scootctl look sync` to confirm no
warnings remain. Then the validator rounding and the label pin, which are
independent of the design call.

## Not in this ticket

A new role for the inactive ring (a separate color from `border`).
