---
title: scootbg troubleshooting
description: "Wallpaper symptoms and their fixes — images, links, outputs, restore."
---

## Symptoms

- *`set` says no such file / not an image / too large.* The file must
  exist, be a regular file, and start as PNG, JPEG, GIF or WebP (told apart
  by content, not name); images over 16384×16384 pixels are refused.
  Every output keeps what it showed.
- *`set` refuses an animation past the size cap.* An animated GIF, APNG
  or animated WebP holds at most 64 frames and 64 MiB of frames; past
  that the `set` is refused and every output keeps what it showed. Pass
  `--no-animate` to show the first frame as a still instead (playing
  frame by frame is a follow-up). That still is live-only: after a
  restart the caps are checked again, so an over-cap animation is refused
  then and the output loses its wallpaper until the next `set`.
- *A link won't show.* In order: no `curl` on `PATH`; host unresolvable
  (names curl's exit); HTTP error (`exited 22`); error page ("do not
  start as a PNG, JPEG, GIF or WebP", caches nothing); past 32 MiB (link
  something smaller); `sha256` mismatch (prints both hashes — copy the
  actual one in, or drop the pin); unwritable cache dir. Offline at
  startup, the background shows until the next `set`, `apply-config`,
  reconfigure, or restart with the network back. Full detail in
  [Wallpaper from a link](./from-url.md).
- *Image not filling the screen.* Check `--mode`: `fill` covers and
  crops (the default), `fit` letterboxes with `--fill`, `stretch`
  distorts, `center` draws unscaled, `tile` repeats. A 20,000,000×1
  strip with `fit`/`stretch` is a drawing error — `fill`, `center` or
  `tile` show it.
- *Wrong output / unknown output.* `set --output NAME` names a
  connector `query` lists; a name no output has right now is an error
  (exit 1) changing nothing. `set` with no `--output` replaces every
  choice, per-output ones included.
- *Restore lost it.* `query` reports `"saving":false` while saving is
  off (unreadable state file, or a newer version's format — stderr
  names which file to remove or fix). A saved image that moved is
  skipped with a warning until the file is back; a saved *link*
  re-downloads instead. See [Restore](./restore.md).
- *A second daemon / stale socket.* One daemon per display; a second
  `scootbg daemon` refuses while one runs, and a socket left by a dead
  one is replaced. Signals leave the socket file; the next daemon
  replaces it.
- *The change lands at once, no animation.* No `--transition` on the
  request (the default is `none`), a zero `--duration-ms`, or a `clear`
  (always instant). With a `[wallpaper]` section, the keys are strings
  and the table you mean carries them; an older scootbg refuses the
  section outright. See [Transitions](./transitions.md).
- *`set` refuses the transition.* A value outside the lists in
  [Transitions](./transitions.md) (durations are digits in milliseconds,
  angles plain numbers, positions two fractions with a comma), a
  transition flag without `--transition`, or any of them on a `clear`:
  exit 2, nothing changed.

- *Out of memory: `scootbg` exits with `memory allocation of ... bytes
  failed`, or a draw fails with `Cannot allocate memory`.* The daemon
  runs under an address-space limit (`ulimit -v`, systemd's `LimitAS=`)
  or strict overcommit (`vm.overcommit_memory=2`). A draw that scales
  (`fill`, `fit`, `stretch`, any image sized differently than the
  output, with any `--filter`) first probes the scaler's whole budget —
  the output, each scaled axis's weight tables, and the row scratch when
  both axes scale — and refuses
  the draw when the probe fails instead of letting the scaler end the
  daemon: `set` exits 1, `query` reports `draw_failed: true` with
  `draw_error: "out of memory: cannot allocate ... bytes for scaling"`,
  the output keeps what it showed, and the next `set` retries. That probe
  is a heuristic: nearly sound under an address-space limit (the daemon
  draws one job at a time), racy under strict overcommit, where another
  process can take the commit charge between the probe and the scaler's
  own allocation — and the image decoders' own working memory stays
  infallible either way, so a huge image can still end the daemon where a
  scaling draw no longer does. Diagnose with the failed draw first, then
  the limit:

```sh
scootbg query        # draw_failed, draw_error: what the draw needed
prlimit --as --pid "$(pidof scootbg)"   # the daemon's address-space limit
ulimit -v            # your shell's (a daemon started here inherits it)
cat /proc/sys/vm/overcommit_memory   # 2 means strict overcommit
```

What to change: give the daemon address space — raise or drop the
`LimitAS=` line that starts it (or start it with `ulimit -v unlimited`),
rather than shrinking the wallpaper. Do not answer a wallpaper failure
by switching the whole machine to strict overcommit; it trades one
refused draw for aborts anywhere the probe's race is lost.
- *Nothing applied from scoot.* Check the `[wallpaper]` section
  ([The `[wallpaper]` section](./index.md#the-wallpaper-section)): an
  unknown key or wrong-typed value refuses the section (the rest of the
  file still applies); `command` naming nothing warns and retries next
  reload. And leave `scootbg daemon` out of `[autostart]` —
  `apply-config` starts it.

## Diagnose with query

```sh
scootbg query
```

`draw_failed: true` tells a failed draw apart from a `clear`;
`draw_error` says why; `shows` names what's on screen (with `url`
beside `image` for a download); `saving` and `profile` name the state
behind [Restore](./restore.md). Exit statuses: `0` done; `1` no
daemon, unknown output, unshowable image, or drawing failed; `2`
usage error.
