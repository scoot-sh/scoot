---
title: "wlr-output-power-management-unstable-v1: turn screens off when idle"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-04"
---

# wlr-output-power-management-unstable-v1: turn screens off when idle

Filed 2026-10-04. Serves **daily-drive first, computer use second**: the
maintainer daily-drives scoot on an Asahi MacBook Air M2 (`--tty`, `eDP-1` +
`DP-1`) and an idle laptop keeps both screens lit until the lid closes,
because scoot has `ext-idle-notify-v1` and `ext-session-lock-v1` but no way
to switch an output off. Auto-suspend is not an option on that box (driven
over SSH). The standard answer is the same one `swayidle` + `wlopm` use
everywhere else; per CLAUDE.md, implement the standard protocol rather than
a bespoke one. The agent half: an IPC action so automation can drive the
same state (screenshots stay honest about it).

## The gap

`docs/protocols.md` advertises `ext-idle-notify-v1` (a daemon learns the
seat is idle) and `ext-session-lock-v1` (a locker blanks the screens) but no
`zwlr_output_power_manager_v1`: nothing turns the panels off. Verified
2026-10-04 on `origin/main` (`1a570ab`): `grep -ri output.power
crates/ docs/` is empty, and `docs/protocols.md`'s implemented table has no
power row. Smithay carries no helper for this protocol at the pinned fork
rev (`035d447`: no `output_power`/`OutputPower` under `src/wayland/`),
so like `wlr-gamma-control-v1` (`compositor/gamma_control.rs`) this is
hand-implemented against the `output_power_management` server bindings
`wayland-protocols-wlr 0.3.12` already carries (re-exported through
Smithay, which pins it with `server`).

## What to do

- `zwlr_output_power_manager_v1`: `get_output_power` per `wl_output`,
  `set_mode(off|on)` with `invalid_mode` on anything else, `mode` events to
  every object for that output (including when scoot itself changes the
  mode, e.g. over IPC), `failed` when the output goes away or the hardware
  refuses. No exclusivity transfer (the protocol has none): last writer
  wins. Unrestricted like every other global (`docs/protocols.md` trust
  note: an allow-list without security-context support would be theatre).
- Off means: no page flips, no render work, no frame callbacks for surfaces
  only on that output; pointer/keyboard keep working; input by itself never
  turns the screen back on (the idle daemon's `resume` does that). Session
  lock, hotplug and VT-switch-back must leave a sane state: a powered-off
  output counts as blanked for lock confirmation (dark is blank); a
  replugged monitor comes back on under a fresh id (ids are never reused,
  power state is keyed by id); reactivation re-applies the hardware state.
  Headless/nested track the mode honestly and skip the render work; only
  `--tty` touches hardware (DPMS connector property, CRTC-disable fallback).
- IPC: session-level `output-power ID|all on|off` (an `Action` would need a
  `scoot-core` counterpart, and every action is refused while locked --
  while the idle cycle is off-after-lock / on-at-resume, so this must work
  under lock like gamma does), plus `powered` in `outputs`. Both additive,
  no `PROTOCOL_VERSION` bump (new request tag; defaulted snapshot field).
- Tests: protocol object lifecycle and events through a real client,
  the IPC request, power state across lock and hotplug in headless.
- Live proof on the dev VM `--tty`: `wlopm` off/on, DRM state before/after,
  idle CPU and wakeups while off, `swayidle` with a short timeout.
- Docs: `docs/protocols.md` (inventory row + section), `docs/ipc.md`, and a
  short idle recipe in `docs/configuration.md` (`swayidle` + `wlopm` via
  `[autostart]`).

## Not in this ticket

- Output reconfiguration (mode/position/scale): deliberately refused, see
  `resolved/output-management-reconfiguration-done.md`.
- Auto-suspend / lid-switch handling: policy belongs to the idle daemon.
- Per-output power in `scootbar`: no shell demand.

## Resolution (2026-10-04)

Shipped as designed, with one shape change: the IPC half is a session-level
`output-power ID|all on|off` request, not an `action`. An `Action` would
have needed a `scoot-core` counterpart (every `scoot_ipc::Action` converts
into one) for state that is not layout, and every action is refused while
locked -- while the idle cycle is off-after-lock / on-at-resume, so it
must work under lock like gamma does. `outputs` carries `powered`; both
are additive, no `PROTOCOL_VERSION` bump.

- `compositor/output_power.rs` (new): the manager global, per-output
  control lists (no exclusivity -- last writer wins), the on/off store, and
  `State::set_output_powered` / `reapply_output_power`.
- Render loop skips powered-off outputs wholesale (no draw/present/frame
  callbacks); a pending lock confirms off them (dark is blank).
- `--tty` hardware: the connector DPMS property; re-applied after every
  hotplug reconfigure / VT reactivation. Deliberately no CRTC-disable
  fallback, although "What to do" above names one: disabling the CRTC
  behind Smithay's surface state would fight the next commit on both tiers
  (see `Tty::set_power`), while a driver without a DPMS property degrades
  to render-skipping alone, which is still the compositor-side power
  saving. Removal fails the output's objects; a replug starts on under a
  fresh id.
- Captures stay honest about power state on both paths, not just IPC: an
  IPC screenshot of a powered-off output is refused, and a parked
  `ext-image-copy-capture-v1` frame due on one fails with `unknown` (the
  client may retry once the screen is back on) rather than serving the
  stale framebuffer -- which, across a lock taken while off, would be the
  pre-lock desktop served while locked. Powering back on under lock resumes
  drawing locked frames first.
- 12 protocol/state tests (`output_power/tests.rs`), wire pins for the new
  request tag and `powered` field, CLI parsing tests.
- Docs: `docs/protocols.md` row + section, `docs/ipc.md` request + field,
  `docs/configuration.md` idle recipe (swayidle via an autostart-spawned
  script -- `spawn` splits on whitespace, so the daemon line cannot be an
  entry itself), README "What works" row.

Evidence: `cargo nextest run -p scoot -p scoot-ipc -p scootctl
-p scoot-core` 2425 passed; `clippy -p scoot --all-targets -D warnings`
clean; `fmt --check` clean; `scripts/smoke-test.sh` rc=0. Live on the dev
VM `--tty` (pixman/dumb, one `Virtual-1` output): `wlopm --off/--on`
drives DPMS `Off`/`On` (`/sys/class/drm/.../dpms`, `wlopm` with no args
reports `Virtual-1 off`); 300 pointer moves draw 28 flips on, 0 off;
sustained-move CPU 222 jiffies on vs 157 off over 12 s; `swayidle timeout
5` off + resume on IPC input; IPC `output-power`/`outputs`/screenshot
refusal all behave. A `chvt` away/back cycle showed no `PauseSession` in
this VM's console setup, so the re-apply path is reviewed, not live-proven;
hotplug remove/replug is covered headless. Full workspace nextest was not
run to completion on the VM (disk: the scootbar all-features test link
needs more than the 2.5 GB free) -- `scootbar` takes no `scoot_ipc` type
apart from `encode`/`SOCKET_ENV`/one `Action::Quit`, verified by grep;
CI covers the rest.
