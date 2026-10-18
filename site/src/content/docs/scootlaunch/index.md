---
title: scootlaunch overview
description: "The keyboard-first application launcher — this page covers the shipped desktop entry index and ranked search; the panel, launching and the dmenu mode arrive in later tickets."
---

A keyboard-first application launcher for scoot and any compositor
with `wlr-layer-shell-v1`. What ships today is the foundation every
later piece stands on: the desktop entry index, ranked search, and
the headless agent interface. The panel, launching and the dmenu mode
are later tickets and are not here yet.

```sh
scootlaunch --list --json                  # every visible entry, as JSON
scootlaunch --query "fire" --json          # ranked as the UI would rank it
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
- Ranks entries for a query (`--query TEXT`): a case-folded fuzzy
  match over the name, generic name, keywords and Exec basename,
  plus a frecency bonus from `$XDG_STATE_HOME/scootlaunch/history`.
  The bonus never outranks a much better text match. An empty query
  orders by frecency, then name.
- Prints the agent JSON schema: `id`, `name`, `generic_name`,
  `comment`, `keywords`, `icon`, `exec` (the expanded argv),
  `terminal`, `dbus_activatable`, `actions`, `path` and `hidden`.
  `--query --json` prints the same schema in ranked order.
- Expands `Exec` strictly per the spec, or with `--lenient` the way
  GLib reads the real files that break the quoting rule.

Every flag is in the [CLI reference](./cli.md). An app missing from
the list is diagnosed there too.
