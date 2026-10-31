---
title: scootlaunch CLI reference
description: "Every scootlaunch command and flag that ships today: the panel, --list, --query, --run-id, --dmenu, --run, --all, --lenient, --json, --check, and the agent JSON schema."
---

Every `scootlaunch` command and flag that ships today, and how it
behaves at the edges. What scootlaunch is for is in
[Overview](./index.md). The panel's keys, config and theming are in
[The panel](./panel.md).

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
scootlaunch --run-id ID [--action A]       # launch one entry headlessly, one JSON object
scootlaunch --dmenu [flags]                # lines on stdin, the pick on stdout
scootlaunch --run                          # the panel over executables on $PATH
scootlaunch --help                         # print help
scootlaunch --version                      # print the version
```

`--all` and `--json` need `--list` or `--query` (`--lenient`
also shapes the panel's index and `--run-id`). `--action` needs
`--run-id`. `--font` is a panel option; `--config` belongs to the
panel and `--check`. `--list`, `--query`, `--run-id`, `--check`,
`--dmenu` and `--run` cannot combine. `--check` refuses a bad config
file with exit 1, except a bad value in `[effects]`: that prints on
stderr and exits 0, as the panel ignores it. TEXT is the next
argument after `--query`, whatever it holds. Anything else is a loud
error: exit 2, naming the flag and the supported list, never silently
ignored. The panel launches the picked entry (exit 0) or nothing on
dismiss (exit 1): the codes are unchanged from the print-the-id days,
but a launch prints nothing anymore.

## Dmenu mode

`--dmenu` reads lines on stdin and prints the pick on stdout, with
fuzzel's meaning for every flag the repo's call sites pass. A dmenu
pick prints and launches nothing. The panel is drawn with the look's
shadow, fill and opening ([Depth and motion](./panel.md#depth-and-motion)), and exits
the moment it prints, with no fade.

```sh
printf 'wifi-a\nwifi-b\n' | scootlaunch --dmenu --prompt='wifi: ' --no-run-if-empty --only-match
```

Stdin is read to EOF before the first frame, with a 5 s bound on a
writer that never closes (after which what arrived is shown). Bounds:
at most 1 MiB and 65,536 lines, each line at most 4 KiB (longer lines
are cut at a character boundary). Non-UTF-8 input is decoded lossily;
NUL bytes ride through as text (there is no icon protocol in v1, so
`foo\0icon\x1fbar` is one literal line). An empty query shows stdin
order — the writer's order, with no frecency to break it.

| Flag | Type / default | Example |
|---|---|---|
| `--prompt TEXT` | text, `"> "` | `--prompt='wifi: '` |
| `--prompt-only TEXT` | text, none (needs `--dmenu`) | `--prompt-only='key for net: '` |
| `--password` | flag, `*` | `--password` |
| `--no-run-if-empty` | flag, off | `--no-run-if-empty` |
| `--only-match` | flag, off | `--only-match` |
| `--index` | flag, off | `--index` |
| `--placeholder TEXT` | text, empty | `--placeholder='pick one'` |
| `--layer overlay` | `top` or `overlay`, `overlay` | `--layer=overlay` |

`--prompt` text takes its value attached (`--prompt=TEXT`) or as the
next argument. `--prompt-only` is an input field with no list: stdin
is not read at all (a password or WiFi-key prompt reading from
`/dev/null`). `--password` masks the entry with `*`; the secret is written to stdout
only on Enter, never logged, and wiped when the panel closes. `--no-run-if-empty` exits 1 without
opening on empty stdin. `--only-match` makes Enter with no match do
nothing (without it the typed text prints); Ctrl+Enter still prints
the typed text, even with `--only-match`. `--index` prints the
0-based stdin position of the pick instead of its line. The panel is
always the `overlay` layer; `top` is accepted and draws the same.

Exit codes: 0 for a printed pick, 1 for a dismiss (Escape), an empty
feed with `--no-run-if-empty`, or a runtime failure, 2 for a usage
error (an unsupported flag, a missing value, `--dmenu` combined with
another command). A second `scootlaunch --dmenu` while one is open
opens a second panel: dmenu instances neither listen for nor send the
single-instance toggle.

## Run mode

`--run` (alias `--list-executables-in-path`, fuzzel's flag) opens the
panel over every executable file on `$PATH`: the first directory
naming a basename wins, at most 65,536 names. Typing filters,
frecency orders (keyed `run:<basename>` in the same history file),
and Enter launches the pick by basename through a `PATH` lookup —
under the same token, scope and fallback rules as an apps pick, and
recording the same history. Ctrl+Enter runs the typed text split on
spaces. Nothing prints; dismissing exits 1.

```sh
scootlaunch --run
scootlaunch --list-executables-in-path
```

## Per-invocation colors

Seven `rrggbbaa` flags override the look's roles for one open,
above the user's file and the look — the same seven the look passes
the repo's pickers, so one menu reads as one wherever it opens.
`--layer=overlay` rides along the same way (the launcher script
passes it to the bare panel too).

| Flag | Role |
|---|---|
| `--background-color` | panel fill (its alpha becomes the panel's opacity) |
| `--text-color` | input and row text |
| `--border-color` | panel border |
| `--selection-color` | selected row fill |
| `--selection-text-color` | selected row text |
| `--match-color` | matched characters (on the selected row too when it contrasts with the fill; else the inverted chip, so the row stays legible) |
| `--prompt-color` | prompt text |

Only `--background-color` may be translucent; any other color with an
alpha past `ff` is refused loudly (exit 2), since the panel draws
those roles solid.

## Ranking

The query matches case-insensitively (Unicode folded, so `strasse`
finds `Straße`) against the name, generic name, keywords and Exec
basename, in that weight order. A prefix beats a scattered match.
Each launch adds frecency weight: recent and frequent entries rank
higher, but never above a much better text match. The history lives
at `$XDG_STATE_HOME/scootlaunch/history` (default
`~/.local/state`); deleting it clears the bonus. An entry's desktop
actions rank as their own rows ("Firefox: New Private Window") but
print once, at the best row's position. Dmenu mode keeps no history:
an empty query shows stdin order.

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

## Run one entry headlessly

`--run-id ID [--action A]` launches one entry with no compositor
connection and no surface, through the same spawn path as a panel
pick (activation first for `DBusActivatable` entries, else `Exec`;
the environment rules, the systemd scope, the detached fallback with
no bus, frecency). A fresh spawn still takes focus on map. No token:
there is no surface and no serial to mint from.

Stdout carries exactly one JSON object, nothing else. Exit 0 prints a
success object; exit 1 prints an error object. Notes and warnings go
to stderr.

| Field | Type / default | Example |
|---|---|---|
| `ok` | `true` on success | `true` |
| `pid` | number, when a child was spawned | `4242` |
| `scope` | text, when the scope move landed | `"app-scootlaunch-firefox.desktop-1a2b3c4d.scope"` |
| `activated` | `true`, when the launch was an in-place activation | `true` |
| `detached` | `true`, when the launch went through the bus-less fallback | `true` |

Only the fields that happened are present. A spawn whose scope move
was refused still runs: `pid` is set, `scope` is absent, and one
warning names the refusal on stderr.

```sh
scootlaunch --run-id org.mozilla.firefox.desktop
# {"ok":true,"pid":4242,"scope":"app-scootlaunch-org.mozilla.firefox.desktop-1a2b3c4d.scope"}
scootlaunch --run-id org.mozilla.firefox.desktop --action new-private-window
# {"ok":true,"pid":4310,"scope":"app-scootlaunch-org.mozilla.firefox.desktop-9f8e7d6c.scope"}
```

The id matches exactly (`firefox.desktop`); the `.desktop` suffix may
be left off when it names exactly one entry (`firefox`). Anything
else never launches: a prefix, a fuzzy match, or a suffix-insensitive
form naming more than one entry is an error. Hidden entries (`--all`)
are still launchable by id. The action must name one of the entry's
`actions` exactly.

| `code` | Meaning |
|---|---|
| `invalid-id` | empty, longer than 1024 bytes, or holding control characters |
| `unknown-id` | no entry matches |
| `ambiguous-id` | the suffix-insensitive form matches more than one entry |
| `invalid-action` | empty, longer than 1024 bytes, or holding control characters |
| `unknown-action` | the entry has no action with that id |
| `launch-failed` | the spawn or activation failed (nothing started) |

Exit codes: 0 for a launch, 1 for a refused launch (the error object
above), 2 for a usage error (`--action` without `--run-id`,
`--run-id` combined with another command, a missing value). An agent
loop an agent can paste:

```sh
scootlaunch --list --json | jq '.[] | {id, name}'
scootlaunch --query "fire" --json | jq '.[0] | {id, name}'
scootlaunch --run-id org.mozilla.firefox.desktop
scoot msg windows | jq '.windows[] | {app_id, title}'
```

> **Symptom:** `--run-id` says `unknown-id` for an app the list shows.

The list shows the exact id: copy it whole, suffix included. A typed
prefix never launches.

> **Symptom:** `--run-id` says `launch-failed` with no window.

The `Exec` target vanished between the index and the launch, or the
activation was refused and no `Exec` fallback exists. The object's
`error` names the step; the index note on stderr names the file.

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

> **Symptom:** a dmenu script that worked under fuzzel fails loudly.

`scootlaunch --dmenu` speaks the flags in the table above and
`--index` / `--placeholder` only. Any other fuzzel flag exits 2
naming the flag: the call sites in this repo are enumerated, and an
eval pin keeps them inside the supported set. A user's own script
with another flag fails here instead of silently changing meaning.
