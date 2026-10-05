---
title: "graphical-session.target is reached before the session has a display"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-04"
---

# graphical-session.target is reached before the session has a display

Filed 2026-10-05. Serves **daily-drive** first (idle dim/screen-off never
runs: the session's own Home Manager `swayidle.service` is skipped every
login), and **computer use** second (any agent-driven unit ordered after
the session target reads the same empty environment).

## The gap

Found on the Asahi M2, 2026-10-04, in a real greetd login (user
`scoot-test`): Home Manager's `swayidle.service`
(`WantedBy`/`PartOf`/`After `graphical-session.target`,
`ConditionEnvironment=WAYLAND_DISPLAY`) is skipped every login —
`start condition unmet ... ConditionEnvironment=WAYLAND_DISPLAY was not
met` — and the same holds for the maintainer's own login, so idle
dim/screen-off never runs.

Cause, verified in the tree: `resources/systemd/user/scoot.service` is
`Type=simple` with `BindsTo=`/`Before=graphical-session.target`, so the
target becomes active the moment the service forks, long before
`resources/scoot-session` waits for IPC readiness and imports
`WAYLAND_DISPLAY`/`XDG_CURRENT_DESKTOP` (steps 3-4 of its header). Every
unit ordered after the target starts without a display. scootbar only
survives by retrying (`Restart=on-failure`, no `ConditionEnvironment`: a
skipped start is never retried).

## What to do

Gate `graphical-session.target` on the display import with the blessed
systemd shape (`systemd.special(7)`: a session target starts and stops
`graphical-session.target` with `BindsTo=`; leaf services ride `PartOf=`):

- new `resources/systemd/user/scoot-session.target` (`BindsTo=`
  `graphical-session.target`, `Wants=`/`After=`
  `graphical-session-pre.target`), started by the launcher *after* the
  scoped display import — never enabled, like the shutdown target;
- `scoot.service` drops `BindsTo=`/`Before=` `graphical-session.target`
  (which is what pulls the target in at fork time: `BindsTo=` activates
  the listed units, the sway wiki measures this) and takes
  `PartOf=scoot-session.target` instead — stop propagation only, no
  start pull — keeping `Wants=`/`After=`
  `graphical-session-pre.target`;
- `scoot-shutdown.target` gains `Conflicts=scoot-session.target`, so the
  launcher teardown stops the session target (and through `PartOf=` the
  compositor) together with the rest;
- the launcher refuse-or-heal check covers `scoot-session.target`
  beside the service (heal stops it too), the deadline/socket-count
  failures still leave nothing active (the target is only started past
  them), and `systemctl --user stop` / logout still stops everything
  (graphical stopping stops the session target through `BindsTo=`,
  which stops the service through `PartOf=`).

Why this shape and not the alternatives: niri gates the same target with
`Type=notify` (the compositor reports readiness itself), but scoot
deliberately stays off the bus — no `sd_notify` in the compositor, per
the launcher header — so the gate has to live in the launcher, after its
IPC readiness wait. sway (`sway-session.target`, started by
`exec systemctl --user start sway-session.target` after
`import-environment`), Hyprland (`hyprland-session.target`, same shape,
now mostly delegated to uwsm), and the recipe `docs/nix.md` already
gives manual sessions are all this target — this change promotes that
shim into the launcher flow, so the launcher-started and hand-rolled
routes converge on one mechanism. uwsm (a session manager owning units,
environment and target lifecycle itself) is a bigger machine than this
ticket needs.

## Not in this ticket

- `sd_notify`/`Type=notify` readiness in the compositor: the kept
  no-bus decision (see the launcher header) rules it out.
- `xdg-desktop-autostart.target` execution: separate ticket, as before.
- Changing the unwired route (`session.command` hand-rolled entries):
  they keep the documented manual `scoot-session.target` recipe, which
  is now the same mechanism under a launcher-owned unit file.

## Resolution (PR #TBD)

Landed as designed: new `resources/systemd/user/scoot-session.target`
(`BindsTo=graphical-session.target`), `scoot.service` on
`PartOf=scoot-session.target` with no graphical binding,
`scoot-shutdown.target` conflicting all three session targets, and the
launcher starting the session target past the scoped display import
(with the no-dbus-tool fallback fixed to import the manager half).
`nix/modules/nixos.nix` installs the new target;
`scripts/scoot-session-test.sh` (17 asserts) plus `nix/tests.nix` pins
cover the ordering, the deadline/stale/refusal paths and the
dbus-less import.

Evidence: stub harness 17/17 on Linux and macOS (fails on the old
launcher at T1: never starts any session target); `scoot-modules` nix
check green; `systemd-analyze verify` clean; `shellcheck -S warning`
clean on both shell files; a real user-manager run on the dev VM
(session target start pulls the graphical target with the display in
the manager, a `ConditionEnvironment` probe starts, quit stops all
four units and restores the env). On the Asahi M2 (greetd,
scoot-test): before, `swayidle.service` every login `inactive (dead),
ConditionEnvironment=WAYLAND_DISPLAY was not met`; after,
`swayidle.service active (running)`, a D-Bus-activated probe inherits
`WAYLAND_DISPLAY`/`XDG_CURRENT_DESKTOP`, and logout stops everything
with the greeter returning.
