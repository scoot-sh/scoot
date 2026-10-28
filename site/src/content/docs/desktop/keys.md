---
title: Hardware keys and desktop actions
description: "The profile keymap for hardware keys and desktop actions."
---

The profile ships one keymap for hardware keys and desktop actions —
on with the profile. The compositor's own
built-in defaults stay window management only: hardware binds spawn
tools scoot does not ship, so they live in the profile.

| Press | Does | Needs (beside the profile) |
|---|---|---|
| `XF86MonBrightnessUp` / `Down` | panel `+5%` / `-5%` | brightnessctl (installed) |
| `XF86AudioRaiseVolume` / `LowerVolume` | default sink `+5%` / `-5%` | PipeWire running |
| `XF86AudioMute` | default sink mute toggle | PipeWire running |
| `XF86AudioMicMute` | default source mute toggle | PipeWire running |
| `XF86AudioPlay` / `Pause` / `Stop` / `Next` / `Prev` | play-pause / pause / stop / next / previous | a player speaking MPRIS |
| `Super+Escape` | lock (through logind) | the locker |
| `Super+d` | launcher (XDG apps) | `launcher.enable` (scootlaunch panel) |
| `Ctrl+Alt+Space` | run mode (PATH executables alone) | `launcher.enable` (scootlaunch `--run`) |
| `Super+v` | clipboard picker | `clipboard.enable` |
| `Super+n` | dismiss visible notifications | `notifications.enable` |
| `Super+Shift+n` | do-not-disturb toggle | `notifications.enable` |
| `Super+Ctrl+n` | show hidden notifications | `notifications.enable` |
| `Super+p` | cycle the power profile | `power.enable` (opt-in, never with the profile) |
| `Print` | screenshot every output into `~/Pictures` | `capture.enable` |
| `Shift+Print` | screenshot a picked region into `~/Pictures` | `capture.enable` |
| `Ctrl+Print` | screenshot a picked region into the clipboard | `capture.enable` |
| `Super+w` | WiFi picker | `apps.network.enable` (NetworkManager running) |
| `Super+b` | Bluetooth picker | `apps.bluetooth.enable` (BlueZ running) |

On an Apple keyboard these are the Fn row: `F1`/`F2` brightness,
`F7`/`F8`/`F9` previous/play/next, `F10` mute, `F11`/`F12` volume
down/up. Volume, brightness and mic-mute show the
[on-screen display](./sound.md);
every other bind runs silent. Every
hardware keysym and every `Super` combo above has no overlap with the
compositor defaults, including `Super+Shift+e` (quit) and
`Super+Space` (float focus).

Why these two combos: `Super+d` is the launcher in niri, and
`Super+Space` — the other candidate —
already moves focus between floating windows and the strip, so taking
it would rename a shipped default out from under existing users.
`Ctrl+Alt+Space` keeps the run menu on the combo the old `wofi`
example used (hands that already know it keep a menu there; the drun
half moves to `Super+d`, where the rest of the desktop expects it).

Every value is an option, applied on rebuild/switch plus a session
reload (`scoot msg reload`) or re-login:

| Option | Type | Default | Meaning |
|---|---|---|---|
| `desktop.keys.enable` | bool | `true` with the profile | render the keymap into `[binds]` |
| `desktop.keys.binds.<name>.enable` | bool | `true` | bind that key (`brightnessUp`, `brightnessDown`, `volumeUp`, `volumeDown`, `volumeMute`, `micMute`, `mediaPlay`, `mediaPause`, `mediaStop`, `mediaNext`, `mediaPrev`, `lock`, `launcher`, `launcherRun`, `clipboard`, `notifDismiss`, `notifDnd`, `notifHistory`, `powerProfile`, `captureOutput`, `captureRegion`, `captureClipboard`, `network`, `bluetooth`); `false` leaves its combo unbound |

Each bind renders as a default a value you set in `settings.binds` wins
over — override or remove one bind like this:

```nix title="home.nix"
programs.scoot.desktop.keys.binds.volumeUp.enable = false;
programs.scoot.settings.binds."XF86AudioRaiseVolume" = "spawn wpctl set-volume @DEFAULT_AUDIO_SINK@ 3%+";
```

Overriding with a plain string like that drops the bind's repeat and
lock-screen behavior (a string carries no flags). To keep them, override with the table form — action plus flags:

```nix title="home.nix"
programs.scoot.settings.binds."XF86AudioRaiseVolume" = {
  action = "spawn wpctl set-volume @DEFAULT_AUDIO_SINK@ 3%+";
  repeat = true;
  allow_when_locked = true;
};
```

A bind whose tool is missing fails quietly — scoot logs a warning and
carries on, input never wedges — so a partial setup degrades to dead
keys, never to a broken session.

Troubleshooting, by symptom:

- *A Fn key does nothing.* Check the tool first, not the bind:
  `brightnessctl get`, `wpctl get-volume @DEFAULT_AUDIO_SINK@`,
  `playerctl status`. Then check the bind reached the file (`[binds]`
  in `~/.config/scoot/config.toml`, after a reload). Then check scoot
  saw the key: `scoot msg key XF86AudioRaiseVolume` should do what the
  key does — if it does, the compositor never received the keystroke.
- *Volume or brightness keys die at the lock screen.* Those binds
  fire while locked by default. If yours do not, the bind
  lost its flags — overriding one with a plain string drops
  `allow_when_locked`, and removing it through
  `binds.<name>.enable = false` leaves the combo unbound entirely.
  Check the rendered `[binds]` in `~/.config/scoot/config.toml`: a
  working volume bind is a table with `allow_when_locked = true`.
- *Holding volume up steps once.* It should keep stepping at 25 Hz
  after a 200 ms delay. A bind that fires once per press lost its
  `repeat` flag the same way: a plain-string override, which the
  rendered `[binds]` shows as a bare string instead of a table with
  `repeat = true`. (Overriding with a plain string drops both flags;
  to keep them, override with the table form — action plus flags.
  See [the bind grammar](../scoot/keybindings.md#the-bind-grammar).)
- *`Super+d` opens nothing.* The launcher slot is off: that bind
  renders only with `launcher.enable` beside the keymap (both on
  with the profile), and either off leaves the combo unbound. Same
  for `Ctrl+Alt+Space` (the run bind) and for the three capture
  binds ([Screenshots](./screenshots.md)) — while `Super+v` runs the clipboard
  picker ([Clipboard](./clipboard.md)) whenever `clipboard.enable` is on
  beside the keymap, as does the `Super+n` family (the notification
  daemon's own
  commands) whenever `notifications.enable` is, and `Super+w` /
  `Super+b` ([the pickers](./network.md)) whenever
  `apps.network.enable` / `apps.bluetooth.enable` is.
