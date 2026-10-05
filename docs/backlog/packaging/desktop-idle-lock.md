---
title: "Desktop idle policy and lock screen from the flake"
status: "open"
area: "packaging"
priority: "high"
blocked: "branch fix/scoot-session-target-after-display: the session target is reached before WAYLAND_DISPLAY is imported, so idle/lock user units would start too early and skip"
---

# Desktop idle policy and lock screen from the flake

Filed 2026-10-04, child 2 of `desktop-paved-path`. Serves
**daily-drive** (a laptop that never locks or never sleeps its panels is not
daily-drivable) — this is the M2's hand-wired swayidle (lock 10 min,
`wlopm --off` 5 min… actually dim 2 min, off 5 min) made into a default.

Blocked on branch `fix/scoot-session-target-after-display`: the session
target is reached before `WAYLAND_DISPLAY` is imported, so idle/lock user
units would start too early and skip. Implement against that branch's shape
once it lands; acceptance needs it merged.

## The gap

Protocols exist (`ext-idle-notify-v1` v2, `idle-inhibit-v1`,
`wlr-output-power-management-v1` — `docs/protocols.md:28-30`, #427); policy
is a hand-written swayidle script (`docs/configuration.md:816-847`) the flake
never installs or starts. No dim, no lock-before-sleep, no media inhibit.

## What to do

Fill the `desktop.idle` / `desktop.lock` slots from the profile child:

- Idle daemon `swayidle` (docs already standardize on it) as a user unit
  bound to `graphical-session.target`: timeouts for dim (via the night-light
  slot's mechanism or output-power low state — decide, say which), lock
  (`loginctl lock-session`-driven locker, not a bare `swaylock` timeout, so
  lid-close/manual-lock share the path), screens off via `wlopm`
  (`output-power` IPC exists for agents too).
- Locker over `ext-session-lock-v1` (candidates swaylock/waylock/gtklock/
  hyprlock — all work; pick the lightest well-maintained, record closure
  size and idle RSS, say why). Lock-before-sleep via
  `logind` (`sleep.target` → lock) + unlock resume.
- Idle-inhibit while media plays (inhibit interface exists; wire the
  player side, e.g. `playerctl`-visible inhibitors — decide in ticket).
- Native-replacement contract: option/binds shape must survive a future
  scootlock swap unchanged.
- Edge cases: lid closed on a docked multi-output box (lock, don't suspend
  blindly — say the rule); AC vs battery timeouts; what `resume` re-arms;
  locked reload refusing new spawns (already compositor behavior,
  `configuration.md:451-460`).

Acceptance: eval pins in `nix/tests.nix` (unit files, timeout values,
per-slot disable); real-login proof on the M2 (idle to lock, screens off/on
by real input, lid-close lock, suspend-resume lock); docs in `docs/nix.md`
+ the `configuration.md` snippet retired to point at the module.

## Not in this ticket

The OSD for lock state, the greeter (exists), biometric/PIN unlock methods.
