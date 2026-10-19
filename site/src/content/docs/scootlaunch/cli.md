---
title: CLI reference
description: "Every scootlaunch command and flag that ships today: the panel, --list, --query, --all, --lenient, --json, --check, and the agent JSON schema."
---

Every `scootlaunch` command and flag that ships today, and how it
behaves at the edges. What scootlaunch is for is in
[Overview](./index.md). Later tickets add `--run-id` and
the dmenu mode; this page grows with them.

## Commands

```sh
scootlaunch                                # open the panel (see The panel)
scootlaunch --font PATH                    # the panel with an explicit typeface
scootlaunch --config PATH                  # the panel with an explicit config file
scootlaunch --check [--config PATH]        # validate the config file and exit
scootlaunch --list                         # every visible entry: id and name per line
scootlaunch --list --all                   # hidden entries too, with their reason
scootlaunch --list --json                  # the agent JSON schema, one array
scootlaunch --list --all --json            # hidden entries included, with hidden set
scootlaunch --list --lenient               # Exec quoting the way GLib reads it
scootlaunch --query TEXT                   # entries ranked for TEXT, best first
scootlaunch --query TEXT --json            # the agent JSON schema, in ranked order
scootlaunch --query ""                     # empty query: by frecency, then name
scootlaunch --help                         # print help
scootlaunch --version                      # print the version
```

`--all` and `--json` need `--list` or `--query` (`--lenient` also
shapes the panel's index). `--font` is a panel option; `--config`
belongs to the panel and `--check`. `--list` and `--query` cannot
combine, and neither combines with `--check`. TEXT is the next
argument after `--query`, whatever it holds. Anything else is a loud
error: exit 2, naming the flag and the supported list, never silently
ignored. The panel launches the picked entry (exit 0) or nothing on
dismiss (exit 1): the codes are unchanged from the print-the-id days,
but a launch prints nothing anymore.

## Ranking

The query matches case-insensitively (Unicode folded, so `strasse`
finds `Straße`) against the name, generic name, keywords and Exec
basename, in that weight order. A prefix beats a scattered match.
Each launch adds frecency weight: recent and frequent entries rank
higher, but never above a much better text match. The history lives
at `$XDG_STATE_HOME/scootlaunch/history` (default
`~/.local/state`); deleting it clears the bonus. An entry's desktop
actions rank as their own rows ("Firefox: New Private Window") but
print once, at the best row's position.

## The agent JSON schema

`--list --json` prints one JSON array. Each entry carries `id`,
`name`, `generic_name`, `comment`, `keywords`, `icon` (name or path),
`exec` (the expanded argv), `terminal`, `dbus_activatable`, `actions`
(`[{id, name}]`), `path` (the desktop file) and `hidden` (null, or the
reason: `NoDisplay`, `OnlyShowIn`, `NotShowIn`, `TryExec`). Absent
optionals serialize away rather than as nulls. `--query TEXT --json`
prints the same schema in ranked order.

```sh
scootlaunch --list --json | jq '.[] | select(.id == "org.mozilla.firefox.desktop")'
scootlaunch --query "fire" --json | jq '.[0] | {id, name}'
```

## Troubleshooting

> **Symptom:** an app is missing from the list.

Ask for the hidden entries with their reasons:

```sh
scootlaunch --list --all --json | jq '.[] | select(.hidden != null) | {id, hidden}'
```

A `TryExec` reason means its helper is not installed or not
executable. An entry dropped entirely (not even listed with
`--all`) has no usable `Exec` — an unknown `%x` field code is the
usual cause — and the run says so on stderr with the file and the
reason. `--lenient` recovers the entries whose quoting breaks the
spec's rule but parses the way GLib reads them.

> **Symptom:** the list is stale after editing a desktop file.

The cache key covers every directory and file walked plus the locale
and `$XDG_CURRENT_DESKTOP`, so an edit rescans on the next run. A
bad cache is never an error: any mismatch means a rescan.
