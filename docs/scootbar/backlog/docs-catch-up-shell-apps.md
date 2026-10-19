---
title: "docs: catch the site up with the merged shell-app work"
status: "open"
area: "scootbar"
priority: "medium"
blocked: null
---

# docs: catch the site up with the merged shell-app work

Filed 2026-10-19. Serves **daily-drive** / **computer use** (pick one, say why).

## The gap

What is wrong or missing, with evidence (file paths, measured numbers).

## What to do

The proposed shape, and the edge cases to pin.

## Not in this ticket

What is deliberately out of scope.

Filed 2026-10-19 after #668–#674 merged. The new pages (`scootlaunch/`,
`scootnotify/`) landed with their PRs, but these spots still describe the
world before the panel and server existed. Audit each against `main`, fix, and
run the site build and snippet gates.

## What to do

- `site/src/content/docs/looks/index.md` (~line 90): `scoot look pick` is
  described as a fuzzel stopgap "until scootlaunch lands". It has landed; say
  what `look pick` uses now, or whether it should move to the panel.
- `site/src/content/docs/reference/look-format.md`: add the launcher fragment
  (`launch.toml`, its 13 roles, render version 11) beside the other fragments,
  and the `scootctl look` render/verify commands that now cover it.
- `site/src/content/docs/msg/index.md` (~line 43): the keyboard-holding layer
  surface text names fuzzel; mention the scootlaunch panel's exclusive keyboard.
- `README.md`: What works / Not yet rows for the launcher panel and the
  notification server (the server runs headless; popups are not in yet, so say
  that plainly).
- `site/src/content/docs/start/` install and Nix pages: README line 42 promises
  "notifications and a launcher"; confirm what `programs.scoot.desktop`
  installs today and correct either side.
- `site/OUTLINE.md`: the scootlaunch and scootnotify sections as they now stand.
- `dev/`: a short note on scootui's features (surfaces, keyboard, entry) for
  implementers, if none exists; keep it out of `site/`.

## Docs (same PR)

This ticket is the docs work. Each fix cites the commit that changed the behavior.

## Added 2026-10-19 (after #679)

- The panel screenshots on `scootlaunch/panel.md` use solid red and blue fixture squares for icons.
  They are accurate but plain next to the looks. Recapture with a couple of designed fixture icons
  (still generated in the test harness) so the docs show what the panel looks like with real apps.
- The entry's focus ring starts right of the entry icon; check the ring and icon spacing look
  deliberate at 1x and 1.5x.
