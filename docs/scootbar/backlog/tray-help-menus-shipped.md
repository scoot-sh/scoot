---
title: "scootbar --help says tray menus are not built, but they are"
status: "open"
area: "scootbar"
priority: "low"
blocked: null
---

# scootbar --help says tray menus are not built, but they are

Filed 2026-10-14 (docs review slice 3). Serves **daily-drive**: `--help`
is the first thing a user reads, and it contradicts the shipped behavior.

## The gap

`crates/scootbar/src/cli.rs:389` (`tray_help!`) says: "Item menus are
not built yet (no DBusMenu client). Nothing is shown where no item is
registered." But the menus shipped: `crates/scootbar/src/modules/tray/mod.rs:11-14`
("Item menus (`ContextMenu`, the DBusMenu protocol) open through [the]
DBusMenu client"), `menu.rs`, `menu_tests.rs`, and the site reference
(`site/src/content/docs/scootbar/modules.md` tray section). The site docs
are the correct side (verified 2026-10-14 against the tray sources).

## What to do

Replace that help sentence with something like: "Item menus open in a
popup (DBusMenu: GetLayout, row click, submenus). Nothing is shown where
no item is registered." Check whether any test pins the old string, and
regenerate any pinned help output.

## Not in this ticket

No behavior change to the tray itself; text only.
