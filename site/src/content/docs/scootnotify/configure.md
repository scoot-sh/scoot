---
title: Configure scootnotify
description: "Popup placement, timeouts, urgency, and the look layer."
---

How the cards look, where they land, and how long they stay. The file
is `$XDG_CONFIG_HOME/scoot/notify.toml`; every key is optional, and a
missing file is the built-in defaults. A typo'd key refuses the whole
file at start (the daemon runs on defaults and says why); `daemon
--check` refuses loudly instead:

```sh
scootnotify daemon --check --config ./notify.toml
```

```toml
width_px = 360
max_visible = 5
margin_px = 12
anchor = "top-right"
# output = "HDMI-1"
body_lines = 5
border_px = 2
radius = 8
pause_when_idle_secs = 60
timeout_low_ms = 5000
timeout_normal_ms = 10000
timeout_max_ms = 3600000
max_live = 128
rate_per_sec = 20
rate_burst = 40
history_max = 64
dnd_allows_critical = true
```

## Placement

To move the stack, set where it anchors and how far it stands off the
edges:

- `width_px`: the card width in logical pixels. Type: integer, 10 to
  1024. Default: `360`. Example: `width_px = 400`. Reload: `msg reload`.
- `margin_px`: the stack's distance from the screen edges in logical
  pixels. Type: integer, 0 to 128. Default: `12`. Example:
  `margin_px = 24`. Reload: `msg reload`.
- `anchor`: the screen corner. Type: `top-left`, `top-right`,
  `bottom-left` or `bottom-right`. Default: `top-right`. Example:
  `anchor = "bottom-right"`. Reload: `msg reload`.
- `output`: the output the stack shows on. Type: string (1 to 256
  printable bytes) or absent. Default: absent, a null output — the
  compositor's choice. Example: `output = "HDMI-1"`. Reload: `msg reload`.
  On hotplug the stack moves if its output left; with zero outputs the
  cards wait with their clocks stopped.

> **Symptom:** the stack sits on the wrong monitor.
> Set `output` to the name scoot reports for the monitor. A name with
> no output left falls back to the compositor's choice and warns once.

## Timeouts and urgency

To change how long cards stay, set the per-urgency defaults and the
clamp. The clock starts on the frame that first presents the card —
never at arrival — so a card held by the lock or by missing outputs
expires normally once shown. Cards already on screen keep expiring
through a manual lock.

- `timeout_low_ms`: low-urgency expiry for `expire_timeout = -1`, in
  milliseconds. Type: integer, at most `timeout_max_ms`. Default:
  `5000`. Example: `timeout_low_ms = 3000`. Reload: `msg reload`.
- `timeout_normal_ms`: normal-urgency expiry for `expire_timeout =
  -1`, in milliseconds. Type: integer, at most `timeout_max_ms`.
  Default: `10000`. Example: `timeout_normal_ms = 8000`. Reload:
  `msg reload`.
- `timeout_max_ms`: what a positive `expire_timeout` is honored up to,
  in milliseconds. Type: integer, 1 to 86400000. Default: `3600000`
  (one hour). Example: `timeout_max_ms = 600000`. Reload: `msg reload`.
- `pause_when_idle_secs`: seconds without input after which every
  expiry clock pauses (0 disables). Type: integer, 0 to 3600. Default:
  `60`. Example: `pause_when_idle_secs = 120`. Reload: `msg reload`. Only
  real input counts (a playing video does not), and an agent driving
  the session keeps cards flowing.

> **Symptom:** a card never expires.
> Critical cards never expire unless the client asked with a positive
> timeout. Hover and input idling pause the clock; the card goes when
> the pointer leaves and input returns.

## Limits

- `max_visible`: cards on screen at once; past it a "+N more" pill
  covers the rest. Type: integer, 1 to 16. Default: `5`. Example:
  `max_visible = 3`. Reload: `msg reload`.
- `body_lines`: body lines per card before the ellipsis. Type:
  integer, 1 to 16. Default: `5`. Example: `body_lines = 8`. Reload:
  `msg reload`.
- `border_px`: the urgency border width in logical pixels (0 draws
  none). Type: integer, 0 to 8. Default: `2`. Example: `border_px =
  3`. Reload: `msg reload`. Low draws `border`, normal `focus`, critical
  `urgent`.
- `radius`: the card corner radius in logical pixels. Type: integer, 0
  to 64. Default: `8`. Example: `radius = 12`. Reload: `msg reload`.
- `max_live`: live notifications at most; past it the oldest
  non-critical one closes with reason 4. Type: integer, 1 to 1024.
  Default: `128`. Example: `max_live = 64`. Reload: `msg reload`.
- `rate_per_sec`, `rate_burst`: the per-sender token bucket,
  notifications per second and burst. Type: integers, 1 to 1000.
  Defaults: `20`, `40`. Example: `rate_per_sec = 10`. Reload:
  `msg reload`. Past the bucket a `Notify` still gets its id, but the card
  goes unshown.
- `history_max`: history ring entries kept, oldest first. Type: integer,
  0 to 256 (0 keeps none). Default: `64`. Example: `history_max = 100`.
  Reload: `msg reload` (truncates to the new cap, newest kept).
- `dnd_allows_critical`: critical cards show while DND holds the rest.
  Type: boolean. Default: `true`. Example:
  `dnd_allows_critical = false`. Reload: `msg reload`.

## History

Every closed card lands in the in-memory ring (newest last, at most
`history_max`), and so does every card that skips the screen — held by
DND, hidden by a rule, or past the rate bucket. `transient`
notifications never enter. Entries keep app, summary, body, urgency,
category, time and an icon name or path — never pixels. The ring
survives a reload, never a restart. Read it with
`scootnotify msg history`, bring the newest back with
`scootnotify msg restore` (a new card with a new id, even under DND).

> **Symptom:** the unread count never drops.
> Turning DND off shows nothing retroactively. Each `restore` marks one
> entry seen; the count is the entries never shown on screen.

## Do not disturb

While DND is on, new cards skip the screen and go straight to history,
except critical ones (while `dnd_allows_critical` holds) and cards a
rule lets through (`dnd_bypass`). A held card's expiry never starts, so
it stays unread until seen. The mode survives a reload, never a
restart.

```sh
scootnotify msg dnd on       # hold everything but criticals
scootnotify msg dnd toggle   # the bar's click runs this
scootnotify msg dnd status   # or with no argument: on or off
```

## Rules

`[[rule]]` tables match cards and steer them, in file order. A rule
with no match key matches everything. Every matching rule applies;
for the scalar actions the later rule wins; every matching `exec`
runs, at most 4 children at once.

```toml
[[rule]]
app_name = "Mail"
category = "email."
set_urgency = "normal"
timeout_ms = 8000

[[rule]]
app_name = "Volume"
hide = true

[[rule]]
app_name = "Thunderbird"
exec = ["play", "/usr/share/sounds/new-mail.wav"]
```

Match keys (all optional, all must hold):

- `app_name`: exact app name. Type: string, 1 to 128 bytes.
  Example: `app_name = "Mail"`. Reload: `msg reload`.
- `desktop_entry`: exact desktop entry. Type: string, 1 to 128 bytes.
  Example: `desktop_entry = "firefox"`. Reload: `msg reload`.
- `category`: exact category, or a `class.` prefix when it ends with a
  dot. Type: string, 1 to 128 bytes. Example: `category = "device."`
  matches `device.added`. Reload: `msg reload`.
- `urgency`: `low`, `normal` or `critical`. Example:
  `urgency = "critical"`. Reload: `msg reload`.
- `summary_contains`: a plain substring of the summary, never a regex.
  Type: string, 1 to 128 bytes. Example:
  `summary_contains = "Download complete"`. Reload: `msg reload`.

Action keys:

- `set_urgency`: the card's urgency, over the hint's (the design's
  `urgency` action, renamed: one table cannot carry the match
  `urgency` and the action under the same name). Type: `low`, `normal`
  or `critical`. Example: `set_urgency = "low"`. Reload: `msg reload`.
- `timeout_ms`: expiry over the resolved one. Type: integer, 0 to
  86400000 (0 never expires). Example: `timeout_ms = 30000`. Reload:
  `msg reload`.
- `hide`: straight to history, never on screen. Type: boolean, default
  `false`. Example: `hide = true`. Reload: `msg reload`.
- `history`: `false` keeps the card out of history, shown or not. Type:
  boolean, default `true`. Example: `history = false`. Reload:
  `msg reload`.
- `output`: pins this card's output, over the file's. Type: string (1
  to 256 printable bytes). Example: `output = "HDMI-1"`. Reload:
  `msg reload`.
- `dnd_bypass`: shows through DND, like a critical card. Type: boolean,
  default `false`. Example: `dnd_bypass = true`. Reload:
  `msg reload`.
- `exec`: an argv run on arrival, never through a shell, with the
  card's fields in `SCOOTNOTIFY_APP`, `SCOOTNOTIFY_SUMMARY`,
  `SCOOTNOTIFY_BODY`, `SCOOTNOTIFY_URGENCY`, `SCOOTNOTIFY_CATEGORY`,
  `SCOOTNOTIFY_DESKTOP_ENTRY` and `SCOOTNOTIFY_ID`. At most 16 words,
  each 1 to 4096 bytes. A rule `exec` never fires for a `replaces_id`
  update. At most 4 run at once; past that the spawn refuses with a log
  line. Example: `exec = ["play", "ding.wav"]`. Reload: `msg reload`.
  A sound hook is the expected use: the daemon plays no sound itself.

At most 32 `[[rule]]` tables; a 33rd refuses the file, like any
out-of-range value.

> **Symptom:** a rule never fires.
> Match keys are exact (except `category` past a trailing dot, and
> `summary_contains`). Check `msg list --json` for the card's real
> `app_name` and `category`: senders often name themselves differently
> than the app you see.

> **Symptom:** the sound never plays.
> Check the daemon log for `cannot run`: the program must exist and
> run (a shell line needs `sh -c`, quoted by you). Past 4 running
> children the spawn refuses until one exits.

Every key above applies live: edit the file, then `scootnotify msg
reload`. A refused file keeps the running config and cards, and says
why. `msg reload --look` re-reads only the look layer, never touching
the file — a look swap under a half-edited file.

## The look layer

Colors, font, opacity and corners come from the active look, under the
cards — never from this file. The daemon reads
`<state>/look/notify.toml` at start (a dangling or missing link means
no layer, plus a warning), with the user's file winning per key over
the look over the built-ins. The thirteen color roles map to the
cards:

| Element | Role |
|---|---|
| Card fill | `surface` (default `background`), at `opacity.popups` |
| Summary, body | `foreground` |
| App name, age, "+N more" | `dim` |
| Border, normal | `focus` (default `accent`) |
| Border, low | `border` (default `dim`) |
| Border, critical | `urgent` |
| Progress bar | `selection` (default `accent`) |
| Action button outline / hovered fill | `focus` / `hover` |
| Link text (underlined, never clickable in v1) | `accent` |
| Font | `fonts.ui` at `fonts.popup-size` |
| Corners | `shape.radius` |

> **Symptom:** the cards ignore a look swap.
> The swap reaches the daemon through `scootnotify msg reload --look`
> (plain `msg reload` re-reads the file too). If the look changed but
> the cards did not, the fragment refused: the daemon warns per key and
> keeps the last good look.

> **Symptom:** no icon on the card.
> PNG `image-data` and reachable PNG paths draw; a theme name draws
> the app's initial as a letter tile in v1, and anything refused (a
> FIFO, an absurd size, a missing file) is a tile too, never an error.
