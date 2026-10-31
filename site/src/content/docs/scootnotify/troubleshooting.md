---
title: scootnotify troubleshooting
description: "Notification symptoms and their fixes — popups, fullscreen, DND, history."
---

No card, a card under fullscreen, soft text on a HiDPI screen, DND that
never clears, or history that never comes back. Each fix below names the command that proves it.
What the daemon does is in [Overview](./index.md); placement, timeouts
and rules are in [Configure](./configure.md); every command is in the
[CLI reference](./cli.md). The profile setup (units, bar module, mako
escape hatch) is in [Desktop notifications](../../desktop/notifications/);
the cross-app list is in the [Symptom index](../../troubleshooting/).

> **Symptom:** no popups at all.

Check the unit, then the bus name, then the queue:

```sh
systemctl --user status scootnotify
busctl --user list | grep -F Notifications
scootnotify msg list
scootnotify daemon --check
```

Nothing on the socket means no daemon for this display
(`WAYLAND_DISPLAY` names the socket, so a nested session and its host
never share one). Exit 3 at start means the name is taken — mako (or
dunst, swaync) still runs; stop it and remove its D-Bus activation
file or it comes back on the next notification. A bad config runs on
built-in defaults and says why; `--check` refuses loudly instead.
Cards that arrive while locked or with no outputs wait with their
clocks stopped. Past the caps a `Notify` still gets its id but the
card counts unshown: 20 per second with burst 40 per sender, at most
128 live (oldest non-critical closes with reason 4), `transient`
never enters history, and a `hide` rule sends straight to history.

> **Symptom:** popups vanish under fullscreen.

Popups use `layer=overlay`, so they stay above fullscreen windows.
With the mako escape hatch, something set `layer` back to `top` —
read the generated mako config, the first content line is the layer.
scootnotify has no layer knob to mis-set. See
[Desktop notifications](../../desktop/notifications/).

> **Symptom:** cards look soft on a HiDPI screen.

The stack follows the scale the compositor asks for:
`wp_fractional_scale_v1` (with `wp_viewporter`), else
`wl_surface.preferred_buffer_scale`, else the `wl_output.scale` of the
output it is on. Compare what it was told with what it drew:

```sh
WAYLAND_DEBUG=1 scootnotify daemon 2>&1 | grep -E 'preferred|create_buffer|set_destination'
```

`preferred_scale` is in 120ths (240 is 2x). The `create_buffer` size
should be the `set_destination` size times that scale. A buffer the same
size as the destination under a scale above 1 means the compositor sent
none of the three, and the stack has nothing to follow. Without
`wp_viewporter` the stack follows whole scales only, the fraction
rounded up. A pooled buffer of the right size is attached again without
being created again, so after a scale change read the last
`damage_buffer` for the size on screen.

> **Symptom:** DND is stuck on, or the unread count never drops.

Ask the daemon, then steer it:

```sh
scootnotify msg dnd status
scootnotify msg dnd off
scootnotify msg history
```

While DND is on, new cards skip the screen for history, except
criticals (while `dnd_allows_critical` holds, the default) and cards
a rule lets through with `dnd_bypass`. Turning DND off shows nothing
retroactively — only `restore` brings one back on screen. The unread
count is live cards plus unseen history; each `restore` marks one
entry seen.

> **Symptom:** `msg restore` says the history is empty, or a card never expires.

The ring holds closed and held cards only (newest last, at most
`history_max`, 64 by default): `transient` cards and cards a rule
keeps out with `history = false` never enter, and a restart empties
the ring — it is in memory by design. A card that never expires is
usually correct: critical never expires unless the sender passed a
positive timeout, and `expire_timeout = 0` means never. Hover pauses
that card's clock and input idling pauses them all; the defaults apply
only to `expire_timeout = -1` (low 5 s, normal 10 s), counted from
first presentation. Check the sender's urgency and timeout with
`msg list --json` before the config:

```sh
scootnotify msg list --json | jq '.notifications[0] | {urgency, expires}'
```
