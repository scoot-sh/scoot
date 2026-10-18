---
title: CLI reference
description: "Every scootlaunch command and flag that ships today: --list, --all, --lenient, --json, and the agent JSON schema."
---

Every `scootlaunch` command and flag that ships today, and how it
behaves at the edges. What scootlaunch is for is in
[Overview](./index.md). Later tickets add `--query`, `--run-id`, the
panel and the dmenu mode; this page grows with them.

## Commands

```sh
scootlaunch --list                         # every visible entry: id and name per line
scootlaunch --list --all                   # hidden entries too, with their reason
scootlaunch --list --json                  # the agent JSON schema, one array
scootlaunch --list --all --json            # hidden entries included, with hidden set
scootlaunch --list --lenient               # Exec quoting the way GLib reads it
scootlaunch --help                         # print help
scootlaunch --version                      # print the version
```

`--all`, `--lenient` and `--json` need `--list`. Anything else is a
loud error: exit 2, naming the flag and the supported list, never
silently ignored.

## The agent JSON schema

`--list --json` prints one JSON array. Each entry carries `id`,
`name`, `generic_name`, `comment`, `keywords`, `icon` (name or path),
`exec` (the expanded argv), `terminal`, `dbus_activatable`, `actions`
(`[{id, name}]`), `path` (the desktop file) and `hidden` (null, or the
reason: `NoDisplay`, `OnlyShowIn`, `NotShowIn`, `TryExec`). Absent
optionals serialize away rather than as nulls.

```sh
scootlaunch --list --json | jq '.[] | select(.id == "org.mozilla.firefox.desktop")'
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
