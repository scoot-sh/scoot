---
title: Restore
description: "The wallpaper comes back at the next start: profiles, state files, and what survives."
---

**The wallpaper is restored at the next start.** Every `set` and `clear` is saved, per output, and shown again when the daemon starts.

## Restore

**The wallpaper is restored at the next start.** Every `set` and `clear`
is saved, per output (the choice for every output, and each one made with
`--output NAME`), in a state file, `$XDG_STATE_HOME/scootbg/PROFILE`
(`~/.local/state/scootbg/PROFILE` when `XDG_STATE_HOME` is unset or not
absolute), and `scootbg daemon` shows it again when it starts:

- **A profile, not a display, names the state.** `--profile NAME` picks
  it (default `default`): 1 to 64 of `A-Z a-z 0-9 . _ -`, not starting
  with `.` and without `..`. scoot binds the first free `wayland-N`, so
  the display name is no session identity; give each session its own
  profile (a sway autostart can pass `--profile sway`) and they never
  restore each other's wallpaper. Two sessions sharing a profile share
  its state, the last change winning.
- **When it is saved:** a color or a `clear` as soon as the daemon has it,
  an image once it has decoded (one that cannot be shown changes nothing,
  so saves nothing). The file is written off the daemon's loop,
  atomically (a fresh temporary file that never follows a symbolic link,
  `fsync`, `rename`), private (0600, in a 0700 directory; a state
  directory that already exists and is someone else's, or writable by
  its group or others, is a warning at start-up); `scootbg kill` waits
  for a write under way, and a signal leaves the old file or the new one, never half
  of each.
- **What survives:** a choice for an output that is not plugged in now
  stays saved and comes back with it; `set` without `--output` replaces
  every per-output choice, as it does on screen. A restore saves nothing.
  At most 256 per-output choices are kept (and 256 KiB in all): past
  that, the least recently set go first, with a warning.
- **A saved image that is gone** (moved, deleted, on a disk not mounted
  yet) is skipped with a warning on the daemon's stderr, and that output
  shows the compositor's own background. The daemon starts all the same,
  and the entry stays saved until a `set` or `clear` replaces it, so it
  is restored once the file is back. A saved *link* is never skipped for
  a missing cache file: it is recorded live, and the worker downloads it
  when the outputs are configured (offline, the background shows and one
  line says why; the next start tries again). If the cached file is
  deleted out from under a running daemon, the next reconfigure
  re-downloads it; a `sha256` mismatch on a file already cached
  re-downloads once, then fails loudly.
- **`--no-restore`** starts with nothing shown. The state is still read,
  and a `set` or `clear` then updates it as usual, keeping the rest.
- **A state file it cannot use never stops the daemon.** A malformed line
  (or one whose path is not UTF-8) is skipped with a warning and the rest
  is used; a file that is not one (no `scootbg-state 1` or `scootbg-state 2` first line, over
  256 KiB) restores nothing and is replaced at the next save; a newer
  scootbg's format (a later version) restores nothing and is never
  written over; an unreadable one restores nothing, and nothing is saved
  over it. In those two cases **saving is off until the daemon
  restarts**: `set` still works on screen, stderr says at start-up which
  file to remove or fix to save again, and `scootbg query` reports
  `"saving":false`. The format is a documented, versioned line format
  ([README.md](https://github.com/scoot-sh/scoot/tree/main/docs/scootbg/README.md#restore)).
- **A restored image is decoded once**, when its outputs are configured:
  about 450 ms to a 6000×4000 JPEG on screen on a 4K output, the same as a
  `set` on a running daemon. So is an image `set` sent the moment the
  daemon starts: it waits for the outputs' `configure` (bounded by a
  round trip already sent) rather than decode once to check the file and
  again to draw it.
