---
title: "Desktop power: profiles, lid/battery policy, charge limit"
status: "open"
area: "packaging"
priority: "medium"
blocked: null
---

# Desktop power: profiles, lid/battery policy, charge limit

Filed 2026-10-04, child 10 of `desktop-paved-path`. Serves
**daily-drive** (battery life, lid behavior, and charge limits are the
laptop contract) — the M2's hand-wired charge-limit service is the
reference to obsolete.

## The gap

scootbar has `battery`/`power` *display* modules only. The flake has no
`power-profiles-daemon`, no logind lid-switch / low-battery wiring, and no
charge-limit option. The maintainer's Asahi M2 carries a hand-wired
charge-limit service and lid/suspend behavior in local config — exactly the
work this child deletes.

## What to do

Fill the `desktop.power` slot:

- `power-profiles-daemon` as the system service + a bar/CLI-visible profile
  switch (performance/balanced/power-saver; wire to the bar's `power`
  module or a keybind — decide, say which).
- Lid and low-battery policy via logind (`HandleLidSwitch`,
  `HandleLidSwitchDocked` vs undocked rule — state the exact rule for the
  M2 + external monitor case; low-battery suspend threshold), coordinated
  with the idle/lock child (lock-before-sleep ordering — say which unit
  orders after which).
- Charge-limit option where the hardware has it
  (`desktop.power.chargeLimit`, e.g. the Apple-silicon sysfs node the M2's
  hand-wired service uses — read the maintainer's local config as the spec;
  absent hardware = option refused loudly at eval or silently inert — decide
  and pin it).
- Edge cases: AC/battery profile auto-switch; suspend with an SSH session
  sharing the user manager; docked clamshell (lid closed, external output —
  must not suspend); hibernate vs suspend (say which is supported and why).

Acceptance: eval pins in `nix/tests.nix`; real-login proof on the M2 (lid
close/open, low-battery threshold, charge limit holding at the set
percentage, profile switch changing the daemon state); docs in
`docs/nix.md`.

## Not in this ticket

Thermals/fan curves; TLP-style deep tunables beyond profiles + limit;
auto-suspend on metered idle past the idle child's timeouts (that child owns
idle timing — coordinate, don't duplicate).
