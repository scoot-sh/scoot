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

## Resolution (PR #TBD)

Landed as `programs.scoot.desktop.power`, opt-in (never with the
profile — lid-close suspend and the 80% cap are behavior changes, not
defaults), in `nix/modules/desktop.nix` (options), `nixos.nix`
(system half), `power-home.nix` (CLI + bar fill unit),
`power-charge.nix` (the desk-aware script), `keys-home.nix` (the
`Super+p` switch), `nix/tests.nix` (eval pins + behavior tests) and
the site's desktop Power section (docs in the site tree, not the
`docs/nix.md` stub — user reference moved there in PR #447).

- Profiles: PPD as the system service; the switch is the keybind
  (`Super+p` through `scoot-power-profile`, plus `powerprofilesctl`
  directly), NOT the bar's `power` module — that module is a logind
  suspend/reboot popup, while profiles live on PPD's bus, and a bar
  module speaking it would be Rust work outside this child. On the
  M2 the daemon is driverless-inert by design (no `platform_profile`,
  no EPP, `apple-cpufreq`/`schedutil` — measured over SSH; it still
  runs and owns the bus name for widgets), never refused (eval
  cannot see hardware). Opt-in `profileOnAC`/`profileOnBattery`
  auto-switch via udev (null holds, PPD's own behavior).
- Lid/low-battery: lid `suspend`, docked `lock` (twin of the idle
  child's rule — a docked clamshell never suspends), external-power
  `suspend`, power key `suspend`, low battery `Suspend` at 2%
  through UPower (not the `HybridSleep` default: s2idle-only, zram
  swap — it would fail instead of sleeping). `KillUserProcesses`
  stays false (the M2 is driven over SSH). Lock-before-sleep: the
  idle child's swayidle `before-sleep` (with `-w`) runs the locker
  through logind's delay inhibitor before
  `systemd-suspend.service` — this child adds no bypassing path.
  Hibernate is not wired (no persistent swap, s2idle only).
- Charge limit: the M2's `charge.nix` state machine ported whole
  (80% default, full-once until unplug, trip after 30 min on
  battery, back after a day on the charger; `scoot-charge
  status|toggle|full-once|limit|sync|push`), parameterized
  (`limit`, `battery` with auto-detect, `fullAfter`,
  `tripEndsAfter`) and inert without the sysfs node (one line,
  exit 0 — refusing at eval would break shared configs across
  heterogeneous hardware). The bar button is two user lines
  (`push.charge` + toggle click) plus the shipped fill unit; text,
  not Nerd icons (the bar's default font has no battery glyph that
  reads — the envelope lesson); no `theme.targets.power` (no new
  themed surface: the button inherits bar colors).

Evidence: `nix build .#checks.aarch64-linux.scoot-modules` green on
the M2 (eval pins for every default/refusal, `osRealPower`
eval-config pin against nixpkgs' real modules, behavior tests
running the real scripts against stub PPD and fake sysfs);
`nix build .#checks.aarch64-darwin.scoot-modules` green on a Mac;
`nix build .#docs-site` green (llms/snippet/nix-parse gates over
the new section). Bug-bash finds: a test variable named `out`
clobbered nix's `$out` (empty `power-saver` file, no output —
renamed); the script's `capacity` line dropped the reference's
literal `%` (restored); the no-trip test read a never-written mode
file (implicit default — assert the default). Mutation proof: the
built script with `want=100` sed-mutated to `want=99` writes 99 on
`full-once`, which the threshold assertions catch.

Not provable remotely (needs the maintainer at the machine — the
M2's live threshold, logind config and services were never
touched, only read): physical lid close/open, docked-lid behavior
on the external monitor, a real low-battery trip, the cap holding
at a set percentage, and a profile switch changing daemon state.
See the PR body's maintainer checklist. No VM test: the repo has
no `nixosTest` pattern (closest is the `osRealPower` eval-config
pin); nixpkgs' own `nixosTests.power-profiles-daemon` exists but
needs KVM and was not run.
