---
title: "A native WiFi list in a popup, where the dmenu picker stops being enough"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M6"
resolved: "2026-10-03"
---

# A native WiFi list in a popup

Filed 2026-10-02, from [popups](popups-done.md). Serves **daily-drive**
(the WiFi list, without a launcher installed).

## Resolution (2026-10-03, PR #403)

The list landed, opt-in as the volume popup is (`on-click = "popup"` on
the network module; the `menu-command` dmenu path is unchanged).
Reference: [cli.md](../../cli.md#network) and [cli.md](../../cli.md#popups).

- **Scrolling**: a wheel over the popup moves the content a row a notch
  (`Wheel` groups a frame's axis events as the bar's own scroll does),
  held to `Layout::max_scroll`; the popup keeps the size the compositor
  configures. A row too wide is cut with an ellipsis, as a window
  title's is (a stack buffer, so the draw allocates nothing).
- **Selecting a row closes the popup** (a `closes` flag on buttons; the
  volume popup's Mute keeps `false`) and runs `connect N`. `N` names what
  the list showed: the popup records the raw SSIDs in displayed order,
  and a scan that moved underneath is refused ("that network is no
  longer seen") rather than connected to the wrong network. `connect`
  spawns `connect-command` with the SSID as its last argument, never
  through a shell (SSIDs are attacker-controlled radio data: tested with
  shell metacharacters, controls and non-UTF-8). One connect at a time;
  a second while one runs is refused. With `show-ssid = false` the list
  stays closed, as the picker does.
- **Out of scope, as filed**: the password prompt (the command's own
  business), and keyboard navigation, filed as
  [popup-list-keyboard](../popup-list-keyboard.md).

### Evidence

Unit tests through the module harness (a scripted kernel): the list
with the associated selected, unnamed networks not rows, `connect`
spawning the command with the SSID as one argument, the attacker SSID
arriving as data (no `PWNED` file beside the recording), the stale scan
refused (proved: disabling the check fails the test), arg and command
validation, and one-connect-at-a-time. Pure popup tests: the `closes`
flag, scroll clamp and offset hit-testing, the wheel's frame grouping,
and the ellipsis cut; the warm-popup allocation test covers the new
paths (0 allocations). Full verification on the dev VM:
`cargo nextest run -p scootbar` (1232 passed, 4 skipped) and
`cargo test -p scootbar` (0 failed) with `SCOOTBAR_REQUIRE_SCOOT=1
SCOOTBAR_REQUIRE_SWAY=1 SCOOTBAR_REQUIRE_DBUS_DAEMON=1`, `cargo clippy
-p scootbar --all-targets -- -D warnings` across the feature matrix
(30 builds, all OK), `cargo fmt --check`, and `scripts/backlog check`
(only the 3 pre-existing problems). End to end on headless scoot: the
bar with the network module placed (`eth0`), click and `invoke`
`popup`/`connect`/`menu` refused cleanly with an empty scan (the dev VM
has no wireless interface and no AP tools, so a live list with rows
needs real hardware), with screenshots.

### Resource ratchet (dev VM, release builds, network placed, popup closed)

| Row | before (`origin/main`) | after (branch) |
| --- | --- | --- |
| stripped file | 2,101,984 B | 2,101,984 B (+0) |
| `.text` | 1,581,224 B | 1,589,544 B (+8,320, +0.5%) |
| loaded sections | 1,942,155 B | 1,949,759 B (+7,604, +0.4%) |
| idle RSS (60 s) | 4792 kB | 4796 kB (+4, noise) |
| idle wakeups (60 s) | 0 | 0 |
| fds / threads | 9 / 1 | 9 / 1 |

`ldd` still shows only libc, libm and libgcc_s; `Cargo.lock` unchanged.
Popup-open rows were not measured here (no scan on the VM); the shared
open-idle machinery is covered by the volume popup's tests.

Original entry, left as written:

## The gap

[Popups](resolved/popups-done.md) landed with the volume slider as their one
consumer, so the widget set has text, a slider and buttons, and a *list* is a
column of buttons (one `selected`). The network module's picker is still the
interim path: `network.menu-command` is fed the scan's SSIDs on stdin and does
the connecting itself. That is deliberate. A list of up to 32 networks needs
what the popup does not have yet, and building it without the consumer
would be guessing.

## What to do

- **Scrolling** in a popup taller than the output allows (a wheel over the
  popup, a clamp to the output's height from the positioner's constraint), and a
  row cut with an ellipsis, as the window title's is.
- **Selecting a row closes the popup** (a `closes` flag on a button), and runs
  an action on the network module that takes the choice: an SSID is text, and
  an action takes one whole number, so the action is `connect N` (the index
  into the scan the popup was opened with, checked against the scan as it is
  then), spawning a configured `connect-command` with the SSID as its last
  argument, never through a shell.
- **Opt-in**, as the volume popup is: `on-click = "popup"` on the network
  module, with `menu-command` left as it is, so the dmenu path does not regress.
- A password prompt is out of scope (the command does that).

## Not in this ticket

Keyboard navigation of a list (arrows, Enter), which the popup grab already
gives the popup the keyboard for: its own entry once a list exists.
