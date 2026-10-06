---
title: "Desktop apps: terminal, file manager, network/bluetooth pickers, automount"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-06"
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
  chooser and browsers agree, and `xdg.userDirs` so Downloads, Pictures
  and the screenshot folder exist.
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

## Resolution (2026-10-06)

Landed in PR #484 (`feat(nix): desktop apps`), documented at
site/src/content/docs/desktop/index.md (*Terminal, files, and removable
media*; *WiFi and Bluetooth*).

- Terminal: foot on with the profile, `TERMINAL=foot`, `xdg-open` plus
  `BROWSER=xdg-open` (no browser slot), `xdg.userDirs` created.
- File manager: pcmanfm, opt-in, directories and mount points opening in
  it. Picked on numbers (M2): 347 MiB full closure and 5.3 MiB over the
  profile against thunar's 376/27.0 and wrapped yazi's 525/75.1; ~50 MB
  idle RSS, 0 context switches in 60 s idle (yazi in foot: 418), nothing
  left running once closed.
- Pickers: `Super+w`/`Super+b` and the bar's network/bluetooth modules
  (menu-command/connect-command wired at `mkOptionDefault`) through fuzzel.
  WiFi joins saved networks by UUID (matched by SSID), open ones directly,
  secured ones with the keyring's key or a masked prompt, the key on
  nmcli's stdin, a failed first join's profile removed. Bluetooth toggles
  paired devices, pairs new ones after a bounded 10 s scan with a no-input
  agent (PIN devices from a terminal), switches power and the audio sink.
  Every bluetoothctl call is bounded: with no bluetoothd it waits forever
  (measured against an empty private bus on the M2). Failures notify.
  Neither picker takes over NetworkManager or BlueZ.
- Automount: trayless udiskie over udisks2, its config ignoring the
  machine's own disks (stock udiskie would try every unmounted internal
  partition at login, an admin prompt each on a dual-boot box), safe
  removal by `udiskie-umount -d`. udiskie ~58 MB RSS, 0 wakeups; kept over
  a hand-rolled `udisksctl monitor` loop (~8 MB floor) for its partition,
  LUKS and Browse handling. An empty `/etc/nvme` stops udisksd's 4 s GLib
  retry on the missing directory (15 wakeups/min to 0, measured).
- Proven live on the M2 in a real greetd `scoot-test` login against stub
  `nmcli`/`bluetoothctl` (the M2's networking is off limits); the physical
  Bluetooth pair and a real thumb drive are the maintainer's checklist in
  the PR.
