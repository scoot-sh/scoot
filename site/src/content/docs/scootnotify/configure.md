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
look = true
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
- `look`: whether the look layer themes the cards (see [The look
  layer](#the-look-layer)). Type: boolean. Default: `true`. Example:
  `look = false`. Reload: `msg reload`.

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
> entry seen; the count is live cards plus unseen history.

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

## The bar feed

On every change of DND or of the unread count — an arrival, a
dismissal, an expiry, a restore, a DND toggle — and only then, the
daemon sends `set notifications <payload>` to the bar's control
socket. The bar needs a `notifications` push module placed (see
[Button, push and exec modules](../scootbar/modules.md#button-push-and-exec-modules));
without one the daemon logs guidance once per run and leaves the bar
alone. A bar started later shows its placeholder until the owed send
lands when its socket appears.

The payload is `{text, class, tooltip, icon?}`: `DND n` (or `DND`)
with class `muted` while DND is on, the count `n` with class `urgent`
(any counted card is critical) or `normal` otherwise, and `{text:
""}` when idle. The count is live cards plus unseen history.

- `bar_icon_unread`: the icon sent beside the unread count (a
  per-update icon, so it overrides the module's static icon while
  set). Type: empty or exactly one character. Default: `` (no icon;
  the key stays out of the payload). Example: `bar_icon_unread =
  "●"`. Reload: `msg reload`.
- `bar_icon_dnd`: the icon sent while DND is on. Type: empty or
  exactly one character. Default: `` (no icon; the key stays out of
  the payload). Example: `bar_icon_dnd = "☾"`. Reload: `msg reload`.

> **Symptom:** the bar never shows a count.
> The module is not placed, or the bar is not running: the daemon
> says which on its stderr, once for the module, once per failed
> change for the bar. A running bar that still shows its placeholder
> is waiting for the next change — unless a send is owed, which lands when the bar's socket appears.

> **Symptom:** the count survives dismissing everything.
> Turning DND off shows nothing retroactively, but it reshapes the bar
> at once (`DND n` becomes `n`). Each `restore` marks one entry seen;
> the count is live cards plus unseen history.

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

Colors, font, opacity, corners, depth and motion come from the active look, under the
cards — never from this file. The daemon reads
`<state>/look/notify.toml` at start (a dangling or missing link means
no layer, plus a warning), with the user's file winning per key over
the look over the built-ins. The thirteen color roles map to the
cards.

To keep the built-in colors whatever look is active, set `look =
false` in the file. The layer is then not read at start, a plain `msg
reload` applies the built-ins, and `msg reload --look` answers ok and
changes nothing, so a look swap does not report a failure. Set it
back to `true` and `msg reload` to re-read the layer. The switch is
all or nothing: the layer carries colors, font size, opacity and
the icon theme, the file carries geometry, timeouts and rules, and
to change one value of the look, override it in the look's
`[apps.scootnotify]` table. With the Nix profile, `look = false` is
what [`theme.targets.notifications.enable =
false`](../../desktop/notifications/) writes.

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
| Corners | the file's `radius` (see the note below), or `card.shape` |
| Shadow, fill | `effects.shadow`, `effects.fill` (see [Depth](#depth)) |
| Motion | `effects.motion`, `effects.motion-duration-ms`, `effects.motion-bounce`, and `card.enter`, `card.exit` (see [Motion](#motion)) |
| Accent marker, outline, per-app color | `card.mark`, `card.shape`, `card.app-accent` (see [Card styles](#card-styles)) |
| Summary weight and spacing | `title.weight`, `title.spacing` (see [Title weight and spacing](#title-weight-and-spacing)) |

The corner radius is the file's `radius` (see [Limits](#limits)),
not the look's `shape.radius`: the layer carries it, but the cards do not
read it yet, so a look cannot round the cards except through `card.shape`.

### Depth

A look can give the cards a shadow and a gradient fill. Both are off by
default, and a look that sets neither paints exactly what it did
before. Set them in `look.toml`:

```toml
[effects]
shadow = "soft"      # none | soft | deep | glow
fill = "gradient"    # solid | gradient
```

To override one for the notifier alone, put the key in the look's
`[apps.scootnotify]` table (`shadow = "glow"`). The look renders them as
top-level `shadow` and `fill` keys in `notify.toml`. A value outside the
lists is refused by `scoot look validate`; a hand-edited
`notify.toml` with one is reported in the daemon log and ignored, and the
card keeps its default.

| `shadow` | What it draws | Room it needs |
|---|---|---|
| `none` | Nothing. The default; builds and paints nothing. | none |
| `soft` | A low shadow: 4 px down, 12 px blur, 36% black. | 8 px above, 12 px at the sides, 16 px below |
| `deep` | A heavy shadow: 10 px down, 28 px blur, 55% black. | 18 px above, 28 px at the sides, 38 px below |
| `glow` | A 16 px halo, centered, in the card's border color (`border`, `focus` or `urgent` by urgency), at 51%. | 16 px on every side |

| `fill` | What it draws |
|---|---|
| `solid` | The `surface` color, flat. The default. |
| `gradient` | Light at the top. On a dark `surface` the top edge is lightened about 10% toward white and fades to `surface` at the bottom; on a light one the bottom is darkened about 8%. The card keeps the look's `opacity.popups`. |

The shadow's room comes out of `margin_px`: with the default 12 px
margin and `soft`, a top-right stack sits 12 px from the screen edge, as
it does without a shadow (a stack anchored to the bottom needs 16 px). A margin smaller than the room moves the cards that
much further in, since a surface cannot start off screen. The shadow
falls outside the cards only (a translucent card does not show its own
shadow through its body), is never clickable (the gaps and the room
around the stack click through), and is painted when the stack repaints,
not while it is idle.

`soft` and `deep` are black, so they show over light and mid-tone
backgrounds and nearly vanish over a near-black wallpaper; `glow` reads on
any background.

`scootnotify` and `scootlaunch` read these tokens (see the launcher's
[depth](../scootlaunch/panel.md#depth)); `scootbar` ignores `[effects]`,
and an `[apps.scootbar]` `shadow` key refuses at validation.

> **Symptom:** a look sets `shadow` and the cards have none.
> The daemon logs `shadow refused ... default applies` or `shadow is not
> a shadow style` for a value outside `none`, `soft`, `deep` and `glow`.
> Check `<state>/look/notify.toml` for a `shadow` line, and that the
> config has no `look = false`.

> **Symptom:** the cards sit further from the screen edge than
> `margin_px`.
> The shadow needs room that comes out of the margin. With `margin_px`
> below the room (for a top-right stack, 12 px for `soft` and 28 px for
> `deep`), the cards sit that much further in. Raise `margin_px`, or pick a lighter `shadow`.

### Card styles

A look can mark each card with an accent, give the cards a pill outline
and color each app's cards differently. All three are off by default, and
a look that sets none paints what it did before. One layout fix
applies to every look: a progress bar or buttons on a card with an icon now
start below the icon, not through it, so such a card (a volume OSD, say) is
up to 24 px taller.
They are notifier keys, in the look's `[apps.scootnotify]` table, quoted:

```toml
[apps.scootnotify]
"card.mark" = "stripe"      # none | stripe | dot | badge
"card.shape" = "pill"       # rounded | pill
"card.app-accent" = true    # true | false
```

The look renders them into `notify.toml` as a `[card]` table with
`mark`, `shape` and `app-accent`, only when the look sets them. A value
outside its list is refused by `scoot look validate`; a hand-edited
`notify.toml` with one is reported in the daemon log and ignored, and the
key keeps its default.

| `card.mark` | What it draws |
|---|---|
| `none` | No marker. The default. |
| `stripe` | A 3 px bar down the card's leading (left) edge, just inside the border, cut by the card's rounded corners. Takes no room. |
| `dot` | An 8 px dot before the app name on the `app · age` line. The text moves 14 px right to make room; the card does not grow. |
| `badge` | The icon slot on a tinted tile. A card with no icon gets the slot too, showing the app's initial, so every card is 48 px tall at least and its text starts 58 px in. |

The marker's color is the look's `accent`. With `card.app-accent` on it
is the app's own color instead (below).

`card.app-accent` gives every app a color hashed from its name: the same
app is the same color on every run and every machine, and `Firefox` and
`firefox` share one. The hue is spread around the wheel at medium
saturation and lightness, then darkened (on a light-on-dark look) or
lightened (on a dark-on-light one) until the look's `foreground` reaches
a 4.5:1 contrast ratio on it, WCAG AA for normal text. That is what makes
the letter on a `badge` tile readable; on a dark look it also means
accents are deep rather than bright. An app with no name has no accent and
uses the look's `accent`. The color is worked out when the card is laid
out, not on every repaint. `card.app-accent` with `card.mark = "none"`
draws nothing.

| `card.shape` | What it draws |
|---|---|
| `rounded` | The corner radius from the config file. The default. |
| `pill` | Ends rounded to half the card's height, to at most a 36 px radius: a one- or two-line card is a true pill, a tall one keeps round ends of that size. The content moves 8 px further in on each side so text, the progress bar and the buttons stay clear of the curve. The shadow follows the shape. |

A `stripe` on a `pill` shows only where the curved end is nearly
straight, a short sliver at its middle; `dot` and `badge` suit a pill
better.

A card that is moving carries its marker with it, and the marker, the
tile and the pill are all inside the card's shadow and clipped by its
shape.

The click area is still each card's rectangle, as with rounded corners: a
click in the cut corner of a pill invokes or dismisses the card instead of
reaching the window behind it.

> **Symptom:** a look sets `card.mark` and the cards have none.
> The daemon logs `card.mark refused ... default applies` or
> `card.mark is not a card mark` for a value outside `none`, `stripe`,
> `dot` and `badge`. Check `<state>/look/notify.toml` for a `[card]`
> table, and that the config has no `look = false`.

> **Symptom:** every app's marker is the same color.
> `card.app-accent` is off, or the app's name is empty (`msg list --json`
> shows the `app_name` the sender gave).

> **Symptom:** the dot or the badge crowds the text.
> Both reserve room at layout, so a look swap needs a daemon that
> re-laid out the cards: `scoot look apply` does, and plain `msg reload
> --look` does too.

### Title weight and spacing

The summary (the first, bold line of a card) can be set apart from the
body. `title.weight` and `title.spacing` apply to the summary only; the
`app · age` line and the body keep their weight, and bold inside a body
keeps drawing as it always did.

```toml
[apps.scootnotify]
"title.weight" = 800    # 100 to 900
"title.spacing" = 1     # -2 to 8, pixels after each character
```

The look renders them into `notify.toml` as a `[title]` table with
`weight` and `spacing`, only when the look sets them.

| `title.weight` | What it draws |
|---|---|
| 100 to 500 | The summary in the regular weight, like the body. |
| 501 to 700 | Bold, as the summary always was. The default is 700. |
| 701 to 900 | Heavier: one more strike, so the strokes are a pixel thicker again. |

scootnotify has no fontconfig and no variable-font support: it draws with
one font file (`--font PATH`, else the first well-known file) and makes
bold by drawing a run again 1 px to the right. So a weight is three steps
and not nine, and it does not pick another font file. `font` in the look
is validated and still does not choose the file.

`title.spacing` adds that many logical pixels after every character of the
summary (negative tightens). The summary wraps and ellipsizes with the
spacing included, so a wider summary breaks earlier instead of running out
of the card.

> **Symptom:** `title.weight` makes no visible change between 600 and 700.
> They are the same step. The steps start at 501 and 701.

### Motion

By default cards appear and disappear at once and the stack jumps when a
card closes. A look can make them move: a new card slides, fades, grows
or drops in, a closed card fades out, and the cards beside it spring into
the space it left. Motion is off by default, and a look that does not set
it paints and behaves exactly as before.

```toml
[effects]
motion = "spring"          # none | ease-out | spring
motion-duration-ms = 380   # 0 to 1000
motion-bounce = 0.6        # 0 to 1, spring only

[apps.scootnotify]
"card.enter" = "slide"     # slide | fade | scale | drop
"card.exit" = "fade"       # slide | fade | scale | drop
```

| Key | Value | Default | What it does |
|---|---|---|---|
| `motion` | `none`, `ease-out` or `spring` | `none` | The curve every move follows. `none` moves nothing and costs nothing. |
| `motion-duration-ms` | whole number, 0 to 1000 | `250` | How long one move takes. `0` is the same as `none`. |
| `motion-bounce` | number, 0 to 1 | `1` | How far a `spring` overshoots its target: `0` is the `ease-out` curve, `1` is the compositor's own spring (about 9% past the target before it settles). Ignored by `ease-out`. |
| `card.enter` | `slide`, `fade`, `scale` or `drop` | `slide` | How a new card arrives. |
| `card.exit` | `slide`, `fade`, `scale` or `drop` | `fade` | How a closed or expired card leaves. |

Every style fades the card; the style adds a move. `slide` comes in from
the edge the stack is anchored to (40 px), `scale` grows from 88% of its
size, `drop` falls 24 px from above. A card leaving runs its style
backwards, always without overshoot, whatever `motion` is.

`motion`, `motion-duration-ms` and `motion-bounce` go in `[effects]` to
apply to every app that reads them, or in `[apps.scootnotify]` to
override them for the notifier alone. `card.enter` and `card.exit` are
notifier keys: they go in `[apps.scootnotify]`, quoted. The look renders
them into `notify.toml` as `motion`, `motion-duration-ms` and
`motion-bounce` at the top level and `enter` and `exit` in a `[card]`
table, only when the look sets them. A value outside its list or range is
refused by `scoot look validate`; a hand-edited `notify.toml` with one is
reported in the daemon log and ignored, and that key keeps its default.

Cards move on the compositor's frame callbacks, so the motion is as
smooth as the display and costs nothing while the stack sits still: no
timer runs and no frame is requested once the last card lands. A close
only changes what is drawn: the notification is closed, and its
`NotificationClosed` signal sent, at the moment of the close, not when the
card has faded out. A card that is leaving cannot be clicked. A card that
is moving is clicked where it is drawn, not where it is going.

While a card travels, the surface stays as tall as the motion needs
(up to the distance a card moves, plus a little for a spring's overshoot)
and returns to the stack's height when it lands. A spring settling toward
the screen edge the stack is anchored to stops at the edge instead of
overshooting off it. Cards that were not moving paint exactly as they do
without motion.

The `radial-burst` look ships with a glow shadow, a spring and a scale entrance;
`ginger-night`, the default look, sets no motion. `ginger-night` also draws
the `stripe` marker; `radial-burst` the `badge` marker with per-app color, the
`pill` shape and a heavier, spaced summary.

There is no reduced-motion setting to follow: scoot has none yet, and the
compositor's `[animation] enabled` is a separate opt-in for window
slides. To turn card motion off, set `motion = "none"` in the look or in
`[apps.scootnotify]`, then `scoot look apply` (or `scootnotify msg reload
--look`).

> **Symptom:** a look sets `motion` and the cards still jump.
> The daemon logs `motion refused ... default applies` or `motion is not
> a motion` for a value outside `none`, `ease-out` and `spring`.
> Check `<state>/look/notify.toml` for a `motion` line, that
> `motion-duration-ms` is not `0`, and that the config has no `look =
> false`.

> **Symptom:** a card slides in with no overshoot.
> `motion-bounce` only shapes a `spring`. With `ease-out`, or a bounce of
> `0`, the card lands without passing its place.

> **Symptom:** the stack stutters while cards move.
> Motion repaints at the compositor's frame rate, so a compositor that
> serves few frames (a software-rendered session over a slow link) shows
> few. A shorter `motion-duration-ms` or `motion = "ease-out"` finishes
> sooner; `motion = "none"` moves nothing.

> **Symptom:** the cards ignore a look swap.
> `scoot look apply` reloads the running daemon live (plain `msg
> reload` re-reads the file too). If the look changed but
> the cards did not, the fragment refused: the daemon warns per key and
> keeps the last good look.

> **Symptom:** the cards keep the built-in colors under a look.
> Check the file for `look = false`, which the Nix option
> `theme.targets.notifications.enable = false` writes. With it set
> the daemon logs `look = false in the config: the look layer is not
> applied` on each `scoot look apply`.

> **Symptom:** no icon on the card.
> PNG `image-data` and reachable PNG paths draw; a theme name draws
> the app's initial as a letter tile in v1, and anything refused (a
> FIFO, an absurd size, a missing file) is a tile too, never an error.
