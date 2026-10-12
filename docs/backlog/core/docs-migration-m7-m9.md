---
title: "Docs migration remainder: split the desktop page (M7), STE pass (M8), coming-flips (M9)"
status: "open"
area: "core"
priority: "medium"
blocked: null
---

# Docs migration remainder: split the desktop page (M7), STE pass (M8), coming-flips (M9)

Filed 2026-10-11. Serves **daily-drive**: the site is how people learn the keys and install.

## The gap

The docs migration has landed M1 to M6 and the README rewrite (M4) is in
flight. Left:

- **M7**: split the desktop page.
- **M8**: a STE-100 style pass over the remaining pages, keeping the logo's dry
  voice in headings and intros only.
- **M9**: flip each "coming" slot (deb, rpm, Arch, ISO, shortcut teacher app) as
  its channel goes live.

## What to do

One PR per step, each through the docs gate (`nix build -L --no-link
.#docs-site`). Hold any new distro channel until the install guide is the easy
one (user rule, 2026-10-10).

## Not in this ticket

Publishing any package channel.
