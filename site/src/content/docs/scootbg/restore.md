---
title: Restore
description: "The wallpaper comes back at the next start: profiles, state files, and what survives."
---

Every `set` and `clear` is saved, per output, and shown again when the daemon starts.

## Restore

**The wallpaper is restored at the next start.** Every `set` and `clear`
is saved, per output (the choice for every output, and each one made with
`--output NAME`), in a state file, `$XDG_STATE_HOME/scootbg/PROFILE`
(`~/.local/state/scootbg/PROFILE` when `XDG_STATE_HOME` is unset or not
absolute; with neither it nor `HOME` absolute there is nowhere to keep
state, and nothing is restored or saved), and `scootbg daemon` shows it again when it starts:

- **A profile, not a display, names the state.** `--profile NAME` picks
  it (default `default`): 1 to 64 of `A-Z a-z 0-9 . _ -`, not starting
  with `.` and without `..`. scoot binds the first free `wayland-N`, so
  the display name is no session identity; give each session its own
  profile (a sway autostart can pass `--profile sway`) and they never
  restore each other's wallpaper. Two sessions sharing a profile share
  its state, the last change winning.
- **When it is saved:** a color or a `clear` as soon as the daemon has it,
  an image once it has decoded (one that cannot be shown changes nothing,
  so saves nothing). The file is written
  atomically, private (0600, in a 0700 directory); `scootbg kill` waits
  for a write under way.
- **What survives:** a choice for an output that is not plugged in now
  stays saved and comes back with it; `set` without `--output` replaces
  every per-output choice, as it does on screen. A restore saves nothing.
  At most 256 per-output choices are kept (and 256 KiB in all): past
  that, the least recently set go first, with a warning.
- **A saved image that is gone** (moved, deleted, on a disk not mounted
  yet) is skipped with a warning, and that output
  shows the compositor's own background.
  The entry stays saved until a `set` or `clear` replaces it, so it
  is restored once the file is back. A saved *link* is never skipped for
  a missing cache file: it is downloaded
  when the outputs are configured (offline, the background shows until
  the next start).
- **`--no-restore`** starts with nothing shown. The state is still read,
  and a `set` or `clear` then updates it as usual, keeping the rest.
- **A state file it cannot use never stops the daemon.** A malformed line
  (or one whose path is not UTF-8) is skipped with a warning and the rest
  is used; a file that is not one (no `scootbg-state N` first line
  (versions 1–3), over
  256 KiB) restores nothing and is replaced at the next save; a newer
  scootbg's format (a later version) restores nothing and is never
  written over; an unreadable one restores nothing, and nothing is saved
  over it. In those two cases **saving is off until the daemon
  restarts**: `set` still works on screen, stderr says at start-up which
  file to remove or fix to save again, and `scootbg query` reports
  `"saving":false`. The format is a documented, versioned line format
  ([README.md](https://github.com/scoot-sh/scoot/tree/main/docs/scootbg/README.md#restore)).
- **A restored image is decoded once**, when its outputs are configured.
