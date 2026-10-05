---
title: "Desktop idle policy and lock screen from the flake"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-04"
---

# Desktop idle policy and lock screen from the flake

Filed 2026-10-04, child 2 of `desktop-paved-path`. Serves
**daily-drive** (a laptop that never locks or never sleeps its panels is not
daily-drivable) — this is the M2's hand-wired swayidle (dim to 10% at 2 min, `wlopm --off`
at 5 min, no lock yet) made into a default with a lock added. The M2's
local config is the reference; the `docs/configuration.md` snippet
(lock 10 min, off 15 min) is only an example.

Was blocked on `fix/scoot-session-target-after-display`, merged as #431 (2026-10-04): the session
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

## Landed (PR #435, 2026-10-05)

Filled the `desktop.idle` / `desktop.idle.lock` slots: swayidle user
unit (`scoot-idle`, `WantedBy graphical-session.target`, `Restart`
retried like the bar's unit, `-w` so the sleep lock lands first),
`sway-audio-idle-inhibit` hold while media plays, swaylock locker
(smallest working closure at the pinned rev -- waylock 1.6.0 has no
substitute and its source build dies under zig; swaylock 155.8 MiB vs
hyprlock 210.9 vs gtklock 351.2), themed from the look with a
`theme.targets.lock.enable` opt-out (first target of the one
namespace), `HandleLidSwitchDocked = "lock"` on the NixOS side
(docked/multi-output only; undocked lid still suspends, owned by
`desktop-power`), the locker's PAM service, and the stable
`lock.command` (`loginctl lock-session`) the future `desktop-keys`
binds without renaming. Defaults with the profile: dim 2 min to 10%,
lock at 4 (before screens-off, so no unlocked frame shows on wake),
off at 5, lock-before-sleep via logind delay inhibitor plus a direct
locker spawn, one timeout set for AC and battery (dual sets need a
supervisor swayidle lacks; power policy is the power child's).

Decisions: dim is `brightnessctl` (the measured reference; the
night-light slot hasn't landed and output-power has no low state);
inhibit is audio-activity, not per-player MPRIS (covers browsers, no
polling); `before-sleep` runs the locker directly, not through
`lock.command` (swayidle's own man page: `-w` + `-f` is what guarantees
the lock before the inhibitor releases); no system sleep unit (a root
hook locking all sessions is useless with no per-session listener --
the user unit IS the logind path here).

Measured live on the M2 (scoot-test login, 15/30/45 s test timeouts,
no suspend, no lid per the safety rule): dim 107 -> 54
(`actual_brightness`, seat ACL grants the active login backlight
rights; EPERM over ssh), restore to 107 on activity; lock repeatedly
on schedule plus a SIGUSR1-forced firing; screens off (`dpms: Off`)
and on (`output-power all on` while locked); unlock by typing the
password through `scoot msg type` (locker exits, desktop screenshot);
re-arm by activity; `systemd-inhibit` shows swayidle's `delay` lock on
`sleep`. A dbus-monitor trace caught the `Lock` signal on the
graphical session (`_352222`) with the locker spawning behind it, so
the timeout, manual (`loginctl lock-session` from the session) and
sleep paths share the one listener; the docked-lid lock lands through
the same logind `Lock` (all sessions, per the man page) with no extra
wiring, closing it is maintainer-only. RSS: swayidle 3.7 MB,
inhibitor 7.3 MB, swaylock 6.1 MB locked. Skipped cleanly: power draw
(`Not charging` at the 80% desk limit, `pwr.sh` invalid), functional
audio-hold (unit runs, PipeWire present, nothing played), real
suspend/resume and lid-close (for the maintainer: `systemctl suspend`
from the lock with `journalctl _SYSTEMD_USER_UNIT=scoot-idle` proving
`before-sleep`, then lid-close on a docked box proving lock-not-suspend
via `HandleLidSwitchDocked`). Eval pins in `nix/tests.nix` (timeouts,
units, per-slot disable, refusals, real-NixOS logind/PAM pins); docs in
`docs/nix.md` ("Idle and lock", slot table with Type/Default columns)
with the `configuration.md` recipe retired to a pointer.
