---
title: scootnotify overview
description: "The notification daemon — this page covers the shipped D-Bus server and its model; popups, the control socket and history arrive in later tickets."
---

A notification daemon for scoot and any desktop with a session bus.
What ships today is the foundation every later piece stands on: the
`org.freedesktop.Notifications` server and its model, headless. Cards
on screen, the `msg` control socket, Do Not Disturb, history, rules
and the bar feed are later tickets and are not here yet.

```sh
scootnotify daemon                        # own the name and serve until killed
scootnotify daemon --check                # validate the config file and exit
```

One binary, linking nothing beyond glibc and libgcc_s, needing no
compositor connection. It answers every call, including oversized
ones: a call past the 4 MiB cap gets `LimitsExceeded`, never silence.

## What it does today

- Owns `org.freedesktop.Notifications` with `DO_NOT_QUEUE`, never
  replacing and never replaceable. A taken name refuses with the
  owner named, and exits 3.
- Answers `GetCapabilities` (`body`, `body-markup`, `actions`,
  `icon-static`), `Notify` (all eight arguments, bounded),
  `CloseNotification` (an unknown id gets the spec's empty error) and
  `GetServerInformation` (`scootnotify`, `scoot`, the version, `1.3`).
- Emits `NotificationClosed` with reasons 1 (expired), 3 (closed) and
  4 (evicted). Reason 2 (dismissed) and `ActionInvoked` arrive with
  the UI and `msg invoke`; `ActivationToken` with pointer input.
- Hands out u32 ids (0 skipped, live ids skipped), reserved in blocks
  of 1024 in `$XDG_RUNTIME_DIR`, so a restart never reissues an id a
  client may still hold.
- Honors `replaces_id` for the same sender or the same app, in place
  with the expiry restarted. A stranger's id is a fresh card.
- Starts the expiry clock at first presentation (at arrival while
  headless), with per-urgency defaults: low 5 s, normal 10 s, critical
  never. A positive timeout is honored up to one hour.
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
  closes with reason 4.
- Reads `$XDG_CONFIG_HOME/scoot/notify.toml` (every key optional). A
  bad file at start runs on built-in defaults and reports the error;
  `--check` refuses loudly instead.

Every flag is in the [CLI reference](./cli.md).

## Spec conformance

Against version 1.3 of the Desktop Notifications Specification:

| Item | Status |
|---|---|
| Bus name, object, interface (§2) | Yes, with `DO_NOT_QUEUE` ownership |
| D-Bus activation | A service file ships; the bus activates the unit |
| `GetCapabilities` (§9.1.1) | `body`, `body-markup`, `actions`, `icon-static` (not `persistence`) |
| `Notify` (§9.1.2) | All eight arguments, bounded |
| `CloseNotification` (§9.1.3) | Closes with reason 3; unknown ids get the empty error |
| `GetServerInformation` (§9.1.4) | `("scootnotify", "scoot", version, "1.3")` |
| `NotificationClosed` (§9.2.1) | Reasons 1, 3, 4 (2 with dismissal) |
| `ActionInvoked` (§9.2.2) | Kept for `msg invoke` |
| `ActivationToken` (§9.2.3) | Pointer invocations, with popups |
| `replaces_id` (§9.1.2) | Same sender or same app, swapped in place |
| Ids (§9.1.2) | u32, 0 skipped, live skipped, reserved across restarts |
| `expire_timeout` (§9.1.2) | -1 the urgency default, 0 never, positive clamped to 1 h, counted from presentation |
| Urgency (§7) | Low 5 s, normal 10 s, critical never (an explicit timeout is honored) |
| Actions (§2) | 16 pairs read; `default` invokes from the body |
| Body markup (§4) | `b i u a img`; the rest stripped with text kept; entities decoded |
| Icons and images (§5.1) | `image-data` over `image-path` over `app_icon`, plus the 1.1 spellings |
| `image-data` (§5.2) | Validated, downscaled to 128 px at arrival |
| Hints (§8) | The documented nine read; sound, position and `action-icons` ignored; `x-` skipped unread |
| `transient` (§8) | Kept for history (never enters it) |
| Categories (§6) | Passed through to rules and JSON |
| `desktop-entry` (§8) | Passed through to rules and JSON |

Arabic, Hebrew and Devanagari draw wrong until shaping lands (as
measured); Latin, Cyrillic, CJK and monochrome-fallback emoji are
acceptable. There is no sound: the `sound` capability is not
advertised, and a rule's `exec` hook plays one instead.
