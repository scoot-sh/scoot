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
  [scoot-integration-done.md](scoot-integration-done.md) (whichever you changed
  last wins) survives restarts. Only `apply-config` writes it.
- **One state file per profile** (`--profile NAME`, default `default`),
  not per display: scoot binds the first free `wayland-N`, so the socket
  name shifts with start order and stale sockets and is no session
  identity. scoot passes `scoot` (or `scoot-nested`), a sway user's
  autostart can pass `sway`, and sessions with different profiles never
  restore each other's wallpapers. A daemon takes its profile from its own
  `--profile`, and adopts the profile of each `apply-config` it receives (see
  [scoot-integration-done.md](scoot-integration-done.md)), so a redundant
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
  ([scoot-integration-done.md](scoot-integration-done.md#what-ticket-9-provides)).
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
- **The scaled-buffer cache is not built.** Decoding and scaling are
  nearly all of a restore (451–497 ms in all, against 3.0–4.1 ms to a
  color on screen). A cache would cut that to about 160 ms: on a running
  daemon, a `set` of a 3840×2160 PNG of the same image (no scale, decode
  only) takes 155–162 ms request to reply when it is compressed as fast as
  the `png` crate's `Fast` would write it (ImageMagick level 1, 15.2 MB),
  105–109 ms at level 6 (13.8 MB), against 437–468 ms for the 6000×4000
  JPEG. But every `set` would pay for it: the `png` crate's `Fast` encode
  of a 4K buffer takes 75–80 ms of CPU and writes 15.4 MB (`Balanced`
  1.39–1.47 s for 14.9 MB), about 18% more CPU per change, which is a
  [release-gate row](../lightest.md); on top come 15 MB of disk per output
  size and the invalidation a cache needs (path, size, mtime, look, buffer
  size). The gap it would shorten is the one
  [ticket 10](scoot-integration-done.md) measures on `--tty` ("the gap
  before the first frame"). If that gap is visible there, the option is a
  cache written lazily (only once a wallpaper has stayed up a while), and
  these are its numbers.

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

### Measurements

**Setup.** The Claude Code web container of ticket 8 (x86_64, 4 CPUs, no
GPU, root, ext4 on a virtual disk), the same scripts extended (`lib.py`
unchanged; `restore_startup.py`, `opens.py`, `opens_restore.py`,
`idle_check.py`, `maps_diff.py`, `cache_cost.py`, in the session's scratch
record, not the repository). Release builds, stripped: **base** is
`origin/main` at `c5f5454`, built in its own worktree and
`CARGO_TARGET_DIR` (sha256 `df09ef3b…`, 1,516,392 B); **final** is
`a2595ca` (`473f9343…`, 1,557,352 B). The compositor is a debug
`scoot --headless --width 3840 --height 2160 --outputs 1`, empty config,
scale 1. The image is ticket 8's 6000×4000 JPEG (same recipe, 8,851,735
B), `fill`.

#### Startup

From `Popen` of `scootbg daemon`: for an **early `set`**, to the reply to
a `set` of the JPEG sent the moment the socket takes a connection (the
reply comes after the commit and a round trip); for a **restore** (a
state file naming the JPEG for every output, `XDG_STATE_HOME` pointed at
it), to the first `query` that reports the output showing it (polled
every 0.5 ms), i.e. its first commit. Base and final interleaved in each
round, 5 rounds per run, two runs:

| | run 1, ms (CPU ms) | run 2, ms (CPU ms) |
|---|---|---|
| base, early `set` | 821.9, 939.9, 969.5, 984.7, 791.3 (760–960) | 819.5, 713.6, 724.8, 736.8, 757.7 (690–800) |
| final, early `set` | 644.4, 578.2, 659.6, 613.5, 474.4 (460–630) | 456.5, 450.3, 467.5, 475.8, 473.6 (440–460) |
| final, restore | 638.1, 619.3, 603.1, 555.1, 641.1 (580–670) | 451.4, 477.4, 459.5, 476.2, 497.2 (470–500) |

Run 1 was slow for both builds alike (the host, not the code: nothing
else ran in the container, and base itself took 791–985 ms against
714–820 in run 2 and 642–720 in ticket 8), so run 2 is the one quoted;
final is 0.6–0.8 of base in both. An earlier run on the pre-commit tree (the same code, a
different binary hash) gave base 705.1–717.9, early `set` 450.4–490.8,
restore 443.3–476.6. Ticket 8 measured base at 642–720. Peak RSS is
unchanged, 87.7–88.2 MB in every row. **A restore, and an early `set`, now
cost what a `set` on a running daemon does** (437.3–467.5 ms, below).

Ticket 8's own `startup.py` (the same three measures, `XDG_STATE_HOME`
set to an empty directory so there is nothing to restore), base and
final interleaved, 5 rounds: to the first answer, base 2.1, 2.0, 2.3,
2.9, 2.1 ms, final 2.0, 2.1, 2.0, 2.0, 2.8; to a color on screen, base
3.3, 3.6, 4.5, 4.2, 3.6, final 3.8, 4.1, 6.9, 3.3, 3.5 (the save's
thread start is some 25 µs of that); to the JPEG, base 765.9, 693.5,
733.0, **461.7**, 700.1, final 459.7, 455.0, 443.2, 471.7, 462.4. Base's
461.7 is the race the hold removes: that once, its `set` was read after
the `configure`, and it decoded once.

#### Opens of the file

`strace -f -e trace=%file,clone,clone3` on the daemon, counting opens of
the JPEG (rustix opens with `open(2)` on x86_64), 3 runs each:

| | base | final |
|---|---|---|
| `set` sent as the daemon starts | 2, 2, 2 | **1, 1, 1** |
| `set` once the output is configured | 1 | 1 |
| restore | (none) | **1, 1, 1**: one `statx` on the loop (the presence check), one `open` on the decode thread |

End to end, in debug builds: `tests/restore.rs` counts opens
by inotify for a restore, an early `set` (three rounds), and a restore on
two outputs of different sizes on sway, all 1. With the hold taken out
(`trial_targets` never returning `None`), the early-`set` test fails with
2 in debug, and `opens.py` counts 2, 2, 2 for a release build of it.

#### Saving

`state::bench` (an `#[ignore]`d test, release, `SCOOTBG_BENCH_DIR` on
the container's ext4), a 308-byte file (an image for every output, three
per-output choices, a 64-byte fingerprint), 200 rounds each; three runs
at `a2595ca`, plus one on the tree before it:

| | medians, µs | worst, µs |
|---|---|---|
| on the loop: build the text | 0.8, 0.8, 0.8, 1.4 | 3.8–31.7 |
| on the loop: hand it over (starts the thread) | 25.3, 22.2, 23.4, 31.6 | 66.4–81.1 |
| on the thread: write atomically (temp, `fsync`, `rename`, dir `fsync`) | 334.0, 407.8, 454.5, 355.7 | 965.9, 1,511.2, 4,694.0, **54,498.4** |

That worst case, 54.5 ms of one `fsync`, on a disk with nothing else
writing, is why the write is not on the loop: a busy or network disk
stalls far longer, and the loop's cost is the tens of µs above.

#### Idle and memory

Once settled (one thread, context switches still for 1 s), a 30 s window,
then `/proc`; 3 rounds, base and final side by side:

| | base | final |
|---|---|---|
| context switches, CPU ticks in 30 s (color, image) | 0, 0 | 0, 0 |
| threads, fds | 1, 8 | 1, 8 (the save thread has ended) |
| color: RSS kB | 3,592, 3,644, 3,644 | 3,956, 3,968, 4,012 |
| color: anonymous kB | 196–200 | 208–216 |
| image: RSS kB | 36,704, 36,724, 36,780 | 36,636, 36,692, 36,700 |
| image: anonymous kB | 564–568 | 360–368 |

With a color the daemon holds 340–370 kB more, all of it clean,
file-backed code (per mapping, `maps_diff.py`: scootbg's own text +232
kB, `libc.so.6` +192 kB, `libm.so.6` +16 kB): the save, the first thing a
color-only daemon does that starts a thread and writes a file, touches
code a color never did before. It is reclaimable page cache, not heap
(anonymous memory is within 20 kB), and with an image, whose decode
thread touched it already, there is no difference.

#### The cache decision

On a running daemon, request to reply, 5 rounds: the 6000×4000 JPEG
437.3–467.5 ms (440–450 ms CPU); a 3840×2160 PNG of it, `stretch`,
105.1–109.2 ms at ImageMagick's compression level 6 (13,776,035 B) and
155.2–162.3 ms at level 1 (15,218,401 B). The `png` 0.18 crate encoding
that 4K image (a throwaway program outside the repository, release, 3
runs each): `Fast` 75.4–80.0 ms, 15,392,016 B; `Balanced` 1,392–1,469
ms, 14,853,309 B; `High` 1,392–1,636 ms, 14,853,307 B. See
[Departures](#departures-from-the-plan-and-why).

#### Binary

Release, stripped: 1,516,392 B before, 1,557,352 B after (+40,960). Links
`libgcc_s`, `libm` and `libc` only; no `libc` crate in the normal tree.

### Verified where

On the container above, at `a2595ca` for the code (the docs-only commit
after it adds this record, the numbers, and help-text wording).

- `SCOOTBG_REQUIRE_SCOOT=1 SCOOTBG_REQUIRE_SWAY=1
  SCOOTBG_TEST_SWAY=…sway-1.12/bin/sway devenv shell -- soft-egl cargo
  nextest run -p scootbg -p scootbg-mem`: every test passed; the two
  skipped are the `#[ignore]`d benchmarks.
- `cargo test -p scootbg -p scootbg-mem` (same variables): every suite
  passed, `tests/restore.rs` 9 of 9.
- `cargo clippy -p scootbg -p scootbg-mem --all-targets -- -D warnings`,
  `cargo fmt --check -p scootbg -p scootbg-mem`, `RUSTFLAGS="-D warnings"
  cargo build --release -p scootbg`: clean.
- `nix build .#scootbg` in the sandbox: built.
- **The end-to-end tests catch what they claim**, checked by breaking the
  code: restore not filling the saved table fails four of the restore
  tests (the absent output's entry, `--no-restore` keeping the state, the
  moved image staying saved, and the survive-a-restart test's later
  checks); ignoring `--no-restore` fails its test; taking the hold out
  fails the early-`set` decode count (after the inotify fix above).

### Not verified, and why

- **`--tty`, the dev VM, a GPU**: not reachable from this container. The
  visible gap before the first frame at login is ticket 10's to measure
  there.
- **A slow or network disk**: the write is off the loop by construction,
  and its worst case here was 54.5 ms; no slower disk was available.
- **A compositor slow to configure, or configuring outputs one per
  batch**: the hold ends when the round trip already sent comes back, as
  a reply's wait does; neither compositor checked is slow, and on both a
  restore on several outputs decoded once. Unit-tested in the model (`Output::coming`
  through every state) and in `Jobs`.
- **The `profile` mismatch warning and the unreadable-file path** (a
  permission-denied state file) are exercised by reading only: as root,
  permission bits do not bind here, and a directory in the file's place is
  the unit test's stand-in.
- **`cargo nextest run --workspace`, `nix flake check`, macOS `cargo
  check`**: no compositor code changed; CI runs them.

### For the next tickets

- [scoot-integration-done.md](scoot-integration-done.md): the fingerprint and
  profile lines exist, are read and kept by every save, and need no format
  bump; `--profile` exists and is validated; `restore::load` and
  `restore::apply(…, show)` are separate, so `apply-config` can compare
  before showing anything; adopting a profile mid-life must also
  reconcile every output, which start-up does not need; `write_atomic` is
  public for `apply-config '{}'` with no daemon. The details are in its
  [What ticket 9 provides](scoot-integration-done.md#what-ticket-9-provides).
  The start-up gap to measure on `--tty` is 450–500 ms with a 4K JPEG
  here, nearly all decode; the cache numbers above are the option if it
  shows.
- [lightest.md](../lightest.md): startup with an image is now one decode
  (450–476 ms early `set`, 451–497 ms restore, on 4K). A color-only daemon
  is 340–370 kB larger in RSS after its first save (clean code pages, not
  heap); the binary is 40,960 B larger.
- **Observability, [testing-done.md](testing-done.md):** a restored image that
  exists but will not decode used to show `shows: null` in `query`, the
  same as a `clear`. Review of PR #290 added `draw_failed` (below), which
  tells the two apart; `query` still does not say *why* (stderr does), and
  a failed draw on a live `set` has no end-to-end `draw_failed` check yet.
- [config-and-rotation.md](../config-and-rotation.md): if a config file
  ever ships, TOML for it and the state file together may cost less than
  two formats (dependencies-done.md §5); the state format's versioning
  rule is above.

## Review of PR #290

Review found nothing blocking; five findings were fixed in follow-up
commits on the same branch (the sixth, `--no-restore` then `set --output
X` keeping the old every-output entry, is the documented, deliberate
behavior, and stays).

1. **The temporary file followed a symbolic link** (confirmed live by
   review). `write_atomic` opened `.PROFILE.PID.tmp` with `create` and
   `truncate`, so a link placed at that name made the write truncate and
   overwrite whatever file the user can write, keeping that file's mode,
   and the `rename` then moved the link into place. Now whatever is at the
   name is removed first and the file is made with `create_new`
   (`O_CREAT | O_EXCL`, mode 0600), which never follows a link, dangling or
   not; a failure to create it removes nothing that is not ours. Unit
   tests: a link to a victim file (untouched, still 0644; the state file a
   fresh 0600 file), a dangling link (its target not created), a leftover
   temporary file (replaced). The link test fails against the old open
   flags ("untouched": left `state\n`).
   **And the directory:** a state directory that already exists and is
   owned by another uid, or writable by the group or others, is a warning
   at start-up (one `rustix::fs::stat`, `rustix::process::getuid`; the
   `process` feature was already enabled in the tree). **A warning, not a
   refusal:** whoever can write there can replace the state file, which
   only picks a wallpaper (paths are opened read-only, and decoded by
   decoders checked against hostile input), while a group-writable
   directory is the norm under a `umask` of 002 with a group per user;
   refusing would cost those users their wallpaper for no safety gained.
   The directory scootbg makes itself is 0700. Unit-tested, and end to end
   (a 0777 directory: the warning, and restore and save still work).
2. **The writer and the reader disagreed on limits.** The reader takes
   256 `output` lines and 256 KiB, but `encode` wrote the saved table
   unbounded, in the order names were first chosen, so past 256 connectors
   the newest entries were the ones dropped at every start. Now `encode`
   takes the per-output choices oldest first (by the generation they were
   set at, so setting an old name again makes it recent), keeps whole
   lines newest first while both limits allow, and writes them oldest
   first, so a restore gives them generations in that order and recency
   survives restarts; the saved table forgets what the file dropped, with
   a warning, so the two agree. The reader, past 256 lines, now keeps the
   last ones too (a hand-edited file). An `all` line longer than the whole
   file (only a hand-edited file can make one: a request's path is at most
   64 KiB, 192 KiB escaped) is left out, with a warning, rather than write
   a file that restores nothing. Unit tests: the line limit, the byte limit
   with 20 KiB paths (read back whole, no warning), the huge `all` line,
   the longest request path fitting, and least-recently-*set* eviction
   through `Saved`.
3. **Saving turned off silently.** With a newer-version or unreadable
   state file, `set` worked on screen and saved nothing, and only the
   start-up warning said so, without saying what to do. The start-up
   messages now name the file and the recovery ("remove or rename PATH
   … then restart `scootbg daemon`, to save again"; "fix or remove PATH
   …"; for no state directory, set one and restart), and `query` has a
   top-level `"saving"`, `false` while changes are not saved: additive to
   protocol 1 (keys may be added). End to end: `saving` false with a
   newer file, true once a broken one is to be replaced.
4. **A restored image that exists but will not decode** showed `shows:
   null`, like a `clear`. Added `draw_failed` to each `query` entry: the
   model's existing "the last draw failed" flag (`Output::has_failed`),
   one bool, set by every draw failure and cleared by the next request
   for the output or a new size, so it needed no new state and no new
   failure path. End to end: a restored PNG corrupted since reads
   `draw_failed: true, shows: null`, and a `set` clears it. What is left
   (no reason in `query`; no end-to-end check for a live `set`'s failed
   draw) is in [testing-done.md](testing-done.md) and "For the next tickets".
5. **Help text:** `scootbg daemon --help` now says the `~/.local/state`
   fallback applies when `XDG_STATE_HOME` is unset, empty or relative, as
   the code and docs do, and describes the saving-off case; `scootbg
   query --help` lists `draw_failed` and `saving`.

The binary grows 12,288 B more (1,569,640 B stripped), from the
limit-fitting encode, the directory check and the new `query` fields;
still `libc.so.6`, `libm.so.6` and `libgcc_s.so.1` only, no `libc`
crate. Nothing on the loop's hot path changed: the extra work is in a
save (a sort of at most 256 names, per `set`) and at start-up (one
`stat`). `state::bench` at `68ab299`, same setup as above: building the
text median 1.2 µs (0.8 before), handing it over 19.3 µs, the atomic
write on its thread median 346.3 µs, worst 1,031.4 µs of 200.
