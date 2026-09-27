---
title: "Restoring the last wallpaper at startup"
status: "resolved"
area: "scootbg"
priority: null
blocked: null
---

# Restoring the last wallpaper at startup — RESOLVED

Resolved 2026-09-27. What landed, where it departs from the plan, the
measurements with their method and raw numbers, and what could not be
verified are in [Resolution](#resolution) at the end; the original
ticket follows unchanged.

- After each successful set, write the choice per output (connector name,
  or "all") to `$XDG_STATE_HOME/scootbg/`, atomically (write and
  rename).
- `scootbg daemon` restores it; `--no-restore` skips it; a missing or
  moved image falls back to the compositor's own background with a
  warning.
- The state file also records the fingerprint of the last section
  applied through `scootbg apply-config`, which is how the rule in
  [scoot-integration.md](../scoot-integration.md) (whichever you changed
  last wins) survives restarts. Only `apply-config` writes it.
- **One state file per profile** (`--profile NAME`, default `default`),
  not per display: scoot binds the first free `wayland-N`, so the socket
  name shifts with start order and stale sockets and is no session
  identity. scoot passes `scoot` (or `scoot-nested`), a sway user's
  autostart can pass `sway`, and sessions with different profiles never
  restore each other's wallpapers. A daemon takes its profile from its own
  `--profile`, and adopts the profile of each `apply-config` it receives (see
  [scoot-integration.md](../scoot-integration.md)), so a redundant
  `[autostart]` daemon never splits state across two profiles. Two
  concurrent sessions sharing a
  profile share state, last writer wins; that is the documented
  trade-off of a name the user controls.
- Optional follow-up, measured first: cache the scaled buffer (compressed)
  in `$XDG_CACHE_HOME/scootbg/` so restore skips decode and scale. Only
  worth it if startup measurements say decode is the slow part.
- **Measured in [ticket 8](memory-and-idle-done.md#startup),
  without restore:** a color is on screen 3.0–4.1 ms after `scootbg
  daemon` is started, but an image `set` sent straight away takes
  642–720 ms against about 440 ms on a running daemon, because it
  **decodes the file twice**: the trial finds no configured output to draw
  for, so it decodes only to validate, and the `configure` that follows
  asks for a render that decodes it again (two opens of the file, by
  strace; one when the `set` comes after the `configure`). A restore at
  login is exactly that request, so this ticket should make it one decode
  (let a trial with no configured target wait for the first `configure`
  of its outputs, bounded like the replies are) and measure startup again
  with a restored 4K JPEG.

## Resolution

### What landed

- **Saving** (`crates/scootbg/src/state.rs`, `state/format.rs`,
  `state/saver.rs`). After each `set` or `clear` the daemon records, the
  choice goes to `$XDG_STATE_HOME/scootbg/PROFILE`
  (`~/.local/state/scootbg/PROFILE` when `XDG_STATE_HOME` is unset, empty
  or relative, as the XDG spec says): the choice for every output and each
  per-output choice by connector name, plugged in now or not. A color or
  `clear` is saved when it is recorded, an image once it has decoded and
  been recorded; a request superseded before it landed is not saved,
  because the file follows the same newest-wins rule
  (`Saved::record` calls `Choices::set` on a table of its own).
- **The file format** is hand-written, versioned lines (the format and
  its guarantees are in [the Restore section](../../README.md#restore)):
  a `scootbg-state 1` header, `profile`, `fingerprint`, `all` and
  `output NAME` lines, each choice `clear`, `color #rrggbb` or `image
  PATH MODE FILL FILTER`; `%`, space and ASCII control bytes escaped
  `%XX`, so paths and names round-trip exactly (spaces, `#`, newlines,
  tabs, `%`, UTF-8). Read defensively: a malformed line, a field not
  UTF-8 once unescaped (the non-UTF-8 path of
  [dependencies-done.md §5](dependencies-done.md#5-serialization-control-socket-and-state-file):
  paths are UTF-8 only, since the control protocol is JSON), an unknown
  key, each skipped with a warning naming its line; duplicates warn and
  the later counts; over 256 `output` lines or a file over 256 KiB are
  bounded; a header that is not one restores nothing; a later version is
  neither read nor written over.
- **Writing off the loop.** A thread started per save, ending when nothing
  waits (the decode worker's pattern), writes a temporary file
  (`.PROFILE.PID.tmp`, 0600), `fsync`s it, renames it over the file and
  `fsync`s the directory (made 0700 if needed). A newer text replaces a
  waiting one, so a burst is at most two writes. The daemon waits up to 2
  s for a write under way on its way out, so `scootbg kill` after a `set`
  keeps it. If no thread can be started, the write happens on the loop.
- **Restoring** (`daemon/restore.rs`): after the Wayland connection, before
  the loop serves anyone, the file's choices go in order (every output,
  then each named one, each with a new generation, so any request is
  newer) into the saved table and, unless `--no-restore`, the daemon's
  choices. Nothing is drawn then (no output is configured yet): each
  output draws when its `configure` comes, as for any choice. A saved
  image that is not a regular file now (one `stat`) is left out of the
  shown choices with a warning, and that output shows the compositor's
  background; it stays in the saved table, so it stays in the file.
- **`--profile NAME`** and **`--no-restore`** on `daemon`. A name is 1 to
  64 of `A-Z a-z 0-9 . _ -`, not starting with `.`, without `..`
  (`state::Profile::parse`); anything else is a usage error, exit 2.
- **The fingerprint is reserved**: read, kept, and written back by every
  save; nothing sets it until `apply-config`
  ([scoot-integration.md](../scoot-integration.md#what-ticket-9-provides)).
  So is `profile`. Neither needs a version bump later.
- **The double decode is gone** (`Output::coming`, `Jobs::next`,
  `daemon::images::trial_targets`). A trial's targets are decided when it
  starts rather than when it is asked, and it does not start while an
  output it targets is about to be configured: not settled yet, its
  surface pending (and not late), being re-created after a `closed`, or
  configured with that `configure`'s round trip still out. Each ends when
  a round trip already sent comes back, the same bound as a reply's, with
  no timer. A restore needs no hold of its own: it makes no trial, and
  its renders are asked for only once outputs are configured.
- **Tests.** Unit: the format (round trips, escaping, hostile paths, every
  malformed-line kind, headers, versions, limits, duplicates, 2,000
  random inputs), the writer (atomic, private, failing, a burst, a
  failure that must not wedge), profile names, the state directory,
  `Saved`'s rules, the CLI flags, `Output::coming` through every state,
  the hold in `Jobs`. End to end (`tests/restore.rs`, added to CI's
  integration job): a `set` surviving `kill` and SIGTERM restarts by
  screenshot; `--no-restore`; a moved image; an unplugged output's entry
  surviving a one-output session; two profiles; unusable state files; and
  the decode counted by inotify (a restore, an early `set`, and a restore
  on two outputs of different sizes on sway). Every session's
  `XDG_STATE_HOME` is a scratch directory (`common::Session::state_home`),
  so no test restores another's wallpaper, or the user's.

### Departures from the plan, and why

- **A restored image is not a trial**, as
  [images-decode-and-fit-done.md](images-decode-and-fit-done.md) had
  suggested ("restoring one is a trial like any `set`"). It is put in the
  choices after a `stat`, and decoded by the ordinary render once its
  outputs are configured. A trial would decode to validate even for an
  entry whose output is not plugged in (hundreds of ms of CPU at login for
  nothing to draw), and its reply machinery has no client here. What a
  trial gives, nothing stale on screen, holds either way: a file that is
  gone is caught by the `stat`, and one that exists but will not decode
  fails its draw, says so, and shows nothing.
- **The file mirrors a table of its own, not what is on screen.** That is
  what keeps entries for absent outputs and for images that could not be
  restored across later `set`s, and what makes a restore write nothing.
- **`--no-restore` still reads the file**, so a `set` then updates it
  rather than dropping everything else it held.
- **A newer version's file is never written over**, and neither is one
  that could not be read: saving is off for that session, said on stderr.
  Replacing either would lose what this build cannot see.
- **The ticket's "hold a trial for the first configure of its outputs"**
  landed as holding the trial until *none* of its outputs is about to be
  configured, and deciding its targets then, so one decode draws every
  output it targets. Renders are not held: tried, and on both
  compositors checked it changed nothing (sway configures a session's
  outputs in one batch, so their renders merge; scoot's outputs are one
  size, so a later one shares the pixels), so it was taken out again as
  speculative. `a_restore_on_outputs_of_two_sizes_decodes_once_on_sway`
  is the check that it is not needed there.
- **The scaled-buffer cache is not built.** Decode is nearly all of a
  restore (443–477 ms of it, against 3–4 ms for a color), and a cache
  would cut it to about 110–160 ms (a 3840×2160 PNG of the same image
  decodes in 105–109 ms at ImageMagick's level 6, 155–162 ms at level 1),
  but every `set` would pay for it: the `png` crate's `Fast` encode of a
  4K buffer takes 75–80 ms of CPU and writes 15.4 MB (`Balanced`: 1.39–1.47
  s, 14.9 MB), about 18% more CPU per change, a
  [release-gate row](../lightest.md), plus 15 MB of disk per output size
  and the invalidation it needs (path, size, mtime, look, buffer size). The
  gap it would shorten is the one [ticket 10](../scoot-integration.md)
  measures on `--tty` ("the gap before the first frame"); if that gap is
  visible there, a cache written lazily (only once a wallpaper has stayed
  up a while) is the option, with these numbers.

### Found along the way

- **inotify merges identical events.** The end-to-end decode count first
  watched `IN_OPEN` alone and read two back-to-back opens as one, so the
  test passed against a build with the hold taken out too. It watches
  `IN_CLOSE_NOWRITE` as well now, which puts a different event between
  two opens; with that, the build without the hold fails it ("left: 2,
  right: 1").
- **An early `set` from a spawned `scootbg set` lands too late to show the
  double decode** in a debug build: process start-up takes long enough
  that the outputs are configured first. The test sends the request from
  its own process the moment the socket listens, which it does before the
  daemon connects to the compositor.
