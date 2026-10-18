---
title: scootlaunch overview
description: "The keyboard-first application launcher — this page covers the shipped desktop entry index; the panel, ranking and launching arrive in later tickets."
---

A keyboard-first application launcher for scoot and any compositor
with `wlr-layer-shell-v1`. What ships today is the foundation every
later piece stands on: the desktop entry index and the headless agent
interface. The panel, fuzzy ranking, launching and the dmenu mode are
later tickets and are not here yet.

```sh
scootlaunch --list --json                  # every visible entry, as JSON
```

One binary, linking nothing beyond glibc and libgcc_s, needing no
compositor connection and holding nothing when closed. It reads the
same desktop files every launcher reads (`$XDG_DATA_HOME` then each
`$XDG_DATA_DIRS` entry's `applications/`, subdirectories walked,
first file per id wins, `Hidden=true` masking across data dirs),
expands each `Exec` to an argv per the Desktop Entry Specification,
and caches the result under `$XDG_CACHE_HOME/scootlaunch/index-v1`.

## What it does today

- Lists indexed entries, with `NoDisplay` and
  `OnlyShowIn`/`NotShowIn`/`TryExec`-hidden entries reachable through
  `--all` with their reason (`--list --all --json`).
- Prints the agent JSON schema: `id`, `name`, `generic_name`,
  `comment`, `keywords`, `icon`, `exec` (the expanded argv),
  `terminal`, `dbus_activatable`, `actions`, `path` and `hidden`.
- Expands `Exec` strictly per the spec, or with `--lenient` the way
  GLib reads the real files that break the quoting rule.

Every flag is in the [CLI reference](./cli.md). An app missing from
the list is diagnosed there too.
