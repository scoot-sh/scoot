---
title: "Desktop audio/brightness/media keys and an OSD"
status: "open"
area: "packaging"
priority: "medium"
blocked: null
---

# Desktop audio/brightness/media keys and an OSD

Filed 2026-10-04, child 7 of `desktop-paved-path`. Serves
**daily-drive** (volume/brightness keys that do nothing fail the laptop
test on day one).

## The gap

scootbar already ships `volume`, `brightness`, `media`, `microphone`
display modules (`crates/scootbar/src/modules/`), but the flake has no
default binds for the keys, no OSD, and no audio baseline
(pipewire/wireplumber appear nowhere in `nix/`). The defaults bind nothing
to `XF86Audio*` / `XF86MonBrightness*`.

## What to do

Fill the `desktop.audio` slot:

- Audio baseline: pipewire + wireplumber as the profile's system services
  (or the lighter `pipewire`-only shape — measure, say why; the M2's
  speakers-heal unit is prior art to fold in or reference).
- Default binds for volume up/down/mute, mic mute, brightness up/down,
  play/pause/next/prev — wired to `wpctl`/`brightnessctl`/`playerctl`
  (pick, record closures) with key names that exist on Apple + PC
  hardware (say which keycodes were verified).
- OSD: `swayosd` vs a bar popup (the bar has the modules; decide with a
  complexity/RSS argument, not taste). Must show above fullscreen (needs
  `overlay` layer — same rule as notifications).
- Edge cases: no audio hardware (headless/VM — binds fail loud per-entry,
  never break the session); Bluetooth sink switching (pairs with the
  network/Bluetooth picker work in `desktop-apps`); per-output brightness
  on multi-monitor.

Acceptance: eval pins in `nix/tests.nix`; real-login proof on the M2
(every bound key changes the real control and shows the OSD; unplug/replug
a sink); docs in `docs/nix.md` + default-binds reference.

## Not in this ticket

EQ/effects (EasyEffects), per-app routing UI, screen-reader support.
