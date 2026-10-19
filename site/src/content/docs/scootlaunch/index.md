---
title: scootlaunch overview
description: "The keyboard-first application launcher — the desktop entry index, ranked search, the overlay panel, launching, and the dmenu and run modes."
---

A keyboard-first application launcher for scoot and any compositor
with `wlr-layer-shell-v1`. What ships today is the desktop entry
index, ranked search, the headless agent interface, the overlay panel
with its entry and virtualized list, launching, and the dmenu and run
modes.

```sh
scootlaunch --list --json                  # every visible entry, as JSON
scootlaunch --query "fire" --json          # ranked as the UI would rank it
scootlaunch                                # open the panel
printf 'a\nb\n' | scootlaunch --dmenu      # pick a line, print it
scootlaunch --run                          # the panel over $PATH executables
```

One binary, holding nothing when closed. It reads the
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
- Opens the panel (bare `scootlaunch`): the overlay
  exclusive-keyboard surface with the entry and the ranked,
  match-highlighted list. Details, keys and theming are in [The
  panel](./panel.md). Picking launches the entry and exits 0 (a dmenu
  pick prints instead of launching; see the [CLI
  reference](./cli.md)).

Every flag is in the [CLI reference](./cli.md). An app missing from
the list is diagnosed there too.
