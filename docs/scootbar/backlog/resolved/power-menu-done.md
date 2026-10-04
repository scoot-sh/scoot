---
title: "A power menu: lock, log out, suspend, reboot, shut down, with a confirm"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M6"
resolved: "2026-10-04"
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
plus the window keep the guard). Lock needs no confirm. Agent `invoke`
(`scootbar msg invoke power logout`) follows the same two-step as a click
— decided and documented against "never defer a user-facing harm" in
`docs/scootbar/cli.md`: a single invoke never ends the session (the stray
single call is the same lost work), and `scoot msg action quit` stays the
direct path for an agent that means it.

Each row overridable with `lock-command`, `logout-command`,
`suspend-command`, `reboot-command`, `poweroff-command` (argv lists, never
through a shell) and hideable with `rows = [...]` (a subset; documented). Lock: `lock-command`, no default, hidden unless configured.
Log out: scoot's `quit` request over its control socket on scoot (reuse the
button modules' scoot-request path); elsewhere `logout-command` or hidden.
Suspend/Reboot/Shut down: logind over the system bus via `dbus::link`
(`org.freedesktop.login1.Manager.Suspend/Reboot/PowerOff(interactive: true)`),
hiding rows whose `CanSuspend`/`CanReboot`/`CanPowerOff` answer `no`/`na`
(re-asked when the popup opens, not per frame). Polkit decides; a refused
call surfaces (stderr at least, plus popup row/tooltip if cheap), never
silent. Replies sender-checked like every other call (#397).

Idle cost: nothing while closed. Lazy connect won by measurement: no bus
fd and no round trips until the popup first opens or an invoke needs
logind (zero sources before first use, unit-tested); afterwards one
connection stays, woken only by its own replies (zero wakeups in 60 s
idle on the Asahi run, logind path and override path alike).

Safety while testing: never perform a real suspend/reboot/power-off/logout/
lock on any machine. Tests drive a scripted logind on a private
`dbus-daemon` (follow `crates/scootbar/src/dbus/testdaemon.rs` and the
bluetooth BlueZ scripting). Live runs override every `*-command` with a
recorder script and point the logind path at nothing real.

## Not in this ticket

Keyboard navigation of popup lists ([popup-list-keyboard](popup-list-keyboard.md)).
Themed/tray-adjacent power affordances. A default lock command.

## Resolution (2026-10-04, PR #414, code commit `5ce16a556`)

Landed the `power` module behind the Cargo feature `power` (in `default`).
What the verification proves, item by item (dev VM aarch64 rustc 1.97.1
unless noted; every code fix below has a test that failed before it):

- **Confirm state machine** (`crates/scootbar/src/modules/power/mod.rs`):
  arm on first invoke, perform on second within 5 s, disarm by another
  row, by the timer (harness `wait`, never sleeps), and survival across
  refills (arming resizes the popup, which reopens it — disarming on a
  fill made the arm invisible, caught live). Tests:
  `destructive_rows_arm_first_and_perform_second` (fails-before proven
  by reverting perform to disarm-only), `arming_another_row_disarms_the_first`,
  `a_lock_click_disarms`, `the_arm_times_out`,
  `an_armed_row_survives_refills`.
- **Each action's call or command**: lock/logout/suspend/reboot/poweroff
  stage `Exec` (all overrides), logout without its command stages
  `Scoot(Quit)`, logind rows queue `Suspend`/`Reboot`/`PowerOff` with
  `interactive: true` (scripted logind asserts the `true`).
- **`Can*` hiding**: `no`/`na` hide, `yes`/`challenge`/malformed/error
  show (`can_answers_hide_no_and_na`, `challenge_shows_the_row`,
  `malformed_and_errored_can_answers_keep_the_row`).
- **Polkit refusal surfacing**: peer `AccessDenied` lands in `value`,
  tooltip and the popup's first line (`a_refused_call_surfaces`).
- **Invoke semantics**: same two-step as clicks (documented in
  `docs/scootbar/cli.md` against "never defer a user-facing harm");
  `NoArg`/`Unknown`/`Refused` naming keys; no-bus perform refused
  (`performing_without_a_bus_is_refused`).
- **Config refusals naming keys** (`power.rows`, `power.icon*`,
  `power.*-command`, `power.on-*`, unknown keys) and the `rows` subset.
- **No allocation per redraw on the popup path**: `format_args!` into the
  popup's reused buffers; refills deterministic
  (`a_popup_refill_is_deterministic`).
- **Idle**: zero sources before first use (lazy link;
  `nothing_is_polled_before_first_use`); 0 wakeups in 60 s idle live,
  override config and logind path alike.
- **Help**: `power` in every `modules!` arm (263 → 519, generated),
  `power_help!`, cli tests extended (sort order, registry-in-order).
- **Docs**: `docs/scootbar/cli.md` module row, `## Power` section, Icons
  intro + per-state table + Nerd Font example (rebased onto #411).

Live on the Asahi M2 (headless scoot at scale 1.5, light palette, every
command a recorder): popup screenshot, armed screenshot
(`Suspend? Click again`), recorder file holding `suspend` after the
confirmed click; `power-popup.png`/`power-armed.png` beside the
implementer report. Real logind answered all three `Can*` (rows shown).

Ratchet (Asahi release, clean builds): plus-minus `.text` +43,328 B,
file +0 (64 KiB quantum), loaded sections +47,961 B. The size row
regresses on `.text`. Idle wakeups, fds and RSS do not regress beyond
what one run resolves.

**Maintainer's ruling (2026-10-04, given in chat): the `.text` growth of
the power module is waived**, recorded in
[lightest.md](../lightest.md#m6-power-menu-measured-2026-10-04).
