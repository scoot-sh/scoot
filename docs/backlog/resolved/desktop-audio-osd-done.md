---
title: "Desktop audio/brightness/media keys and an OSD"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-06"
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

## Resolution (PR #471)

Filled `desktop.audio` on branch `feat/nix-desktop-audio-osd`:

- **Already done by siblings, not rebuilt:** the keymap (`desktop-keys`,
  PR #447) already binds every key this ticket names (brightness,
  volume, mic-mute, media, screenshots); `desktop-capture` (PR #459)
  already runs PipeWire. This child reroutes the volume/brightness/
  mic-mute binds through OSD scripts, adds the OSD, the sink helper,
  and the NixOS PipeWire default.
- **Baseline:** PipeWire with WirePlumber (`services.pipewire.enable`
  as the profile default, merging with the capture slot's). A
  PipeWire-only shape would leave the binds' `wpctl` with nothing to
  call, for ~4.7 MiB saved (measured NAR over `pipewire` itself at
  the pinned rev, `aarch64-linux`). Apple Silicon speaker tuning
  stays the user's own unit (the M2's speakers-heal), referenced from
  the docs.
- **OSD pick (measured, not taste):** wob 0.16 — 57.5 MiB closure,
  225 KiB new over the profile, ~2.2 MB idle RSS, 2 wakeups in 62 s
  hidden (`overlay` in its own source, hides itself after the
  timeout, per-output sections, `value [style]` lines, ISC). Against:
  swayosd 0.3.1 (1.06 GiB closure, 228 MiB new, a GTK4 stack that
  *replaces* `wpctl`/`brightnessctl`/`playerctl` instead of composing
  with them) and a scootbar popup (new Rust surface role + IPC +
  auto-dismiss, and it forces the bar on for the OSD).
- **Behavior:** volume steps the default sink 5% (fractions and
  percents parsed; past 100% clamps into an urgent overflow fill),
  mic-mute toggles the source, brightness steps every backlight
  device and shows the average (keyboard LEDs excluded), media keys
  stay direct (the bar's media module shows state). No device at all
  fails loud per entry (message + exit 1, session unaffected).
  `scoot-audio-sink list|set|cycle` is the contract the future
  Bluetooth picker calls.
- **Proof:** eval pins plus script behavior tests against stub tools
  (`nix/tests.nix`, all green in `nix flake check`); live on the
  Asahi M2 in a headless scoot with module-rendered binds, scripts
  and wob config — key presses drove the real `wpctl` (1.00 to 1.05
  to 0.95, mute toggled, restored to 1.00 unmuted) and the OSD mapped
  above a fullscreen window (IPC screenshots); mic-mute (no sources),
  brightness (no seat: EPERM) and media (no players) each failed loud
  with the session alive; volume and the panel (51/509) restored and
  re-verified.
- **Docs:** the site's desktop page (task-first audio section with
  the measured pick, the sink contract, symptom troubleshooting)
  plus the keybindings reference; the OSD screenshot is IPC-captured.
