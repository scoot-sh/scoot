---
title: "Desktop: one default keymap for hardware keys and desktop actions"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-05"
---

# Desktop: one default keymap for hardware keys and desktop actions

Filed 2026-10-04, child of `desktop-paved-path`. The maintainer: "We should
ship default key shortcuts that do screen brightness, volume, all of that
too. Maximize and all the other main ones." Serves **daily-drive**: a laptop
whose brightness and volume keys do nothing is not daily-drivable.

## Resolution (PR #442, 2026-10-05)

Landed as `programs.scoot.desktop.keys` (`nix/modules/keys-home.nix`,
shapes in `desktop.nix`, system tools in `nixos.nix`, pins in
`nix/tests.nix`, docs in `docs/nix.md` "Hardware keys and desktop
actions"). Nineteen binds: brightness/volume/mute/mic/media/lock
with the profile; launcher (`Super+d`), clipboard (`Super+v`),
notification dismiss/DND/history, `Print`/`Shift+Print` only with
their slots. Kept existing defaults (`Super+m` maximize,
`Super+f` fullscreen, cross-monitor focus/move) and listed them in
the one keymap table. `Super+d` over `Super+Space` for the launcher
(the latter moves floating focus today; renaming it would break
users). Compositor defaults gain no hardware keys: they would spawn
tools scoot does not ship, so they stay a profile concern.

Verified in source, not assumed: missing tools fail quietly
(`State::spawn` warns, returns false); no `[binds]` action fires
while locked except VT switch (filed: `core/bind-allow-when-locked`);
a held key fires once, no repeat timer exists (filed:
`core/bind-repeat`). Keyboard backlight unbound (no stable device;
the M2 exposes none). OSD is a hook for `desktop-audio-osd`.
Notifications (#441 unmerged) got slot-gated `makoctl` binds, not
stubs. Eval pins for all 19 binds, an override, a removal, and
slot gating; live `--tty` proof on the M2 in the PR (before/after
`brightnessctl`/`wpctl` readings, uinput Fn-key injection, lock
behavior shown, volume/brightness restored).

## The gap

scoot's built-in defaults (`docs/configuration.md` "Default keybindings")
already cover window management: focus/move by column, window, workspace
and output, `Super+m` maximize, `Super+f` fullscreen, float, close,
`Super+Return` terminal, quit. Nothing binds the hardware keys
(`XF86MonBrightnessUp/Down`, `XF86AudioRaiseVolume/LowerVolume/Mute`,
`XF86AudioMicMute`, `XF86AudioPlay/Pause/Next/Prev`, `XF86KbdBrightness*`,
`Print`) or the desktop actions the other children add (launcher, lock,
clipboard picker, screenshot, notification dismiss, power menu). Each child
currently plans its own keys, so nothing guarantees one coherent map.

## What to do

- One keymap owned here, used by every child: the desktop profile sets each
  bind with `mkDefault` (overridable or removable one at a time; a user's
  own `[binds]` entry wins), and the other children register their action
  here instead of picking keys alone. Pin it in `nix/tests.nix`.
- Hardware keys: brightness and keyboard backlight (`brightnessctl`, with
  `-e`/exponent so low steps are usable), volume and mute on the default
  sink plus mic mute (`wpctl` on PipeWire), media keys (`playerctl`, MPRIS:
  the bar's media module speaks it too), `Print`/`Shift+Print`/
  `Super+Shift+s` for screen, region and window capture (with
  `desktop-capture`). Each with the OSD from `desktop-audio-osd`.
- Desktop actions: launcher (`Super+d` or `Super+Space` -- `Super+Space`
  is taken by floating focus today; decide and say why), lock
  (`Super+Escape` class), clipboard picker (`Super+v` class), notification
  dismiss and history, power menu. Avoid every existing default and say so
  in a collision table.
- Verify in the compositor source, do not assume: whether binds fire while
  the session is locked (volume and brightness should; launcher and
  clipboard must not), whether a held key repeats a bind (holding volume up
  must step, not fire once), and whether the hardware keysyms reach `[binds]`
  on `--tty` from the M2's keyboard (the Fn row sends `XF86*` there).
  Anything missing on the compositor side is its own fix, filed or done in
  this PR with tests.
- Decide whether the compositor's own built-in defaults gain the hardware
  keys (they would spawn tools scoot does not ship, so a missing tool must
  fail quietly, never wedge input) or whether they stay a profile concern;
  say why either way.
- Docs: a single keymap table in `docs/nix.md` (desktop profile) and the
  built-in table in `docs/configuration.md` if it changes.

## Acceptance

Eval pins for every default bind and for an override and a removal; a real
`--tty` login on the M2 pressing each Fn key (brightness, volume, mute, mic,
media, keyboard light) with before/after readings (`brightnessctl get`,
`wpctl get-volume`), holding a key to show repeat, and the lock-screen
behavior shown.

## Not in this ticket

The tools' own setup (each slot's child installs and themes its tool);
user-defined chords and modes.
