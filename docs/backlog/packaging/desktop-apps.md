---
title: "Desktop apps: terminal, file manager, network/bluetooth pickers, automount"
status: "open"
area: "packaging"
priority: "low"
blocked: null
---

# Desktop apps: terminal, file manager, network/bluetooth pickers, automount

Filed 2026-10-04, child 12 of `desktop-paved-path`. Serves
**daily-drive** (terminal, files, WiFi/Bluetooth joining, thumb drives).

## The gap

`super+Return` spawns `foot` by default but the flake installs no terminal.
No file manager is referenced anywhere. scootbar has `network`/`bluetooth`
modules, but `bluetooth-real-hardware.md` is open and `network-child-stuck`
is claimed/in-flight — display without pickers. `udiskie` has zero mentions.

## What to do

Fill the `desktop.apps` slot (all optional, all default-on except the file
manager — say the defaults):

- Terminal: `foot` installed + the look's `foot.ini` applied (theme child
  owns the palette; this child owns the package + `TERMINAL` env +
  keeping the `super+Return` bind true).
- File manager (optional): pick the lightest well-maintained graphical one
  (or bless `yazi` terminal-based to stay light — decide with a closure/RSS
  argument, not taste); `xdg-open`/`mimeapps` defaults so the portal file
  chooser and browsers agree.
- Network/Bluetooth pickers: complete what the bar modules start — picker
  UI through the launcher dmenu contract (child `desktop-launcher`), using
  `nmcli`/`bluetoothctl` shims or the bar's own popups (coordinate with the
  in-flight bar entries; don't duplicate them). Joining a network and
  pairing a device must work from the keyboard.
- Automount: `udiskie` (or the lighter `udisks2`-trayless shape — decide)
  as a user unit with the safe-removal rule stated.
- Edge cases: no network hardware (VM — pickers fail loud, not hang;
  relates to the claimed `network-child-stuck`); secrets for WiFi psk via
  the auth/secrets child (coordinate, don't duplicate); file manager
  absence must never break `xdg-open` fallback.

Acceptance: eval pins in `nix/tests.nix`; real-login proof on the M2
(terminal bind, file manager opens, WiFi join + BT pair from keyboard,
thumb-drive automount + safe removal); docs in `docs/nix.md`.

## Not in this ticket

Email/calendar/contacts clients; browsers (say the default `BROWSER`, not
the browser); printer/scanner setup (CUPS notes at most).
