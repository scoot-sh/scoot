# scootbg

The lightest wallpaper daemon for Wayland, written in Rust, in place of
`awww` (formerly `swww`), `swaybg`, `hyprpaper`, `wpaperd` and friends.
It shows a color or an image on each output, and one command changes it.

It is built for scoot and set up by scoot's own config, but it is not tied
to it: scootbg speaks only standard protocols, so it also runs on any
compositor with `wlr-layer-shell-v1` (sway, niri, Hyprland, river, labwc).

> **Status: early.** The daemon exists (`crates/scootbg/`, with its
> `unsafe` in `crates/scootbg-mem/`): `scootbg daemon` connects, tracks
> every output as it comes and goes, and gives each one a `background`
> layer surface. **Solid colors and images work**: `scootbg set '#rrggbb'`
> or `scootbg set PATH` (PNG, JPEG, WebP) with a fit mode, on every output
> or one (`--output NAME`), and `scootbg clear`, drawn at each output's
> real device pixels, fractional scales included; `query` reports what
> each output shows, and `version` and `kill` work. **The wallpaper is
> restored at the next start**, per profile ([below](#restore)).
> **In scoot, a `[wallpaper]` section is all it takes**: scoot runs
> `scootbg apply-config` with it at start-up and on every reload
> ([below](#apply-config-scoots-wallpaper-section); the section itself is
> in [scoot's configuration reference](../configuration.md#wallpaper)),
> and the Nix modules install scootbg for it.

## What it is for

- **The lowest resource use of any wallpaper daemon.** Not a goal but a
  release gate: v1 does not ship while any competitor beats scootbg,
  beyond a noise margin, on any measure both can do (CPU, idle wakeups,
  memory, peak memory while decoding, binary size, startup). The
  competitors are `swaybg`, `awww`, `hyprpaper`, `wpaperd` and `wbg`, on
  the same machine, with the table published here. Every
  dependency has to justify its bytes. See
  [`backlog/lightest.md`](backlog/lightest.md).
- **Colors and images.** A solid color or a PNG, JPEG or WebP image per
  output, changed live with one command and restored at login. That is
  v1.
- **Seamless in scoot.** A `[wallpaper]` section in scoot's `config.toml`
  is all it takes: scoot starts scootbg and re-applies the section on
  `scootctl reload`. No autostart entry, no session script.
- **Free when idle.** A static wallpaper costs nothing after it is on
  screen: no timers, no frame callbacks, no wakeups, one buffer per output
  at most (outputs of one size showing one image share one), no file
  descriptor held per buffer, and the decoded source image dropped once it
  is scaled. Measured: [the resource budget](#the-resource-budget).
- **GPU-free.** Decoding, scaling and (later) transitions all run on the CPU
  into `wl_shm` buffers, the same bar the compositor holds itself to for
  webtop and no-GPU boxes. A GPU path, if ever, is an optional tier.
- **Sharp at any scale.** Buffers are drawn at the output's real device
  pixels, fractional scales included, never scaled up by the compositor.
  Working, and measured ([below](#measured-so-far)).
- **Scriptable.** A small CLI over a control socket with JSON replies, like
  `scootctl`, so an agent or a script can set, query and clear wallpapers.

Transitions (fade, wipe, grow) and animated images (GIF, APNG, animated
WebP) are the next milestone, not v1. They are the features people move to
awww for, and they are also a steady CPU cost under a software renderer,
so they arrive once the static path is measured and they can be held to a
per-frame budget.

## How it will work

In scoot, the whole setup is one config section (every key, and what
happens when scootbg is missing, in
[scoot's configuration reference](../configuration.md#wallpaper)). scoot
runs `scootbg apply-config` with it
([below](#apply-config-scoots-wallpaper-section)): no autostart entry, no
session script.

```toml
# ~/.config/scoot/config.toml
[wallpaper]
image = "~/Pictures/hills.jpg"   # or: color = "#1e1e2e"
mode = "fill"                    # fill | fit | stretch | center | tile

[wallpaper.output."DP-2"]        # optional, per output
color = "#101014"
```

And one command changes it live, in scoot or anywhere else:

```sh
scootbg set ~/Pictures/city.png                # every output
scootbg set '#1e1e2e'                          # a color: anything starting with '#'
scootbg set ./#draft.png                       # a file whose name starts with '#'
scootbg set ~/Pictures/city.png --output DP-1 --mode fit --fill '#101014'
scootbg query                                  # what each output shows, as JSON
scootbg clear --output DP-1                    # back to the compositor's own background
scootbg daemon                                 # outside scoot: start it yourself
scootbg daemon --profile sway                  # ... restoring and saving the `sway` profile
```

Every command above works today (`set` also takes `--filter
lanczos3|catmull-rom|bilinear|nearest`, and `daemon` `--no-restore`), and
so does the `[wallpaper]` section. The root
[README](../../README.md#scootbg-early) has the details (exit codes, what
`set` waits for). One binary: `daemon` runs the Wayland client, every
other subcommand talks to it over its socket.

- **One `background`-layer surface per output**, anchored to all four edges,
  exclusive zone `-1`, no keyboard interactivity and an empty input region,
  so clicks reach the desktop underneath, as on any other compositor.
- **Solid colors** use `wp_single_pixel_buffer_manager_v1` scaled by
  `wp_viewporter`: no shared memory at all for a whole output, with a 1×1
  `wl_shm` buffer under the viewport where the compositor lacks
  single-pixel buffers, and a full-size `wl_shm` buffer where it has no
  viewporter either. Working, and measured
  ([below](#measured-so-far)).
- **Images** are decoded on a worker thread, never on the Wayland loop,
  once for every output that needs them, scaled once per buffer size, and
  written into an opaque `XRGB8888` buffer at the output's device pixels,
  with its opaque region set, so a compositor can skip drawing anything
  beneath it. With `wp_fractional_scale_v1` and `wp_viewporter` that is
  the surface's logical size times the scale the compositor asks for,
  rounded as that protocol says, under a viewport (1601×1001 for scoot's
  1067×667 at 1.5 on a 1600×1000 output, which scoot draws one to one);
  without them, or for a moment while the compositor has not told the
  surface its new scale, the surface size times the integer scale (the
  larger of `wl_surface.preferred_buffer_scale` and `wl_output.scale`),
  which the compositor scales down, never up. The decoded image is dropped once drawn (an
  output plugged in later shares the pixels of an output of its size
  showing the image, and otherwise reads the file again). **Outputs of one
  size showing one image share one buffer's memory**: one memfd, mapped
  once, with a `wl_buffer` per output over it (one `wl_buffer` on several
  surfaces would leave its releases undefined, `wl_surface.attach` says),
  so a second 4K output costs no second 32.4 MB. EXIF orientation is
  applied as the pixels are packed, with no extra buffer. A file that
  cannot be shown (missing, not PNG/JPEG/WebP, over 16384×16384 pixels,
  truncated or corrupt) is an error reply that changes nothing. Working,
  and measured ([below](#measured-so-far)).
- **Fit modes:** `fill` (cover and crop, the default), `fit` (letterbox
  with a color), `stretch`, `center`, `tile`.
- **Outputs come and go.** A monitor plugged in gets the wallpaper meant
  for it (by connector name, or the "every output" choice), and one unplugged
  frees its buffer (or its share of one: the others keep showing it).
- **Restore.** The last choice per output is kept in
  `$XDG_STATE_HOME/scootbg/PROFILE`, so `scootbg daemon` at login brings
  it back. Working ([below](#restore)).

## Restore

Every `set` and `clear` the daemon records is saved, and `scootbg daemon`
shows it again when it starts.

- **One state file per profile:** `$XDG_STATE_HOME/scootbg/PROFILE`, or
  `~/.local/state/scootbg/PROFILE` when `XDG_STATE_HOME` is unset, empty
  or relative (the XDG base directory spec). With neither it nor `HOME`
  an absolute path there is nowhere to keep state: nothing is restored
  or saved, and the daemon says so on stderr. `scootbg daemon --profile
  NAME` picks the profile, `default` when not given. A state directory
  that already exists but is owned by someone else, or writable by the
  group or others, is a warning at start-up, not a refusal: whoever can
  write there can change which wallpaper is restored (the paths are only
  read, by decoders checked against hostile input), and a group-writable
  directory is normal under a `umask` of 002 with a group per user. A name is 1 to 64
  of `A-Z a-z 0-9 . _ -`, not starting with `.` and without `..`, so it
  is one plain file name; anything else is a usage error (exit 2).
  Profiles, not displays: scoot binds the first free `wayland-N`, so the
  socket name shifts with start order and is no session identity.
  Sessions with different profiles never restore each other's wallpaper;
  two sharing one share it, the last change winning.
- **What is saved** is the daemon's choices as `set` and `clear` made them:
  the one for every output, and each one made with `--output NAME`, by
  connector name, plugged in now or not. A color or a `clear` is saved as
  soon as the daemon has it, an image once it has decoded (one that
  cannot be shown changes nothing, so saves nothing). A request superseded
  by a newer one before it landed saves nothing: the file follows the
  same newest-wins rule as the screen.
- **What survives:** a restore itself writes nothing. Choices for outputs
  not plugged in now, and images that could not be restored, stay saved
  until a `set` or `clear` replaces them; a `set` for other outputs keeps
  them. A `set` or `clear` without `--output` replaces every per-output
  choice, as it does on screen. **At most 256 per-output choices, and
  256 KiB in all**, the limits the reader takes: past them the least
  recently *set* go first (setting an old name again makes it recent),
  with a warning, so the newest always survive. They are written oldest
  first, and a restore keeps that order.
- **Written off the loop, atomically:** a temporary file beside it
  (`.PROFILE.PID.tmp`: whatever is at that name is removed, then the file
  is made with `O_CREAT | O_EXCL`, which never follows a symbolic link),
  `fsync`, `rename` over the old file, `fsync` of the directory; the file
  is 0600, a directory it makes 0700. The write
  runs on a thread started for it, which ends once nothing more waits
  (an idle daemon has one thread); the loop only builds the text and
  hands it over (tens of µs), so a slow disk never stalls it. While a
  write is under way a newer text replaces any that waits, so a burst of
  changes is at most two writes, ending with the last. `scootbg kill`
  waits for a write under way (up to 2 s) before the daemon exits; a
  signal leaves the old file or the new one, whole, and at worst a
  temporary file.
- **Restoring:** the choice for every output first, then each per-output
  one, before the daemon serves any request (so any request is newer).
  Each output is drawn when its `configure` comes; an image is decoded
  once for them. **A saved image that is gone** (checked with one `stat`,
  not a decode) is skipped with a warning on stderr, and that output shows
  the compositor's own background; the daemon starts all the same. One
  that exists but cannot be decoded fails when it is drawn, and says so
  then. `--no-restore` shows nothing at start; the file is still read, so
  a `set` then updates it, keeping the rest.

**The file** is a hand-written line format, versioned
([`dependencies-done.md` §5](backlog/resolved/dependencies-done.md#5-serialization-control-socket-and-state-file)
chose it over TOML, 180 KB lighter):

```text
scootbg-state 1
profile default
fingerprint 9c1e…
all color #1e1e2e
output DP-1 image /home/me/My%20Pictures/hills.jpg fill #000000 lanczos3
output HDMI-A-1 clear
```

- The first line is `scootbg-state` and the version. Each other line is a
  key and its fields, single spaces between. `all` and `output NAME` take
  `clear`, `color #rrggbb`, or `image PATH MODE FILL FILTER` (a path is
  absolute). `profile` names the profile the file belongs to (the file
  name is the authority; a mismatch is a warning). `fingerprint` is the
  fingerprint of the last `[wallpaper]` section `apply-config` applied for
  this profile ([below](#apply-config-scoots-wallpaper-section)): only
  `apply-config` writes it, and every other save keeps it as read.
- **Fields are escaped:** `%`, space and every ASCII control byte (newline
  and tab included) are written `%XX`, so any path or connector name
  round-trips exactly, `#` and spaces included. Other bytes, UTF-8
  included, are written as they are.
- **Read defensively; never fatal.** A malformed line (a field missing or
  extra, a bad escape, a relative path, an unknown mode or filter), one
  whose fields are not UTF-8 once unescaped, or an unknown key is skipped
  with a warning naming its line, and the rest is used. The same key
  twice: the later line counts, with a warning. More than 256 `output`
  lines: the last 256 are kept (lines are oldest first; scootbg never
  writes more). A file over 256 KiB, or whose first line is not
  `scootbg-state` and a version, restores nothing and is replaced at the
  next save. **A later version** (a newer scootbg's file) restores
  nothing and is never written over, so a downgrade cannot destroy what
  the newer build saved. A file that cannot be read (permissions, not a
  regular file) restores nothing, and nothing is saved over it. In those
  two cases **saving is off until the daemon restarts**: stderr says at
  start-up which file to remove or fix, then restart, to save again, and
  `query` reports `"saving":false` meanwhile.
- **Compatibility:** within version 1 a key may be added only if a reader
  that skips it (with its warning) loses nothing it needs; anything else
  bumps the version. `profile` and `fingerprint` are read and kept from
  version 1 on, so the scoot integration writes them with no bump.

## `apply-config`: scoot's `[wallpaper]` section

`scootbg apply-config [--profile NAME] JSON` is the one command scoot runs
(at start-up and on every reload while its config has a `[wallpaper]`
section, and with `{}` on the reload that removes it; see
[scoot's side](../configuration.md#wallpaper)).
You rarely run it yourself, but anything that owns a config can: it is
how a config, rather than a person, sets the wallpaper.

```sh
scootbg apply-config --profile scoot '{"image":"/home/me/hills.jpg","mode":"fill",
  "output":{"DP-2":{"color":"#101014"}}}'
scootbg apply-config --profile scoot '{}'     # the section is gone: clear
```

**The JSON** is the section, one object, `{}` when there is none:

| Key | |
|---|---|
| `image` | an absolute path (scoot resolves `~/` and relative paths against its config file first) |
| `color` | `#rrggbb`; `image` or `color`, never both; neither is nothing (the compositor's own background) |
| `mode`, `fill`, `filter` | with an `image` only, as `scootbg set` takes them: `fill`/`fit`/`stretch`/`center`/`tile`, `#rrggbb`, `lanczos3`/`catmull-rom`/`bilinear`/`nearest` |
| `output` | per-output tables by connector name, each with the five keys above and nothing else; an empty table is nothing on that output |
| `command` | scoot's (where to find `scootbg`): accepted, ignored, and never part of the fingerprint |

- **Each table stands alone**, as a `scootbg set` does: an output's own
  `image` does not take the top level's `mode`.
- **Strict.** Refused, as a usage error (exit 2) that starts and changes
  nothing, so a typo is never a setting silently not applied: an unknown
  key at either level, a key given twice, `null` or a value of the wrong
  type, an array where an object belongs, a relative path, a path with a
  NUL byte or over 4095 bytes, a malformed color, an unknown mode or
  filter, an empty output name, more than 256 outputs, more than 63 KiB
  (64,512 bytes) of JSON, JSON that is not UTF-8.

**Whichever you changed last wins.** The section is applied only when it
changed since the last `apply-config` for that profile:

- **The fingerprint** is SHA-256, in lowercase hex, of a canonical
  encoding of the section: compact JSON, keys in byte order at both levels,
  strings as given (a color's case included) and escaped as `serde_json`
  escapes them, `command` left out, every other key present in the input
  present in it. The sender's key order never matters (scoot's per-output
  tables come from a hash map); a change to any value does, and so does
  writing out a default (`mode = "fill"`), since the section is compared
  as written. It is kept in the profile's state file
  ([Restore](#restore)), which only `apply-config` writes it to.
- **Different** (or never applied): the section is applied like a `set`
  of everything (the top level for every output, replacing every
  per-output choice, then each output's table) and its fingerprint
  recorded in the same write. **The same:** what shows stays, so a
  `scootbg set` made since keeps showing, across restarts and unchanged
  reloads, until the section itself changes.

| Sequence | Result |
|---|---|
| section A, `set X`, restart | A unchanged, so X is restored |
| section A, reload with B, `set X`, restart | B unchanged since its reload, so X |
| section A, reload with B, restart | B shows |
| section removed by reload, later re-added as A | `{}` was recorded at removal, so A differs and shows |
| no daemon yet, section added by reload | `apply-config` starts the daemon |
| an `[autostart]` entry also starts `scootbg daemon` | whichever binds first, the daemon ends up on scoot's profile, so a `set X` from a previous boot is restored either way |

One order it cannot see: the section removed *while scoot is not
running* and re-added unchanged before the next start. No `{}` was ever
applied, so the old fingerprint matches and the last saved state (a
`scootbg set` pick, say) restores. Every row, and this one, is an end-to-end
test on headless scoot (`crates/scootbg/tests/config.rs`).

**The profile.** A running daemon **adopts** the profile of each
`apply-config` it gets: from then on it restores and saves that profile's
state, whatever `--profile` it started with (it says so on stderr); the
state file of a profile adopted again while its last save is still being
written is read once that write is done (waited for up to 2 s), and one
file never has two writers. So an
`[autostart]` `scootbg daemon` that won the race still ends up on scoot's
profile; on adopting, it shows that profile's saved state if the section
is unchanged, the section if not. A request made before the adoption (an
image `set` still decoding) never lands in the new profile.

**Starting the daemon.** With none answering on the socket:

- **A non-empty section:** `apply-config` starts one, the same binary
  (its path while that is still the running file, so `ps`, `pgrep` and
  `pkill` see `scootbg`; `/proc/self/exe` itself if the path now names
  another file or none, an upgrade meanwhile, so a different build never
  runs; the path alone without `/proc`) as
  `apply-config --serve` (internal, not for use by
  hand), with stdin and stdout on `/dev/null`, **stderr where
  `apply-config`'s goes** (scoot's log), and `/` as its working directory.
  It calls `setsid(2)` first: a session and process group of its own, no
  controlling terminal, so a signal to the caller's process group (a
  terminal's hang-up or Ctrl-C, a supervisor or `timeout(1)` stopping the
  job) does not reach it; `apply-config` exits without waiting for it, so
  it is reparented and nothing is left for the caller to reap. It still
  ends with the compositor, whose connection it loses. It starts **from
  the section**: it reads the profile's state without showing it,
  compares fingerprints, and shows the section or the saved state, never
  one and then the other. `apply-config` then sends it the section as
  usual (unchanged by then, so only the reply is new).
- **An empty section** (`{}`, or only `command`): no daemon is started.
  The clear and the fingerprint are written to the profile's state file
  (unless recorded already), holding the display's lock meanwhile, so a
  `scootbg daemon --profile NAME` started later shows nothing, as the
  config asked. A state file that cannot be read, or is a newer scootbg's,
  is not written over: exit 1. (A `scootbg daemon` started in the same
  few milliseconds as that write finds the lock held and exits with
  "already running".)
- **Races** are settled by the lock: two `apply-config`s at once (or one
  and a `scootbg daemon`) make exactly one daemon; a started one that loses
  forwards its section to the winner, as `apply-config` itself does, so the
  config's values are never dropped; if the winner goes away without
  serving (a `{}` being recorded, a daemon that fails before binding), the
  loser tries once more to be the daemon itself. A daemon that is starting
  (its lock taken, its socket bound, its loop not yet serving) is waited
  for: tries every 1 ms for the first 50 ms, then every 15 ms, 5 s in all.
- **Stderr and pipes.** Because the daemon writes where `apply-config`'s
  stderr goes, a caller that reads that stderr to its end through a pipe
  waits for the daemon too: send it to a file or a log.

**Another build.** `apply-config` sends a `version` request first, on the
same connection. A daemon of another version, same protocol: a warning on
stderr, and the section is sent all the same. A daemon speaking another
protocol, or one older than `apply-config` (it answers `unknown request`):
an error naming both builds, exit 1, and what to do (`scootbg kill`, then
reload). A daemon is never restarted behind your back.

**The reply** comes once every output shows what it should and the
compositor has processed it, as for `set`; `apply-config` prints nothing on
success. An image in the section that is not a file is reported at once
(exit 1, which one on which output); the rest of the section is applied,
and the choice stays saved. **Every `apply-config` reports it until the
file is back**, the unchanged ones included (so the first one after a
cold start does too), and the first one after the file is back shows it:
an image the section chose that is still saved as the section's choice
there, but not showing, is checked with one `stat` and put back, at the
generation it was chosen at. **A `set` made since always stands**: a
`set` or `clear` for that output, or for every output, replaced the saved
choice, so the entry is no longer the section's and is neither put back
nor reported (and putting back at the old generation can never undo a
newer request still decoding).

**Exit status:** 0 applied, or unchanged, and on screen (or `{}` recorded
with no daemon); 1 no daemon could be started or reached within 5 s, no
reply within 30 s, the daemon closed the connection before answering
(`scootbg kill`, a crash), a daemon from another protocol or too old, an
image in the section that is not a file, drawing failed, or the state file
could not be written; 2 a usage error, a refused section included.

**Timing** (release build, `scoot --headless --outputs 2`, a color, two
runs; [the record](backlog/resolved/scoot-integration-done.md#review-of-pr-293)):
from running `apply-config` with no daemon to the section on screen,
medians 6.90 and 5.61 ms (4.85–10.54; a plain `scootbg daemon` then
`set`: 4.27 and 4.01 ms); unchanged on a running daemon 1.96 and 1.86 ms;
changed 2.55 and 2.31 ms (a `set`: 2.39 and 2.34 ms); recording `{}` with
no daemon 2.52 and 2.37 ms. scoot never waits on it: it spawns it.

## The control protocol

Every command but `daemon` is one request on the daemon's socket,
`$XDG_RUNTIME_DIR/scootbg-NAME.sock` (`NAME` the last component of
`$WAYLAND_DISPLAY`), so a script or an agent can skip the CLI: one JSON
object per line each way, each request naming the protocol it speaks.

```text
{"protocol":1,"type":"set","color":"#1e1e2e"}                 -> {"type":"ok"}
{"protocol":1,"type":"set","color":"#1e1e2e","output":"DP-1"} -> {"type":"ok"}
{"protocol":1,"type":"set","image":"/abs/a.jpg"}              -> {"type":"ok"}
{"protocol":1,"type":"set","image":"/abs/a.jpg","mode":"fit","fill":"#101014","filter":"lanczos3","output":"DP-1"}
                                                              -> {"type":"ok"}
{"protocol":1,"type":"clear"}                                 -> {"type":"ok"}
{"protocol":1,"type":"clear","output":"DP-1"}                 -> {"type":"ok"}
{"protocol":1,"type":"query"}                                 -> {"type":"outputs","outputs":[...],"saving":true,"profile":"default"}
{"protocol":1,"type":"version"}                               -> {"type":"version","protocol":1,"version":"..."}
{"protocol":1,"type":"kill"}                                  -> {"type":"ok"}
{"protocol":1,"type":"apply-config","profile":"scoot","config":{"color":"#1e1e2e"}}
                                                              -> {"type":"ok"}
anything wrong                                                -> {"type":"error","message":"..."}
```

- **`apply-config` is additive to protocol 1**: a request type added, no
  existing one changed. A daemon that predates it answers `unknown request
  `apply-config``, which the `apply-config` command reports as such (see
  [above](#apply-config-scoots-wallpaper-section)). `profile` (a profile
  name) and `config` (the section, validated strictly, as above) are both
  required; on any other request they stay ignored unknown fields. It
  answers like a `set`, once every output shows what it should, or at once
  with an error when an image in the section is not a file (the rest is
  applied).

- `set` and `clear` answer once every targeted output shows the change and
  a `wl_display.sync` sent after the commits has come back, so the
  compositor has it; the daemon never blocks on that, other clients are
  served meanwhile. Requests sent behind one on the same connection are
  answered after it, in order. An output unplugged before its commit is
  left out of the wait; an output whose surface is not configured yet is
  waited for, until a `wl_display.sync` sent after scootbg made that
  surface comes back: one the compositor has not configured by then no
  longer holds up replies (said on stderr, once per surface), and is drawn
  when its `configure` comes. No compositor checked is that slow (scoot
  and sway configure before the round trip returns). An output scootbg gave up on (`gave-up` in `query`, said on
  stderr) is left out and shows nothing, and the reply is still `ok`. A client that hangs up before its reply loses only the
  reply: the change still happens.
- `set` takes a `color` or an `image`, never both. An `image` is an
  absolute path (the daemon's working directory is not the client's;
  `scootbg set` resolves a relative one before sending it) to a PNG, JPEG
  or WebP; `mode` (`fill`, `fit`, `stretch`, `center`, `tile`), `fill`
  (`#rrggbb`) and `filter` (`lanczos3`, `catmull-rom`, `bilinear`,
  `nearest`) may be left out, for `fill`, `#000000` and `lanczos3`, and
  are refused with a `color`.
- **An image changes nothing until it has been decoded.** One that cannot
  be shown gets an error reply saying why (no such file, not a regular
  file, not an image scootbg reads, image too large, truncated or
  corrupt), and every output keeps what it showed. Decoding runs on a
  worker thread; other requests are served meanwhile.
- **The newest request wins**, whatever order the work finishes in: a
  `set` or `clear` sent after an image's `set` is never undone when that
  image finishes decoding. An image request that newer ones have replaced
  on every output it asked for, before it was decoded or while, is
  answered `ok` once what replaced it is on screen, as a replaced color
  is, and changes nothing (it may never be decoded at all). At most 32 image
  requests wait at once; one more is refused ("too many images are
  waiting"), nothing changed.
- A request refused outright changes nothing: an unknown `output` name
  (the name must belong to an output present now), a `color` that is not
  `#rrggbb`, a relative `image` path, an unknown `mode` or `filter`. A
  draw that fails on an output (a buffer too large for `wl_shm`, out of
  memory) is different: the choice is recorded and every other targeted
  output shows it; the reply is an error once they have, and the daemon's
  stderr says which output failed and why.
- `query`'s `surface.scale` is the scale an image, or a color on the
  full-size fallback, is drawn at on that output (`1.5` from
  `wp_fractional_scale_v1`, else an integer; a color on a single-pixel or
  1×1 buffer is drawn at 1 whatever it says) and
  `surface.pixels` the size of an image's buffer in device pixels, both
  `null` until the surface is configured; the top-level `scale` stays
  `wl_output`'s integer.
- `query`'s `shows` is `{"color":"#rrggbb"}` (lowercase),
  `{"image":"/abs/path","mode":"fill","fill":"#rrggbb","filter":"lanczos3"}`,
  or `null`. Every key of an entry is always present; keys may be added
  within protocol 1, none removed or changed.
- A choice for every output is kept for outputs plugged in later; a choice
  for one output is kept by its name, across unplugging it, and across a
  daemon restart ([Restore](#restore)).
- `query`'s `draw_failed` (per output) is `true` when the last attempt to
  draw what that output should show failed (stderr says why), so a
  `shows` of `null` from a failure is told apart from a `clear`; the
  next request for it, or a new size, retries. The top-level `saving` is
  `false` while changes are not saved for the next start
  ([Restore](#restore)), and `profile` is the profile whose state is
  restored and saved: the daemon's `--profile`, or the last one an
  `apply-config` made it adopt.
- **An image `set` sent before its outputs are configured** (as the
  daemon starts) waits to be decoded until none of the outputs it targets
  is about to be configured, bounded by a round trip the daemon has
  already sent, so the file is decoded once, for them (it used to be
  decoded once to check it and again for the `configure`).

## The resource budget

What scootbg costs, measured against `scoot --headless` (a debug build,
scale 1) on a 4-CPU Claude Code web container with no GPU, release
builds; the method and every raw number are in
[memory-and-idle-done.md](backlog/resolved/memory-and-idle-done.md#measurements).
The image is a 6000×4000 JPEG, `fill`.

| What | Result |
|---|---|
| **Idle**, 60 s windows, a color or the image on 3840×2160 | 0 context switches and 0 CPU ticks in every window (`/proc`); `perf stat` counts no task-clock, context switch, page fault or wakeup: the daemon never runs. One thread. |
| **Memory at rest with the image**, 1× 1920×1080 | RSS 12.3–12.4 MB, PSS 7.1–7.3 MB (the 8.1 MB buffer is shared with the compositor, so PSS counts half); anonymous 456 kB, `[heap]` 40 kB; 8 fds |
| 1× 3840×2160 | RSS 36.7–36.8 MB, PSS 19.4–19.5 MB; anonymous 564–572 kB, `[heap]` 40 kB; 8 fds |
| **2× 3840×2160** | **RSS 36.7–36.8 MB, PSS 19.4–19.6 MB**, the same as one output: both show one 32.4 MB buffer's pixels (69.2 MB and 35.7–35.8 MB before ticket 8) |
| Memory at rest with a color (a single-pixel buffer: no shared memory at any size), on 3840×2160 | RSS 4.0 MB, PSS 2.8–2.9 MB; anonymous 208–216 kB. 3.6–3.7 MB before ticket 9: saving the choice touches 340–370 kB more of clean code pages (the daemon's own, libc's thread start), not heap |
| **Peak while changing** the image on 2× 4K | 120.4–120.7 MB (152.9–153.0 MB before ticket 8); 87.9–88.1 MB for a first image |
| **Startup**, `scootbg daemon` to its first answer | 1.8–2.5 ms |
| to a color on screen (a `set` sent at once, answered after the commit and a round trip) | 3.0–4.1 ms |
| to the image on one 4K output, likewise | 450–476 ms, one decode (714–820 ms for the build before ticket 9 in the same runs, two decodes; a `set` once the output is configured takes 437–468 ms) |
| **to a restored image** on one 4K output (the state file names the JPEG) | 451–497 ms to the first `query` that reports it shown: one decode ([restore-state-done.md](backlog/resolved/restore-state-done.md#measurements)) |
| Saving a choice | 23–33 µs on the loop, medians (build the text, hand it over); the atomic write on a thread of its own, medians 334–455 µs, worst 1.0–54.5 ms per 200, on ext4 |
| **`apply-config`**, a color on 2× 1600×1000: with no daemon (it starts one) / unchanged / changed, to on screen | medians 5.61–6.90 / 1.86–1.96 / 2.31–2.55 ms over two runs (a `set` 2.34–2.39 ms; `daemon` then `set` from cold 4.01–4.27 ms); the daemon it starts idles like any other: 0 context switches and 0 CPU ticks in 30 s, 1 thread, 8 fds, RSS 3,968 kB ([record](backlog/resolved/scoot-integration-done.md#review-of-pr-293)) |
| File descriptors | 8 whatever is shown: a buffer's memfd is closed once the compositor has it |

## Measured so far

Release build, against `scoot --headless --outputs 2` (1600×1000 each,
a debug build of scoot; the image rows on one 3840×2160 output), on a
4-CPU Claude Code web container; the records, with the method and every
raw number, are in
[solid-color-done.md](backlog/resolved/solid-color-done.md#measurements),
[images-decode-and-fit-done.md](backlog/resolved/images-decode-and-fit-done.md#measurements),
[hidpi-fractional-scale-done.md](backlog/resolved/hidpi-fractional-scale-done.md#measurements)
and [memory-and-idle-done.md](backlog/resolved/memory-and-idle-done.md#measurements).
Not yet against competitors: that is [lightest.md](backlog/lightest.md).

| What | Result |
|---|---|
| Stripped binary | 1,684,328 B with `apply-config` (+114,688 B: by symbol, spawning a process through `std::process` about 27 KB, the strict section parse about 23 KB, the client half about 20 KB; 8,192 B of it the fixes from its review); 1,569,640 B with images and restore (1,557,352 B at ticket 9 before its review fixes, 1,516,392 B before ticket 9, 1,500,008 B at ticket 6; 783,072 B with colors only then); links only `libc.so.6`, `libm.so.6` and `libgcc_s.so.1` |
| `set` of a 6000×4000 JPEG onto a 3840×2160 output, request to reply, ×3 | 397.0–433.6 ms, 390–420 ms of CPU (PNG 408.1–417.6 ms; WebP 1,218.0–1,289.4 ms); peak RSS 120.5–120.6 MB with the previous wallpaper still mapped (88.0 MB for a first set; PNG 120.4–120.6 MB; WebP 142.0–142.1 MB) |
| After it, idle 30 s | 1 thread, heap 372–568 kB, one 32.4 MB buffer; 0 context switches, 0 CPU |
| A few hundred bytes claiming 16384×16384 (PNG, JPEG, WebP) | refused in under 1 ms; peak RSS within 72 kB of before |
| Idle with a color set, 30 s ×3 | 0 context switches, 0 CPU ticks; RSS 2,720 kB, PSS 1,524 kB, 1 thread |
| PSS with a color, 2 outputs: single-pixel / 1×1 shm / full-size shm | 1,556–1,560 / 1,556–1,560 / 7,808 kB (14,060 kB once a change left a spare buffer per output, until ticket 8: no spare is kept now) |
| `set`, request to reply, 10,000 changes ×3 | median 400–433 µs, p99 1.6 ms; no memory growth |
| The same JPEG at scale 1.5 on 3840×2160: buffer, and request to reply ×3 | 33,177,600 B (was 58,982,400 B at `wl_output`'s 2); 420.0–434.4 ms (was 494.9–507.3 ms). On 1600×1000: 6,410,404 B (was 11,387,024 B) |

## Relation to scoot

- **scoot drives scootbg, never the reverse.** At startup and on each
  reload, while `[wallpaper]` exists, scoot spawns one command,
  `scootbg apply-config`, with the section's values
  ([above](#apply-config-scoots-wallpaper-section)), one run at a time.
  It starts the daemon if none is running, otherwise hands the values
  over, and changes the wallpaper only if the section itself changed.
  scoot never waits on it from its event loop: the run is reaped, and its
  exit status logged, when it ends. scoot depends only on scootbg's CLI,
  not its crate, and scootbg knows nothing about scoot's config, so each
  stays usable without the other. See
  [scoot's configuration reference](../configuration.md#wallpaper) and
  [the item's record](backlog/resolved/scoot-integration-done.md).
- **Config versus `scootbg set`: whichever you changed last wins.** Edit
  `[wallpaper]` and the config's wallpaper shows. Run `scootbg set` after
  that and your choice shows, across restarts and unrelated reloads, until
  you next change `[wallpaper]` itself.
- **Packaging.** `scootbg` is its own package, like `scootctl`
  (`packages.<system>.scootbg`, Linux only, and `pkgs.scootbg` from the
  flake's overlay). scoot runs it from `PATH` (or `[wallpaper] command`).
  The home-manager module installs it and points `command` at it whenever
  its settings have a `wallpaper` table; the NixOS module installs it
  system-wide whenever `programs.scoot.enable` is on
  (`programs.scoot.wallpaper.enable`). See [docs/nix.md](../nix.md).
- scoot's `[appearance] background_color` stays: it is the frame clear
  color, what shows with no wallpaper client at all. scootbg draws over it
  on the background layer.
- The compositor's `scootctl screenshot` and `ext-image-copy-capture-v1`
  already include layer surfaces, so screenshots show the wallpaper with no
  extra work.
- scoot's headless backend is the test harness: start
  `scoot --headless --outputs 2`, run scootbg in it, and check real pixels
  with `scootctl screenshot --output N`, the way `scripts/smoke-test.sh`
  already checks the compositor.
- A wallpaper per workspace is planned through `ext-workspace-v1`, a
  standard protocol, so it is not scoot-only either.

## Licensing

MIT, like the rest of scoot. awww/swww are GPL-3.0 and may inspire the
design (their feature set is the bar to meet), but no code, shaders or
assets are copied from them, nor from any other GPL wallpaper daemon.
Every dependency's licence is checked before it is added. All of the
chosen set is permissive and MIT-compatible: MIT, Apache-2.0, Zlib,
Unlicense, and BSD-3-Clause OR Apache-2.0 for the scaler,
`pic-scale-safe`, whose notice a binary package carries alongside the
others. The set and every licence are recorded in
[`backlog/resolved/dependencies-done.md`](backlog/resolved/dependencies-done.md#8-licences).

## Standards

scootbg is held to the same bar as the compositor (see the root
`CLAUDE.md`): no allocation on per-frame paths, crash or hang paths treated
as data loss, edge cases (zero outputs, hotplug mid-decode, a 20000×20000
image, a truncated file) considered explicitly, benchmarks before and after
anything on a hot path, and the full per-feature cycle with independent
review before merge. On top of that, as the user set it, speed and
safety are not traded against each other:

- **Pure Rust, no C.** No C calls and no C dependencies: no `libc` crate,
  no `-sys` crate that links a library, no build script that compiles C.
  The honest exception is the Rust standard library itself, which on
  `*-linux-gnu` links glibc, `libm` and `libgcc_s`. `wayland-backend`'s
  and `wayland-sys`'s C-bringing features stay off: `client_system`
  links libwayland, and `log`, although it only adds the pure-Rust `log`
  crate as a dependency, makes `wayland-backend/build.rs` compile two C
  shims (`src/sys/*/log_shim.c`) with `cc`, even for the pure-Rust
  backend. (Checked against the pinned fork, `70f81e00`.) Without
  `log`, the backend reports errors with `eprintln!`, so the daemon
  guards against that instead: see `crates/scootbg/src/daemon/crash.rs`.
- **`#![forbid(unsafe_code)]` in `scootbg`.** The only `unsafe` is
  isolated in one small crate, `scootbg-mem`:
  - the global allocator that returns large blocks to the kernel;
  - the `wl_shm` buffer mapping.

  Signals are deliberately not caught: SIGTERM and friends kill the daemon
  with their default action, and the stale socket file it leaves is
  harmless (a lock, not the file, says a daemon is alive). Catching them
  would have taken a third `unsafe` module on unstable rustix APIs; see
  [the record](backlog/resolved/crate-and-daemon-done.md#departures-from-the-plan-and-why).

  It is named for what it owns rather than `-sys`, which by Cargo
  convention means bindings to a native library, the one thing it
  exists to avoid. Every `unsafe` block there has a written safety
  argument.
- **Malformed input never panics where it can be avoided.** Sizes and
  file contents are validated before they reach a decoder or the scaler,
  and a bad file is an error reply. A panic aborts the daemon.
- **Dependencies are weighed on safety as well as weight:** how much
  `unsafe` they carry and what for, whether they are fuzzed upstream, and
  their RustSec record. Where a dependency is not fuzzed upstream,
  scootbg fuzzes the path through it.

The audit behind these rules, and the one measured cost they carry (CPU
per change from the safe scaler), is in
[`backlog/resolved/dependencies-done.md`](backlog/resolved/dependencies-done.md).
