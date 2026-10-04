---
title: "A power menu: lock, log out, suspend, reboot, shut down, with a confirm"
status: "open"
area: "scootbar"
priority: "medium"
blocked: null
milestone: "M6"
---

# A power menu: lock, log out, suspend, reboot, shut down, with a confirm

Filed 2026-10-03. Serves **daily-drive** (the maintainer's own scoot desktop
ends the bar with a ⏻ icon; they asked 2026-10-04: "Also should have a
logout button too", and chose a power menu module over a bare logout button)
and **computer use** (an agent needs safe, scriptable lock/logout/suspend/
reboot/shut-down rows with documented `invoke` semantics).

## The gap

No power menu exists. A bare `[button.power] on-click = { scoot = "quit" }`
already works today (`docs/scootbar/cli.md`, button module) but ends the
session, and every client's unsaved work, on one stray click: the project
treats that like data loss (`CLAUDE.md`). There is no lock row, no
suspend/reboot/shut-down path (no logind caller), no confirm step, and no
`CanSuspend`/`CanReboot`/`CanPowerOff` hiding.

## What to do

A `power` module (Cargo feature `power`, in `default`): one icon (`icon`,
default none; docs example uses MDI power U+F0425), no text by default. A
click opens a popup (existing popup machinery; buttons with `closes`) with
rows Lock, Log out, Suspend, Reboot, Shut down, each with an optional glyph
(`icon-lock`, `icon-logout`, … in #411's shape) and the label.

Confirm on the destructive rows (Log out, Suspend, Reboot, Shut down): first
click arms that row ("…? Click again", short window); second click on the
same row performs and closes; clicking another row or waiting disarms.
Closing disarms only through the window: the arm survives refills and
reopens (arming lengthens the row, which resizes the popup, which reopens
it — disarming on a fill makes the arm invisible, measured live), so a
reopen within the window shows the armed row with its explicit label;
a daemon-side close hook through ~10 close sites was evaluated and
rejected (borrow surgery on hot paths for hygiene; the two labeled clicks
plus the window keep the guard). Lock needs no confirm. Agent `invoke` (`scootbar msg invoke power
logout`) semantics decided and documented against "never defer a
user-facing harm": invoke requires explicit confirm (two-step or
`--confirm`-style argument), justified in `docs/scootbar/cli.md`.

Each row overridable with `lock-command`, `logout-command`,
`suspend-command`, `reboot-command`, `poweroff-command` (argv lists, never
through a shell) and hideable (`rows = [...]` plus per-row `false`;
documented). Lock: `lock-command`, no default, hidden unless configured.
Log out: scoot's `quit` request over its control socket on scoot (reuse the
button modules' scoot-request path); elsewhere `logout-command` or hidden.
Suspend/Reboot/Shut down: logind over the system bus via `dbus::link`
(`org.freedesktop.login1.Manager.Suspend/Reboot/PowerOff(interactive: true)`),
hiding rows whose `CanSuspend`/`CanReboot`/`CanPowerOff` answer `no`/`na`
(re-asked when the popup opens, not per frame). Polkit decides; a refused
call surfaces (stderr at least, plus popup row/tooltip if cheap), never
silent. Replies sender-checked like every other call (#397).

Idle cost: nothing while closed (lazy vs connect-at-start decided by
measurement, recorded here and in `cli.md`).

Safety while testing: never perform a real suspend/reboot/power-off/logout/
lock on any machine. Tests drive a scripted logind on a private
`dbus-daemon` (follow `crates/scootbar/src/dbus/testdaemon.rs` and the
bluetooth BlueZ scripting). Live runs override every `*-command` with a
recorder script and point the logind path at nothing real.

## Not in this ticket

Keyboard navigation of popup lists ([popup-list-keyboard](popup-list-keyboard.md)).
Themed/tray-adjacent power affordances. A default lock command.
