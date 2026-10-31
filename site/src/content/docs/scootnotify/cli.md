---
title: scootnotify CLI reference
description: "Every scootnotify command and flag: daemon, --check, --config, --font, and msg (list, history, dnd, restore, dismiss, invoke, reload, subscribe, wait, version)."
---

Every `scootnotify` command and flag, and how it behaves at the
edges. What scootnotify is for is in [Overview](./index.md);
placement, timeouts and the look layer are in
[Configure](./configure.md).

## Commands

```sh
scootnotify daemon                          # own the name and serve until killed
scootnotify daemon --check                  # validate the config, exit 0 or 1
scootnotify daemon --config PATH            # read PATH instead of the default
scootnotify daemon --font PATH              # draw with PATH instead of the look's font or the first well-known one
scootnotify msg list [--json]               # live notifications: lines, or JSON
scootnotify msg history [--json]            # the history ring: lines, or JSON
scootnotify msg dnd [on|off|toggle|status]  # read or steer do-not-disturb
scootnotify msg restore                     # re-show the newest history entry
scootnotify msg dismiss [ID | --all]        # close cards, as a click would
scootnotify msg invoke ID [ACTION]          # run a card's action (default: "default")
scootnotify msg reload [--look]             # re-read the config and the look layer
scootnotify msg subscribe [notify] [close] [invoke] [dnd]  # print events until closed
scootnotify msg wait [--app APP] [--summary-contains TEXT] [--timeout SECS]
scootnotify --help                          # print help
scootnotify --version                       # print the version
```

`--help` and `--version` win wherever they stand. Exit statuses: 0
done, 1 failed, 2 misused, 3 the name is already owned (the unit stops
instead of restart-looping).

A taken name refuses with the owner named:

```sh
scootnotify daemon
# org.freedesktop.Notifications is owned by mako (pid 1234);
# scootnotify will not take it over. Stop that daemon first and
# remove its D-Bus activation file, or it will come back on the
# next notification.
# exit 3
```

## Configure

`$XDG_CONFIG_HOME/scoot/notify.toml` beside the bar's (`--config`
overrides it, `--check` validates it). Every key is optional; a
missing file is the built-in defaults. Unknown keys are refused (a
typo that silently did nothing would be worse than an error), and
out-of-range numbers name their key. The file is capped at 64 KiB. A
bad file at daemon start runs on the built-in defaults and reports
the error; `--check` and `msg reload` exit 1 on it. A bad value in the
`[effects]`, `[card]` or `[title]` tables is the one exception: `--check`
prints it on stderr and still exits 0, and the daemon ignores it. Every
key lives in [Configure](./configure.md).

```sh
scootnotify daemon --check --config ./notify.toml
# ./notify.toml: valid
```

## Ask the running daemon

`msg` talks to the daemon over its control socket, one request per
invocation. `dismiss` and `invoke` print nothing on success;
`list`, `reload` and `version` print the reply; `subscribe` prints the
`subscribed` reply and then one JSON event per line until the daemon
closes the connection.

```sh
scootnotify msg list
# 3 [normal] Firefox: Download complete
scootnotify msg list --json
# {"type":"notifications","notifications":[{...}]}
scootnotify msg history
# * [normal] Mail: New mail
scootnotify msg history --json
# {"type":"history","history":[{...}]}
scootnotify msg dnd status
# {"type":"dnd","dnd":"off"}
scootnotify msg dnd on
scootnotify msg dnd toggle
scootnotify msg restore
scootnotify msg dismiss 3
scootnotify msg dismiss --all
scootnotify msg invoke 3            # the "default" action
scootnotify msg invoke 3 open
scootnotify msg reload              # the file and the look layer
scootnotify msg reload --look       # only the look layer
scootnotify msg subscribe notify close dnd
scootnotify msg version
# {"type":"version","protocol":2,"version":"0.3.1"}
```

Unknown ids and unknown actions are loud errors, never silent
successes. `msg dismiss` takes an id or `--all`, never both; over the
raw socket a `dismiss` carrying both dismisses that id. `msg dnd` with
no argument reads the mode; `msg restore` re-shows the newest history
entry as a new card even under DND, and refuses loudly on an empty
ring. `msg history` marks unseen entries with a `*` in human lines;
`--json` is the stable view.

```sh
scootnotify msg dismiss 999
# scootnotify: daemon: unknown notification id 999
# exit 1
```

`reload` validates whole and keeps the last good: a bad file keeps
the running config and cards, and the reply says why. `--look`
re-reads only the look layer and never touches the user file, so a
swap under a half-edited file still applies. A refused fragment (or
no layer at all) refuses the scoped reload with the running look
standing.

## Wait for one

`msg wait` blocks until a notification matches every filter given and
prints its JSON, or until `--timeout` (30 s when absent) and exits 1.
`--app` names the app exactly; `--summary-contains` is a substring of
the summary. Cards already live match at once; later ones match as
they arrive, with nothing missed between the baseline and the watch.

```sh
scootnotify msg wait --app Thunderbird --summary-contains "New mail" --timeout 60
# {"id":4,"app_name":"Thunderbird",...}
```

This is the computer-use primitive: an agent that started a download
blocks on its completion instead of polling screenshots.

## The control socket

One daemon per Wayland display:
`$XDG_RUNTIME_DIR/scootnotify-<display>.sock`, owner-only (`0600` from the
moment its name exists, whatever your umask; a crash in the instant of
start-up can leave an empty `.scootui-*` directory beside it, safe to
delete), with a `.lock` beside it. At most 16 connections (a 17th closes the
oldest non-subscriber) and 4 subscribers (a 5th is refused); a peer
that stops reading is dropped past 30 s. Requests are one JSON object
per line carrying the protocol they speak:

```sh
printf '{"protocol":2,"type":"list"}\n' | socat - UNIX-CONNECT:"$XDG_RUNTIME_DIR/scootnotify-$WAYLAND_DISPLAY.sock"
```

A request whose `protocol` is not `2` is refused with a mismatch
error, so a client and a daemon from different builds disagree loudly
rather than misread each other. Compatibility rides on that number,
never on the crate version.

## The JSON view

`list` and `notify` events carry this object per live card
(`--json` for the raw form):

```json
{
  "id": 3,
  "app_name": "Firefox",
  "desktop_entry": "firefox",
  "summary": "Download complete",
  "body": "plain text, markup stripped",
  "urgency": "normal",
  "category": "transfer.complete",
  "actions": [{"key": "open", "label": "Open"}],
  "icon": {"kind": "name", "value": "firefox"},
  "created": 1720000000000,
  "shown": 1720000000123,
  "expires": 1720000010123,
  "output": null
}
```

`urgency` is `low`, `normal` or `critical`. `icon` is absent where the
card shows none; its `kind` is `name`, `path` or `data`, and pixels
never cross the socket (for `data` the value is the size, such as
`"48x48"`). `shown` is null until first presented; `expires` is null
where the card never expires, and is recomputed from the running
clock on every read (hover and input-idle pauses push the real
deadline past it). `output` is the pinned output, or null where the
compositor chooses. New keys may be added; none is removed or changes
meaning within a protocol version.

Events on a subscribed connection:

```json
{"type": "notify", "notification": {"id": 3, "...": "..."}}
{"type": "close", "id": 3, "reason": "dismissed"}
{"type": "invoke", "id": 3, "action": "open"}
{"type": "dnd", "dnd": "on"}
```

The close reasons are `expired`, `dismissed`, `closed`
(`CloseNotification`) and `evicted` (the live cap). The `dnd` event
carries the mode after the change (`on` or `off`).

## History entries

`msg history --json` carries this object per ring entry, oldest
first:

```json
{
  "app_name": "Mail",
  "desktop_entry": "",
  "summary": "New mail",
  "body": "plain text, markup stripped",
  "urgency": "normal",
  "category": "email.arrived",
  "icon": {"kind": "name", "value": "Mail"},
  "created": 1720000000000,
  "seen": false
}
```

`seen` says whether the card was ever on screen; the unread count is
the entries with `seen: false`. There is no live id: the old one was
invalidated by its `NotificationClosed`. New keys may be added; none
is removed or changes meaning within a protocol version.

## Troubleshooting

> **Symptom:** `msg` says no daemon is running.

Nothing listens on the socket: start one with `scootnotify daemon`,
and check the display matches (`WAYLAND_DISPLAY` names the socket,
so a nested session and its host never share one).

> **Symptom:** the daemon exits 3 saying the name is owned.

Another notification daemon (mako, dunst, swaync) holds
`org.freedesktop.Notifications`, or its D-Bus activation file brings
it back on the next notification. Stop it first
(`systemctl --user stop mako`) and remove or mask its activation
file; scootnotify never takes the name from anyone.

> **Symptom:** a second daemon refuses, saying one is already running.

One daemon owns each display's socket and lock. Stop the running one
first; a crashed one's stale socket is recognized by its free lock
and replaced.

> **Symptom:** `msg reload` refuses the file.

The running config and cards stand; the reply names the bad key.
Validate with `daemon --check --config PATH`, fix, and reload again.
A half-edited file never half-applies: with `--look` it is not even
read.

> **Symptom:** turning DND off shows nothing.
>
> That is the design: held cards stay in history, and only `restore`
> brings one back on screen. The unread count is the entries never
> shown.

> **Symptom:** `msg restore` says the history is empty.
>
> The ring holds closed and held cards only; `transient` cards and
> cards a rule keeps out with `history = false` never enter. A restart
> empties the ring: it is in-memory by design.

> **Symptom:** `msg subscribe` ends with no error.

The daemon drops a subscriber it cannot write to (a full socket) and
closes it with nothing more. Resubscribe, then `list`: the baseline
covers what the stream may have missed. The same holds after a
`dropped` line, which is always last.

> **Symptom:** `msg wait` times out although the card is on screen.

The filters must all match: `--app` is exact (check `list` for the
sender's real app name), `--summary-contains` is a substring of the
plain-text summary.

> **Symptom:** notifications never expire.

A critical urgency never expires unless the sender passes a positive
`expire_timeout`, and `expire_timeout = 0` means never. Hover pauses
that card's clock, and input idling pauses them all. Check the
sender's urgency and timeout before the config: the defaults only
apply to `expire_timeout = -1` (low 5 s, normal 10 s), counted from
first presentation.

> **Symptom:** an app's cards stop appearing under load.

The sender tripped its rate limit (20 per second, burst 40): its
`Notify` calls still get ids, but the cards count unshown instead of
showing. A `replaces_id` update never trips it. Wait a second and
retry, or pace the sender.

> **Symptom:** no icon on the card.

`image-data` must be 8-bit, 3 or 4 channels agreeing with its alpha
flag, sides 1 to 2048, with pixels covering every row — anything
else means no image. `image-path` and `app_icon` take a `file://`
URI, an absolute path, or a theme name; every other scheme means no
image. A theme name draws the app's initial as a letter tile in v1.
