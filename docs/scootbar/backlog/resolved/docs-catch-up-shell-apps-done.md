---
title: "docs: catch the site up with the merged shell-app work"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-19"
---

# docs: catch the site up with the merged shell-app work

Filed 2026-10-19. Serves **daily-drive**: every fix tells a user
installing the desktop today which launcher and notification daemon
they get and how to opt into the new ones.

## Resolution

Landed in the `docs/shell-apps-catch-up` PR (see its body for the
per-fix behavior commits, `f49f7f743`–`5c41eb812`):

- `looks/index.md`: `scoot look pick` still uses fuzzel — the panel
  landed but the picker has not moved — said plainly, with the panel
  linked. The apply path now names the shell-app fragments:
  `launch.toml` read at every open, `notify.toml` on
  `msg reload --look` (apply reloads compositor and bars only).
- `reference/look-format.md`: `launch.toml` / `notify.toml` (thirteen
  roles plus font, size, opacity, radius, icon theme) beside the other
  fragments, in the directory listing, the fragment section and the
  `validate` / `build` output list. No render-version number is
  claimed: the tree has none.
- `msg/index.md`: the keyboard-holding layer surface names the
  scootlaunch panel's exclusive keyboard first.
- `README.md`: `scootlaunch` (opt-in panel) and `scootnotify` (popups
  plus `msg`) rows in What comes in the box; Not yet names the default
  flip, the bar feed and scootnotify packaging/Nix.
- Install (`start/install.mdx`), compose (`start/compose.mdx`) and the
  desktop page: the profile installs mako and fuzzel; scootlaunch is
  one switch (`daemon = "scootlaunch"`, `install.sh --launch`);
  scootnotify runs but has no module or packaging. The stale "control
  socket is a later ticket" line on `scootnotify/index.md` now points
  at the landed `msg` reference.
- `site/OUTLINE.md`: both sections as they stand, with the landed
  ticket numbers.
- `dev/scootui.md`: new as-built implementer note (features,
  consumers, how to add one). The design doc stays the plan record.
- Screenshots recaptured from redesigned generated fixtures (terminal
  `>_` PNG mark, browser-disc SVG, letter tile) in both looks; the
  entry ring starts 1 device px right of the entry icon at 1x, 2 px at
  1.5x (measured unit-level probe, temporary), no overlap either way.
- Rebased onto `origin/main` past #685 (DND, history ring with
  restore, `[[rule]]` with exec, control protocol 1→2, itself
  documented in that PR's cli/configure): the overview's What-it-does
  list, the categories/`desktop-entry` conformance rows, the later-
  tickets line, the CLI description and every scootnotify summary now
  cover them; only the bar feed (`notify-bar-feed`, claimed, unmerged)
  stays listed as later, and nothing unmerged is documented.

Evidence: `cargo nextest run -p scootlaunch` 311 passed on the dev VM,
`cargo clippy -p scootlaunch --all-targets` clean, site `astro build`
plus llms / md-links / assets / snippets / mdx-twin / nix /
test-install gates green locally, docs-site CI job on the PR.

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
