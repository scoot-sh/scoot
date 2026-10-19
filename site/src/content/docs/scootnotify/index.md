---
title: scootnotify overview
description: "The notification daemon — the D-Bus server and its on-demand popup stack."
---

A notification daemon for scoot and any desktop with a session bus.
It owns `org.freedesktop.Notifications` and shows the cards on one
on-demand overlay surface: newest on top, at most five, past that a
"+N more" pill. The surface exists only while a card shows; an idle
daemon holds no surface and no pixels.

```sh
scootnotify daemon                        # own the name and serve until killed
scootnotify daemon --check                # validate the config file and exit
```

One binary, linking nothing beyond glibc and libgcc_s. Without a
compositor it stays headless: the bus still serves, and cards wait with
their clocks stopped. It answers every call, including oversized
ones: a call past the 4 MiB cap gets `LimitsExceeded`, never silence.

## What it does today

- Owns `org.freedesktop.Notifications` with `DO_NOT_QUEUE`, never
  replacing and never replaceable. A taken name refuses with the
  owner named, and exits 3.
- Answers `GetCapabilities` (`body`, `body-markup`, `actions`,
  `icon-static`), `Notify` (all eight arguments, bounded),
  `CloseNotification` (an unknown id gets the spec's empty error) and
  `GetServerInformation` (`scootnotify`, `scoot`, the version, `1.3`).
- Emits `NotificationClosed` with reasons 1 (expired), 2 (dismissed),
  3 (closed) and 4 (evicted). A pointer click emits `ActivationToken`
  first, then `ActionInvoked`.
- Shows one card per notification: the icon, the summary in bold, the
  app and age, the body wrapped to five lines, the progress bar from
  `value`, and up to four action buttons. A left click on the body
  invokes `default` (or dismisses without one); a button click invokes
  it; a right click dismisses; a middle click dismisses all.
- Starts each card's expiry clock on the frame callback that first
  presents it. A card arriving while locked or output-less waits with
  its clock stopped; a card already on screen keeps expiring through a
  manual lock. Hover pauses that card; sixty input-idle seconds pause
  them all.
- Hands out u32 ids (0 skipped, live ids skipped), reserved in blocks
  of 1024 in `$XDG_RUNTIME_DIR`, so a restart never reissues an id a
  client may still hold.
- Honors `replaces_id` for the same sender or the same app, in place
  with the expiry restarted. A stranger's id is a fresh card.
- Counts expiry from presentation with per-urgency defaults: low 5 s,
  normal 10 s, critical never. A positive timeout is honored up to one
  hour.
- Caps every field (`app_name` 128 B, summary 1 KiB, body 8 KiB, 16
  action pairs, 64 hints), parses the five markup tags into styled
  spans, validates `image-data` and downscales it to the icon size,
  and reads the documented hints (`urgency`, `category`,
  `desktop-entry`, `image-data`, `image-path`, `transient`,
  `resident`, `value`). Unknown hints are ignored, confused ones keep
  their defaults.
- Rate-limits each sender (20 per second, burst 40): past it a
  `Notify` still gets its id, but the card is counted unshown. At
  most 128 cards are live; past that the oldest non-critical one
  closes with reason 4 (the oldest card if all are critical).
- Reads `$XDG_CONFIG_HOME/scoot/notify.toml` (every key optional; the
  [configure page](./configure.md) lists them) and the look layer
  under it. A bad file at start runs on built-in defaults and reports
  the error; `--check` refuses loudly instead.

Every flag is in the [CLI reference](./cli.md).

## Spec conformance

Against version 1.3 of the Desktop Notifications Specification:

| Item | Status |
|---|---|
| Bus name, object, interface (§2) | Yes, with `DO_NOT_QUEUE` ownership |
| D-Bus activation | A service file ships in the tree; the unit and its install arrive with the Nix module |
| `GetCapabilities` (§9.1.1) | `body`, `body-markup`, `actions`, `icon-static` (not `persistence`) |
| `Notify` (§9.1.2) | All eight arguments, bounded |
| `CloseNotification` (§9.1.3) | Closes with reason 3; unknown ids get the empty error |
| `GetServerInformation` (§9.1.4) | `("scootnotify", "scoot", version, "1.3")` |
| `NotificationClosed` (§9.2.1) | Reasons 1, 2, 3, 4 |
| `ActionInvoked` (§9.2.2) | Clicks on buttons and on the body for `default`; `msg invoke` follows |
| `ActivationToken` (§9.2.3) | Pointer invocations, emitted before `ActionInvoked` |
| `replaces_id` (§9.1.2) | Same sender or same app, swapped in place |
| Ids (§9.1.2) | u32, 0 skipped, live skipped, reserved across restarts |
| `expire_timeout` (§9.1.2) | -1 the urgency default, 0 never, positive clamped to 1 h, counted from first presentation (a scoot-specific guarantee: elsewhere the clock may start while hidden) |
| Urgency (§7) | Low 5 s, normal 10 s, critical never (an explicit timeout is honored) |
| Actions (§2) | 16 pairs read, 4 shown as buttons; `default` never a button but the body invokes it; `resident` keeps the card after an invoke |
| Body markup (§4) | `b` emboldened, `i` upright, `u` underlined, `a` underlined accent text (never clickable in v1), `img` dropped with `alt` kept; the rest stripped with text kept; entities decoded |
| Icons and images (§5.1) | `image-data` over `image-path` over `app_icon`, plus the 1.1 spellings; theme names draw a letter tile in v1 |
| `image-data` (§5.2) | Validated, downscaled to 128 px at arrival |
| Hints (§8) | Urgency, category, `desktop-entry`, `image-data`/`image_data`, `image-path`/`image_path`, `transient`, `resident` and `value` read, `suppress-sound` accepted and ignored; sound, position and `action-icons` ignored; `x-` skipped unread |
| `transient` (§8) | Kept for history (never enters it) |
| Categories (§6) | Stored; rules and JSON consume them in the control ticket |
| `desktop-entry` (§8) | Stored; rules and JSON consume it in the control ticket |

Popups draw at scale 1 in v1: on a HiDPI output the compositor
upscales the buffer (right size, softer text). Arabic, Hebrew and
Devanagari draw wrong until shaping lands; Latin, Cyrillic, CJK and
monochrome-fallback emoji are acceptable. There is no sound: the
`sound` capability is not advertised.

The control socket, Do Not Disturb, rules, history and the bar feed
are later tickets and are not here yet.
