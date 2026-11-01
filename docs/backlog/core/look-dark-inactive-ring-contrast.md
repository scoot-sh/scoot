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

## Notes (2026-11-01) — design-independent parts done, decision pending

**Decision needed from the human:** retune the three dark rings to the
candidates below, or accept them as deliberately muted and say so in
the look docs. Measurements, screenshots and per-look recommendations
follow; no look color was changed. Status stays open, `blocked` stays
null until the call is made.

### Candidate measurements (validator, two decimals)

`scoot-look validate --origin shipped` on border-swapped copies of each
look: every candidate clears its 3.0 bar with no other warning. Ratios
(WCAG, unrounded; the validator now compares unrounded and prints two
decimals):

| look | current `border` | ratio | candidate | ratio |
|---|---|---|---|---|
| ginger-night | `#4E2913` | 1.52 | `#795B46` | 3.12 |
| moonrise | `#5E4B5B` | 1.53 | `#8C7C82` | 3.09 |
| vinyl-sunset | `#7D4564` | 2.52 | `#89556E` | 3.11 |
| music-desk | `urgent #EE6F5E` | 2.88 | none picked | — |

All three dark looks default `surface` to `background`, carry no
gradient fill, and render popups opaque, so the card fill, the
launcher-panel fill and the swaylock inside all equal `background`:
every `border` surface measures the same ratio as the ring. Margins
are thin (3.09–3.12), so a later palette nudge re-opens the question.

Where `border` actually paints (source-checked, correcting the
"card borders and separators" shorthand above): the compositor's
inactive ring; the notification card's edge, but only on low-urgency
cards (normal uses `focus`, critical uses `urgent`); swaylock's
`line-color`/`separator-color`; multi-row separators where present.
The launcher panel's rim is `focus` — `border` draws nothing there —
and the bar, fuzzel, mako, wob and fcitx5 leaves never read `border`.

### Screenshots (before/after, CPU tier)

`headless --renderer cpu`, 1600x1000, one output: two shm windows
(focused + inactive ring), a normal-urgency card, a low-urgency card
(border rim), and the bare launcher panel, each look before (current
border) and after (candidate). Applied with `scoot look apply` into a
scratch state dir. Pixel-verified: every after shot carries only the
candidate hex on its rings/card rim (zero current-border pixels) and
vice versa, with identical ring geometry. Copies on this machine at
`/tmp/ringc-shots/` (`<look>-<before|after>-<ring|notify|notify-low|launch>.png`,
plus `cmp/` side-by-side crops):

- ginger-night: `ginger-night-before-ring.png`,
  `ginger-night-after-ring.png`, `ginger-night-before-notify-low.png`,
  `ginger-night-after-notify-low.png`,
  `ginger-night-before-launch.png`,
  `ginger-night-after-launch.png` (and `-notify` for the normal card).
- moonrise: same six names under `moonrise-`.
- vinyl-sunset: same six names under `vinyl-sunset-`.

### Recommendations

- ginger-night: retune to `#795B46`. The current 1.52 ring is
  indistinguishable from the near-black background at normal viewing
  distance — an unfocused window simply has no visible edge — while
  3.12 reads as a warm-brown edge inside the look's ember range. The
  low-urgency card rim gains the same edge; normal cards and the
  panel are pixel-identical either way.
- moonrise: retune to `#8C7C82`. Same shape as ginger-night with a
  bigger payoff on the card: the low-urgency rim goes from invisible
  to a clearly defined warm-gray outline that matches the look's
  mauve secondary text. 1.53 → 3.09.
- vinyl-sunset: either is defensible, slight lean to retune. The
  current 2.52 already reads as a deliberate muted-mauve outline and
  the candidate (3.11) changes it only modestly, but landing `#89556E`
  clears the bar, stays in-hue, and keeps the validator quiet. If the
  muted ring is intentional, keep `#7D4564` and note it in the look
  docs instead.
- music-desk urgent (2.88): no candidate was picked and none is
  proposed here; it is report-only, so this needs its own design pick
  whenever someone wants it.

### App-theme audit (done)

`looks/moonrise/apps`, `looks/vinyl-sunset/apps` and `docs/examples`
carried the pre-#824 dim hexes in dim-role slots
(`#9C8B95`/`#AA6D5F` in btop, starship, Helix `mist`, Neovim `mist`,
lazygit inactive border, `bar.toml` dim, regreet border, README
swatches; plus music-desk's old dim `#C9CBD0` in `bar.toml` and old
border `#D5D7DD` in `scoot.toml`'s `focus_ring_inactive_color`). All
were exact copies of the pre-#824 manifest hexes, so they were lifted
mechanically to the current manifest hexes. Everything else that
differs from the manifests is the authors' own derived shades
(moonrise `#3a465a`/`#4e3b40`/`#8fb3bd`/`#232c3b`/`#242e3d`/`#333f52`,
vinyl `#582818`/`#965c50`/`#39140f`, ginger-night's set,
music-desk's light-theme set, `#000000` in regreet dropshadows):
left untouched, listed here for the record.

### Done in the accompanying PR (design-independent)

Validator rounding (compare unrounded, print two decimals everywhere,
lock re-pinned at 1.52/1.53/2.52/2.88), the `border` foreground-label
pin (unit + shipped-looks tests, proven to fail under mutation), and
the audit fixes above. Docs updated in the same PR.
