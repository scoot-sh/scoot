---
title: Modules
description: "Every module — clock, workspaces, title, volume, network, brightness, battery, tray, media, bluetooth, power — plus pointer, popups, tooltips, button, push and exec."
---

One page per module family would be thirteen pages; instead every module lives here, each with its own section, so the whole set is one search away. Each module is placed by id in `--left`/`--center`/`--right` or the file's `left`/`center`/`right` lists.

## Modules

| Id | Shows | Wakes |
| --- | --- | --- |
| `clock` | The local time ([below](./modules.md##the-clock)) | its timer: once a minute, or once a second with seconds shown (each redraw adds the compositor's release, below) |
| `workspaces` | Each output's workspace numbers, the active one marked ([below](./modules.md##workspaces)) | on the compositor's workspace changes: one redraw per batch, however many events it held |
| `window-title` | The focused window's title on the bar's own output ([below](./modules.md##window-title)) | on the compositor's toplevel changes: focus at once, retitles at most ten times a second |
| `volume` | The default sink's level and mute ([below](./modules.md##volume)) | on the sound server's sink events, and the server's own: one redraw per batch |
| `microphone` | The default source's level and mute ([below](./modules.md##volume)) | as `volume`, for sources |
| `battery` | The batteries' charge and state ([below](./modules.md##battery)) | on the kernel's power-supply events, and once a minute while discharging |
| `network` | The shown interface's state: name, SSID, VPN or offline ([below](./modules.md##network)) | on the kernel's link, address, route and WiFi events: one redraw per batch, however many events it held; plus the WiFi signal re-read every 10 s while WiFi is shown (two wakes a tick, a redraw only when the shown level moves) |
| `brightness` | The panel backlight's level ([below](./modules.md##brightness)) | on the kernel's backlight events: one redraw per batch, however many events it held |
| `tray` | The applications' tray icons, StatusNotifierItem ([below](./modules.md##tray)) | on the session bus's traffic: item registrations, icon changes, owners vanishing |
| `media` | What the players on the session bus are playing, and their controls, over MPRIS ([below](./modules.md##media)) | on the bus's MPRIS traffic only: a player appearing or vanishing, its track or state changing |
| `bluetooth` | The adapter's power and the connected devices over BlueZ, on the system bus ([below](./modules.md##bluetooth)) | on the bus's BlueZ traffic only: BlueZ appearing or leaving, an adapter or device coming or going, power, connection, name or charge changing |
| `power` | Lock, log out, suspend, reboot and shut down from a popup menu with a confirm step ([below](./modules.md##power)) | on nothing while closed: the popup's `Can*` round trips and its own replies only |

A build can leave a module out (`cargo build --no-default-features`, then
`--features clock`); naming one that is not built is a usage error that
lists those that are. Every module takes the five
[interaction keys](./modules.md##pointer-input). Besides these two, the config can define
modules of its own by name, a [`button`, a `push` and an `exec`
module](./modules.md##button-push-and-exec-modules), each a Cargo feature (`button`,
`push`, `exec`) on by default. The `popup` feature is not a module either:
it is the [popup](./modules.md##popups) code the volume and microphone modules use, on
by default, and a build without it has none of it and does not bind
`xdg_wm_base`. The `icon-image` feature adds the PNG
decoder for [image icons](./modules.md##icons), and is off by default.



## The clock

The local time, in the format `--clock-format` gives, a small `strftime`
subset. The default, `%-I:%M %P`, is a 12-hour clock with no leading zero:
`3:07 pm`. `%H:%M` is the 24-hour one: `15:07`.

| Specifier | Shows | Specifier | Shows |
| --- | --- | --- | --- |
| `%H` | hour, `00`-`23` | `%I` | hour, `01`-`12` |
| `%k` | hour, ` 0`-`23` (space-padded) | `%l` | hour, ` 1`-`12` (space-padded) |
| `%M` | minute, `00`-`59` | `%S` | second, `00`-`59` |
| `%p` | `AM` or `PM` | `%P` | `am` or `pm` |
| `%a` | `Mon` | `%A` | `Monday` |
| `%b`, `%h` | `Sep` | `%B` | `September` |
| `%d` | day, `01`-`31` | `%e` | day, ` 1`-`31` (space-padded) |
| `%m` | month, `01`-`12` | `%j` | day of the year, `001`-`366` |
| `%y` | year, two digits | `%Y` | year |
| `%u` | weekday, `1`-`7` from Monday | `%w` | weekday, `0`-`6` from Sunday |
| `%Z` | zone abbreviation (`EDT`, `+0530`) | `%z` | offset from UTC, `+hhmm` |
| `%R` | `%H:%M` | `%T` | `%H:%M:%S` |
| `%F` | `%Y-%m-%d` | `%D` | `%m/%d/%y` |
| `%%` | a `%` | | |

- **Padding flags**, as GNU `date`: between the `%` and the letter, `-`
  drops a number's padding (`%-d` is `5`), `_` pads it with spaces and `0`
  with zeros (`%_H` is ` 7`).
- **English names, no locale lookup**: day and month names and `AM`/`PM`
  are the C locale's, whatever `LANG` says.
- **Everything else in the format is shown as it is**, any Unicode
  included (as long as the font has it).
- **Its timer fires once a minute** (on the minute), or **once a second**
  when the format shows seconds (`%S`, `%T`): no polling. Each tick that
  changes the text is one frame, and the compositor answers each frame
  with one `wl_buffer.release`, so an idle minute clock wakes the bar twice
  a minute ([What it does on the compositor](./modules.md##what-it-does-on-the-compositor)).
- **The time zone** is read as glibc reads it: `$TZ` if set (a zone name
  such as `Europe/London`, looked up under `$TZDIR` or
  `/usr/share/zoneinfo`; an absolute path, with or without a leading `:`;
  or a POSIX rule such as `EST5EDT,M3.2.0,M11.1.0`), else `/etc/localtime`.
  An empty `$TZ` is UTC. A POSIX value naming summer time but no dates
  (`TZ=EST5EDT`) takes glibc's default US rules, as `date` does. A zone
  that cannot be read is shown as UTC, with one line on stderr saying
  why; the bar still starts.
- **A new zone shows at the next tick**: `/etc/localtime` (or the file `$TZ`
  names) is checked once each wake, so `timedatectl set-timezone` takes
  effect within the minute. A zone file that disappears keeps the zone last
  read until one is back.
- **Clock steps, suspend and summer time** show at once or on the boundary:
  the timer is on the wall clock and cancelled by any step (NTP, `date
  -s`, a resume from suspend), which wakes the bar at once to redraw; a
  summer-time change shows at the first tick after it.



## Workspaces

Each output's workspace numbers, spoken over `ext-workspace-v1`, with the
active one marked by a pill in the accent color (its number drawn in the
bar's background). One number per workspace the compositor reports for the
output the bar is on, sorted by position: what scoot reports is what is
shown, no fixed slots. A workspace adopted from an unplugged monitor shows
its number (`"2 DP-1"` shows `2`). A name with no leading number (a foreign
compositor's free-form name) shows its 1-based position.

**Click a number to show that workspace**: the bar sends `activate` for it
and `commit`s the batch. Clicking the active one sends nothing. A click on
a non-focused output's bar is dropped by scoot today (it takes no output);
`output-targeted workspace switch` (in the backlog) will carry it across.
On a second monitor's bar this is the one limit of the multi-output
setup: each bar shows its own output's workspaces, but until scoot can
switch a specific output's, only the focused output's bar switches
(`focus-output` first, then click). Nothing in scootbar changes when scoot
gains it: the click is the same `activate`.

- **The pill is square and the bar's full height by default**; its shape
  is [configurable](./modules.md##the-active-workspaces-pill): rounded, a pill, or a
  circle.
- **The protocol is bound only while the module is placed.** A bar with no
  workspaces module never binds `ext_workspace_manager_v1` or `wl_seat`, so
  the compositor sends it nothing about workspaces and it wakes for none of
  it. A `scootbar msg reload` that adds the module binds both then; one
  that removes it stops the manager, destroys its handles and releases the
  seat and pointer (a seat below version 5 cannot be released and stays
  bound; the descriptor count returns to what it was). A
  compositor that starts advertising the protocol later is bound at that
  point if the module is placed and nothing was bound before. A
  compositor that finishes the manager and later re-advertises it is not
  rebound until a reload removes and re-adds the module.
- **Without `ext-workspace-v1`** the module shows nothing (one stderr note
  at start-up says the protocol is missing) and takes no space.
  **Without `wl_seat`** clicks do nothing (said too: see [Pointer input](./modules.md##pointer-input)). The bar still starts;
  the clock is unaffected.
- **The list grows and shrinks with the trailing empty workspace**,
  renumbering what follows; only the `active` state bit is shown so far
  (occupied and urgent need scoot-side work, in the module's entry).



### The active workspace's pill

`[workspaces]` shapes the pill behind the active number (all file-only,
logical pixels; the defaults are the square, full-height pill the module
always drew):

| Key | Values | Meaning |
| --- | --- | --- |
| `pill-shape` | `"rect"` (default), `"pill"`, `"circle"` | `rect`: the item's extent, corners per `pill-radius`. `pill`: the same extent with ends as round as fit (half circles). `circle`: at least as wide as tall, centered on the number. |
| `pill-radius` | 0 to 1024 | a `rect`'s corner radius, cut back to half its shorter side. Refused with `pill` or `circle`, which are already as round as they fit. |
| `pill-inset` | 0 to 1024 | the gap from the bar's top and bottom edges, so the pill is shorter than the bar. Cut back so the pill is never shorter than the text's line: the number is drawn in the bar's color over the fill, and a shorter pill would clip it away. |
| `item-gap` | 1 to 8 (default 1) | the spaces between two numbers: one space is about a third of the font size, so 1 is the old tight row and 4 a roomy one. The pill and the click target of each number follow; a click in the gap hits nothing. The module's text is cut at 256 bytes, so a bigger gap shows fewer workspaces: at 8 spaces about 26 numbered up to 99 (28 single-digit ones), the rest cut off the end. |
| `display` | `"numbers"` (default), `"dots"` | `dots`: a dot per workspace in place of the numbers — the active one filled like the pill, the rest dim (or the state colors above). Clicks land by the dots' places, exactly as by the numbers'. A `query` still reports the numbers. |
| `disc` | off by default | with a `circle` pill: grow the module's own span to the disc's diameter, so a single digit is a disc at any `padding`. Refused with any other shape, and showing dots. |
| `active-color` | a color, unset by default | the active pill's fill instead of `accent`. The pointer's `hover` still wins while it is over the module. |
| `inactive-color` | a color, unset by default | inactive numbers' ink instead of `fg` (inactive dots are `dim` unless set). |

```toml
# A rounded pill, lifted 4 off the bar's edges
[workspaces]
pill-shape = "pill"
pill-inset = 4
```

```toml
# More room between the numbers (each number's pill and click target follow)
[workspaces]
item-gap = 4
```

```toml
# A circle around a one-digit number
[bar]
padding = 10           # the circle can be as wide as the module: see below
[workspaces]
pill-shape = "circle"
pill-inset = 3
```

- **How a circle is sized**: its diameter is the pill's height (the bar's
  height less twice the inset). A **two-digit number widens the circle into
  a pill** as wide as the text needs, never a clipped disc. Growth is
  limited to the room around the number: it stops at a neighbouring
  number's ink, and cannot pass the module's own span (the numbers plus
  `padding` either side). So on a crowded bar, or one whose `padding` is
  small next to its height, a single digit gets an oval rather than a
  disc: raise `padding` or the `pill-inset` until it is round, or set
  `disc` to grow the span to the disc instead.
- **Clicks follow the pill**: a press on the drawn pill of the active
  number does nothing (it is already shown), never a neighbour's switch,
  even where a grown circle overlaps the neighbour's own click area; a press
  on the padding around any other number still activates that workspace.
- The corners are the same analytic coverage as the bar's, with no
  supersampling and no allocation, painted only when the module repaints
  (a workspace change). Measured (release build, one pill 80 device pixels
  wide on a 1600x28 bar): the default square full-height pill costs 151 ns
  (the plain fill it replaced, 153 ns); a rounded pill inset by a quarter of
  the height costs 6.3 us, and 20 us on 3200x56 (scale 2), once per
  workspace change. Dots cost one small maximally-rounded fill each, about
  6 us at a 50-pixel test em (linear in the count: 25/49/100 us for 4/8/16
  on the dev VM); at real sizes the discs cover an order of magnitude fewer
  pixels. A grown disc costs its bigger fill, about 8.5 us at the same test
  scale against 5 us for the plain pill. Neither adds a file descriptor, a
  timer or a wakeup: idle costs nothing new.
- **Not built**: urgent and occupied workspace
  colors. The protocol has an `urgent` bit (`ext-workspace-v1`), but the bar
  ignores it (`daemon/workspaces.rs` reads only `Active`) and scoot never
  sends it (nothing marks a window demanding attention yet: no
  `xdg_activation` support) — so there is nothing to drive the colors with.
  The work is read-the-bit plus send-the-bit.



## Window title

The focused window's title on the output the bar is on, spoken over
`wlr-foreign-toplevel-management-v1`. Each output shows the title of the
window activated on it; with none focused the module shows its
`placeholder` (empty by default, taking no space at all).

- **Title and app id.** The title alone, or `title - app` with
  `window-title.show-app-id`; a window with an empty title shows its app
  id, so something is shown whenever a window is focused. A static
  `window-title.icon` stands before the title whenever a window is focused
  (never for the placeholder; `window-title.show-text = false` draws only
  the icon — see [Per-state and per-level
  icons](./modules.md##per-state-and-per-level-icons)). The full title
  is the tooltip (and the `query` value's `title`), uncut by the span.
- **Width.** The span never grows past `window-title.max-width` (default
  480 logical pixels, 1 to 4096), so the title yields the bar to the other
  modules; a longer title is cut with an ellipsis by measured pixel
  width, never counted in characters.
- **Click to focus.** A left click activates the window (as
  `scootbar msg invoke window-title activate` does). A middle click closes
  it only with `window-title.allow-close = true` (off by default: closing
  a window by an accidental click loses work); naming `close` in a binding
  with closing off is refused when the file is read. The module's own
  actions are `activate` and `close`, both taking no number.
- **Cost.** Event-driven, no polling: the protocol is bound only while
  the module is placed (a bar with no window-title module is never told a
  title), and without it the module shows nothing and takes no space. A
  retitle flood draws about ten times a second — focus, output, close and
  fullscreen changes always draw at once; only title and app-id text waits
  — and only this module's span is redrawn. Titles are untrusted text:
  kept at 256 bytes, control characters stripped before they reach the
  view (and the glyph cache).
- **Only the wlr protocol.** `ext-foreign-toplevel-list-v1` has no
  `activated` state, no output events and no requests, and the two lists
  share no client-visible key, so a bar cannot correlate their handles;
  the module binds only the wlr manager.



## Volume

The default sink's level and mute, spoken over the PulseAudio native
protocol (PipeWire's `pipewire-pulse` answers it, as does PulseAudio
itself), with no libpulse and no child process: one client on the bar's
own `poll` loop. The `microphone` module is the same code for the default
source (its level and mute, the `source` key in `query`), configured under
`[microphone]`. Each is a Cargo feature (`volume`, `microphone`), both on
by default.

- **What it shows** is `49%` with the level's icon (muted, low, medium,
  high; a static `icon`, `icon-path` or `icon-image` in the table replaces
  all four — see [Icons](./modules.md##icons)), in the `muted` class while muted. The tooltip names the
  device: `Built-in Audio: 49%`, with `(muted)` after it. While no server
  answers it shows nothing and takes no space.
- **A click toggles mute, a scroll raises or lowers**, with no binding at
  all; a binding you set runs instead, as on every module. The module's
  own actions are `raise`, `lower` and `toggle-mute`, none taking a number
  (`scootbar msg invoke volume raise` raises one step), `set N`, which sets
  the level to `N` percent (held to 0 and `max-volume`, whatever number is
  given), and `popup`, which opens the [slider popup](./modules.md##popups)
  (`on-click = "popup"`). A scroll's steps
  arrive through the scroll itself, one step each. There is no default for
  a right click: point `on-right-click` at a mixer (`pavucontrol`, or
  `wpctl`).
- **`step`** (default 5, 1 to 50) is the percent points per scroll notch
  and per raise. **`max-volume`** (default 100, 100 to 150) is the cap a
  raise stops at: 100 is full scale, past it is over-amplification.
- **Cost.** One socket while the server is up, one inotify watch on its
  directory while it is not: no timer, no polling, no wakeups with no
  audio activity. A server that restarts is found when its socket comes
  back, or at once when it kept the path and only dropped the connection
  (one probe, after the watch is armed). A server that refuses the
  handshake (a cookie it does not accept) is not probed again until
  something new happens in the socket's directory, so a standing refusal
  costs no connect loop. Sets are absolute levels from what is shown
  (never accumulated steps, so a touchpad flood cannot drift), one in
  flight at most, and every answered set is re-read, so what the server
  clamped to is what is shown. The server is `$PULSE_SERVER` when that names a unix socket,
  else `$XDG_RUNTIME_DIR/pulse/native`; device names from it are untrusted
  text, kept at 128 bytes.



## Network

Link state, WiFi name and signal, spoken over rtnetlink and nl80211 with
no daemon and no child process: two netlink sockets on the bar's own
`poll` loop. A Cargo feature (`network`), on by default.

- **What it shows** is one interface's state: the default route's (the
  first usable, v4 before v6), or `network.interface` by name. Ethernet shows the name
  (`eth0`); WiFi shows the SSID (`Wimbly`); a tunnel shows `VPN` (in the normal
  class: being on a VPN is not a warning, only `offline` warns); anything
  without an address shows `offline`, in the `warn` class. A second VPN
  up beside the shown interface appends `· VPN`. The tooltip adds the
  signal (`Wimbly · −54 dBm on wlan0`). Before the first event it shows
  nothing and takes no space. An icon can stand before the text, or
   alone: `icon`, `icon-path` with `icon-viewbox`, or `icon-image` (at
   most one, as the [clock's](./modules.md##icons)) for every state, or one glyph each
   in `icon-ethernet`, `icon-wifi`, `icon-vpn` and `icon-offline` — see
   [Per-state and per-level icons](./modules.md##per-state-and-per-level-icons). The
   WiFi level is four at −55 dBm and better, one at −78 and worse, as
   `query`'s `bars` counts it.
- **At idle on WiFi the bar wakes about twice every 10 seconds**: the
  signal has no kernel event (where the radio refuses signal-threshold
  offload), so the module re-reads it on a 10 s timer, armed only while
  WiFi is shown. The redraw follows only when the shown level moves —
  an idle radio's dBm wanders constantly, and redrawing on every dBm
  would cost a frame and the compositor's release on nearly every tick.
  The tooltip's exact dBm is drawn live from the latest re-read, so it
  is fresh whenever it opens, and an open tooltip refreshes with the
  next redraw.
- **A click opens the picker**, with no binding at all: `network.menu-command`
  is spawned with the cached scan's SSIDs on stdin (one per line), a
  dmenu-style launcher fed from the scan list. Connecting is the
  command's own business, for example
  `menu-command = ["sh", "-c", "fuzzel --dmenu | xargs -d '\\n' -r -n1 nmcli device wifi connect"]`
  (no shell sees the SSID: it travels by pipe into `xargs`, which passes
  it as one argument). With `show-ssid = false` the picker refuses to
  open instead: the scan list would expose the SSIDs the bar hides. The module's own action is `menu`, taking no
  number (`scootbar msg invoke network menu`); it is refused naming why
  with no command configured or no networks seen. A second `menu` while
  one runs ends the running picker and opens a fresh one, so a picker
  left open never blocks the next; only a menu that will open ends the
  old one.
- **A native list**, opt-in as the volume popup is: `on-click = "popup"`
  opens the scan as a [popup](./modules.md##popups) list instead of the dmenu picker
  (the click binding wins over the picker's default). Each row names a
  network, starting with its strength glyph where `icon-wifi` names four
  glyphs and the scan carries a signal (a row with no signal carries
  none), and the
  associated one selected; a wheel over the list scrolls it a row a notch
  where the popup is taller than what the compositor configures for it,
  and a row too wide for it is cut with an ellipsis. Selecting a
  row closes the popup and runs `connect N` (`scootbar msg invoke network
  connect 1` connects to the list's row 1): `network.connect-command` is
  spawned with the SSID as its last argument, never through a shell
  (SSIDs are attacker-controlled radio data, so no byte in one starts a
  command; a password prompt is the command's own business, as with the
  picker). The argument is the SSID the row names (never its strength
  glyph), the same text the picker is fed: invalid UTF-8 and control characters are replaced
  with U+FFFD, so a network whose name has such bytes reaches the command
  under that shown name and the connect fails, never landing on another
  network. The list holds at most 16 rows (the popup's widget limit): a
  scan with more shows the first 16, and the picker still lists them all. `N` names what the list showed: a scan that moved underneath
  is refused rather than connected to the wrong network. `connect` is
  refused naming why with no command configured, a row past the list or
  gone from the scan. A second `connect` while one runs ends the running
  command and starts the new one, so a command that hangs (an `nmcli`
  waiting on a secret agent that never answers) never blocks the next
  connect; only a `connect` that will start ends the old one, and a
  refused one leaves the running command alone. Ending a child is
  `SIGTERM` to its own process group, then `SIGKILL` after 100 ms for
  one that ignores it; a reload or removal of the module ends both
  children the same way, so no child outlives the bar unreaped. With `show-ssid = false` the list
  stays closed, as the picker does.
- **`interface`** (1 to 15 bytes, a kernel interface name) pins what is
  shown; absent is the default route's, tracked by index so a rename
  keeps it. **`show-ssid`** (default true) hides the SSID when false —
  the bar shows `WiFi`, and `query` omits the SSID — because the bar
  is visible in screenshots and to an agent's `query`.
- **Which interface's scan is listed.** The picker and the popup list the
  shown interface's scan where it is a scanning station; where the shown
  interface is not wireless (ethernet, a tunnel, offline), they list the
  associated station's instead — or, with none associated, the first idle
  station's, so the picker still works off-network. An interface in AP mode (an
  access point, its VLAN, a P2P group owner) has no useful scan: its cache is never
  dumped and never listed, so a hotspot beside the station cannot empty
  the list. A second station's cache is never listed either: only the
  target above is dumped. The default route moving to another radio
  re-dumps the scan there; the shown radio vanishing falls back to the
  associated station that is left.
- **`query`** reports `{"state": "wifi", "ssid": "Wimbly", "signal": -54,
  "bars": 4, "interface": "wlan0", "vpn": false}`, `"ethernet"` and
  `"vpn"` with the interface, or `{"state": "disconnected"}`.
- **Cost.** Event-driven, no polling: link, address, route and (where the
  kernel lets the socket join them) `scan`/`mlme` multicast groups; a
  flapping link drains into one redraw per turn. Signal strength has no
  event on drivers without CQM thresholds, so a timer re-reads it every
  10 seconds — armed only while a WiFi network is shown, nowhere else —
  and connect, disconnect and roam stay events. Idle wakeups are only
  real network events. A missed burst, a dead socket or a resume
  re-dumps everything; `Unavailable` (and nothing shown) where the
  machine has no network interface — interfaces that appear later (a
  plugged-in dongle) are picked up by `scootbar msg reload`. Interface names and SSIDs are
  untrusted text: sanitized once, at parse time.



## Brightness

The panel backlight's level in percent, read from `/sys/class/backlight`
and woken by the kernel's uevents on a netlink socket filtered to the
`backlight` subsystem. A Cargo feature (`brightness`), on by default.

- **What it shows** is `49%` (`251` of `509` on the reference machine, an
  M2 Air), with an icon per level when configured (`brightness.icon`
  takes one glyph, or 4 for the levels; `brightness.show-text = false`
  draws only the icon — see [Per-state and per-level
  icons](./modules.md##per-state-and-per-level-icons)). The tooltip names the device: `apple-panel-bl: 49%`. Where
  there is no backlight at all (a desktop, a VM) it shows nothing and
  takes no space, owning no fd.
- **A scroll raises or lowers**, with no binding at all; a binding you set
  runs instead, as on every module. The module's own actions are `raise`
  and `lower`, neither taking a number (a scroll's steps arrive through
  the scroll itself, one step each), and `set`, taking the absolute
  percent (`scootbar msg invoke brightness set 50`). There is no default
  for a click: there is nothing to toggle.
- **`device`** names the backlight where the machine has several
  (`intel_backlight` beside `acpi_video0`); absent is the first usable one
  in sorted name order. **`step`** (default 5, 1 to 50) is the percent
  points per scroll notch and per raise.
- **Writes need permission**: the bar writes the raw value to the device's
  `brightness` file directly (no daemon, no child; logind's `SetBrightness`
  waits for the shared D-Bus client), and a udev rule for the backlight
  class or the `video` group grants it. Without it the action is refused
  naming that. Every write is an absolute level from what is shown (never
  accumulated steps, so a touchpad flood cannot drift), re-read before the
  call returns, and never below raw 1: 0 blanks the panel on the drivers
  measured, and the bar never darkens its own screen past what a scroll
  can bring back.
- **Cost.** The uevent socket, and nothing else: no timer, no polling, no
  wakeups with no backlight activity. A uevent storm drains into one
  re-read per turn; the uevent a write itself emits finds the re-read
  already done. Sysfs values are untrusted text: read once each into
  fixed buffers, digits only, a zero range or a missing file skipping its
  device.



## Battery

The batteries' charge in percent, read from `/sys/class/power_supply`,
woken by the kernel's uevents on a netlink socket (group 1). The
subsystem filter is in userspace: the bar wakes on every kernel uevent
and drops those that are not `power_supply`. A Cargo feature (`battery`), on by default.

- **What it shows** is `72%`, in the `warn` class at or below
  `warn-below` (default 20) and `urgent` at or below `urgent-below`
  (default 10, 0 to 100 each), by level alone; `warn-below` must be at
  least `urgent-below`, else warn is unreachable and the file is refused. The tooltip names the
  state: `Discharging 72%` (`Charging`, `Full`, `Not charging` or
  `Unknown` for a status string no kernel documents, which never refuses
  the battery). An icon stands before the percent when configured
  (`battery.icon` takes one glyph, or 5 for the charge levels, with
  `battery.icon-charging` and `battery.icon-full` for those states;
  `battery.show-text = false` draws only the icon — see [Per-state and
  per-level icons](./modules.md##per-state-and-per-level-icons)). Where there is no battery at all (a desktop, a VM) it
  shows nothing and takes no space, owning no fd.
- **Several batteries** are combined by default (`batteries =
  "combine"`: the mean capacity, discharging winning the state), or the
  first in sorted name order with `batteries = "first"`. A battery
  removed at runtime hides the module until one is back; the uevent socket
  stays as the appearance watch, so a reinsert shows again with no
  polling. The percentage always comes from `capacity`, never from
  `charge_now`/`charge_full` (on the reference machine the two disagree
  by four points at full). No time-remaining is shown: it needs rate
  smoothing to be honest, and a wrong estimate is worse than none.
- **`on-low = { exec = [...] }`** runs once per downward crossing of
  `urgent-below` (and re-arms when the level rises back above, so the next
  crossing fires again; starting below it is not a crossing). It runs
  through the bar's own spawner, like a click binding: bounded and
  reaped, never through a shell. The module defines no actions of its
  own, so its five interaction keys take commands only.
- **Cost.** The uevent socket always (one wake per kernel event, however
  many datagrams arrive, whatever their subsystem: a storm is drained and
  re-read once per turn), and a timerfd re-reading once a minute only
  while discharging. Measured on the Asahi M2: plug and unplug each emit a
  burst of `power_supply` uevents, and capacity steps while discharging
  emit none (five steps over 62 minutes, zero uevents), so the discharge
  timer is what sees them. Other drivers may differ. Charging, full and
  absent batteries own no timer. Sysfs files are read once each into fixed buffers; a
  capacity past 100 is clamped, an unparsable one skips its battery,
  and a removed battery is one line on stderr, not one per wake.



## Tray

The applications' tray icons: the StatusNotifierItem watcher and host over
the session bus, one icon per item, drawn at the output's real device
pixels. A Cargo feature (`tray`), on by default; the smallest build
(`--no-default-features`) has none of it. It speaks D-Bus through the
bar's own client (`src/dbus`: no `zbus`, no libdbus, no thread; its
socket is one more source in the `poll` loop), which is the
[spike's](https://github.com/scoot-sh/scoot/tree/main/docs/scootbar/spikes/dbus-client.md) hand-rolled choice.

- **The bar is the watcher.** It owns `org.kde.StatusNotifierWatcher` (and
  the `org.freedesktop` twin) when they are free, answers apps'
  registrations itself, and re-takes the name if its owner leaves. Where
  another process already owns it (a desktop's own tray), the bar hosts
  against that watcher instead: its items still appear, a click still
  works, and the bar takes over when that owner goes. Items that
  registered before the bar started are found by listing the bus's names
  at connect (an item behind a plain unique name registers explicitly, and
  is only seen if it registers after the bar is up: the KDE watcher's own
  limit). With no session bus the module shows nothing and waits for it
  (one inotify watch on the bus socket's directory for a `unix:path=`
  address, no polling; one retry timer polling the name every 30 seconds
  for a `unix:abstract=` address, which has no directory to watch), and a
  bus that
  dies mid-run drops every icon at once and dials once more. A bus that
  takes the bar in and drops it within five seconds, three times running,
  is not dialled again at once (a refusing bus must not spin the bar): it
  is tried once more after 30 seconds, and again each time that try dies
  the same way, or as soon as its socket is made anew (for an abstract
  address, at the next 30-second poll). Only the bus's own
  `NameOwnerChanged` is believed (a peer's, addressed to the bar, removes
  nothing), and when hosting only the watcher hosted against speaks for it.
  The address is `DBUS_SESSION_BUS_ADDRESS`'s `unix:path=` or
  `unix:abstract=` (`%xx` escapes decoded; a path wins when the address
  names both), else `$XDG_RUNTIME_DIR/bus` when it is not set; an address
  that names neither (`tcp:`, `autolaunch:`) is refused with a
  line on stderr and no tray, not replaced by another bus that happens to
  be at the default place. A bus that refuses the bar the watcher name (a
  policy that denies `own`) is said once on stderr, and the bar hosts.
- **What is drawn.** An item's `IconPixmap` (raw `ARGB32` over the bus),
  picked at the output's device size from the entries sent and scaled
  only when none matches, from the shared icon cache: a steady bar
  re-reads nothing. An item that sends only an icon *name* is resolved
  through the `hicolor` icon theme instead (`$XDG_DATA_DIRS`, the item's
  `IconThemePath` first): PNG files, decoded once per icon version
  (downscaled to at most 64 a side, the pixmap bound) through the same
  cache, at the output's device pixels like a pixmap. The size dir
  closest to the drawn size wins (looked up for 24 device pixels where
  the bus turn knows no output size; the cache scales smoothly to the
  real size, including HiDPI); a trailing lowercase `.png` on the name is
  accepted (`.PNG` stays hidden), `.svg`/`.xpm` stay hidden.
  While `NeedsAttention` the attention name is drawn instead of the
  main one, for items without a pixmap (an item that sent a pixmap
  keeps it while alarmed). An item whose `Status` is `Passive` (the spec's "hide
  me"), or whose name resolves to nothing (missing theme, SVG-only,
  hostile name), is tracked and reachable by index but takes no room.
  Overlay icons are read for shape and not drawn (compositing a badge
  is a second scaled draw a frame for something real items rarely
  send), and tooltip icons are not drawn. A theme installed or changed
  is picked up on the item's next update, or on restart: nothing polls
  the theme directories. The tooltip over the module lists the shown
  items' titles.
- **Clicks, with no binding at all**: a left click is `activate`, a middle
  click `secondary` (the spec's `SecondaryActivate`), a scroll `wheel-up`
  or `wheel-down`, each on the item under the pointer. A right click opens
  the item's menu (as `menu N` does), and a click on an item that is its
  own menu (`ItemIsMenu`, whose whole point is its menu, which an
  `Activate` may ignore) does the same instead of activating. The
  module's actions are `activate`, `secondary`, `wheel-up`, `wheel-down`
  (each taking the item's index in the order `query` lists them:
  `scootbar msg invoke tray activate 0`), `menu` (taking the index too),
  and the popup rows' own `menu-select`, `menu-drill` and `menu-back`
  (a row's dbusmenu id, or nothing for `menu-back`).
  (The wheel actions are not called `scroll-up` and `scroll-down`: those
  are the names of the interaction keys, and `msg invoke` reads them as
  those.) `Activate` and `SecondaryActivate` are sent with position `(0,
  0)` (a bar has no screen coordinates to give), and a wheel action sends
  `Scroll(n, "vertical")` with `n` the notch count clamped to 64, negative
  for `wheel-up` and positive for `wheel-down`. The sign is GTK's,
  measured 2026-10-07 against a real item: pasystray 0.8.2
  (libayatana-appindicator maps a positive vertical delta to scroll-down,
  so wheel-up must send negative; verified on the wire as `Scroll(-1)`),
  and matching Waybar (which sends −1 for up, from its source). The SNI
  spec is silent on the sign. Plasma sends Qt's convention instead (its
  tray forwards `+angleDelta`: up positive at 120 a notch) and kmix turns
  the volume up on positive deltas — so KDE volume items need Qt-scale
  deltas this bar does not send; with one unit a notch no sign satisfies
  them, and the GTK side (the items that observably respond) decides it.
  Bound to a key (`on-scroll-up = "wheel-up 0"`)
  the notch count of the scroll itself is what is sent.
- **Menus.** An item's menu (`ContextMenu`, the DBusMenu protocol) opens
  in a [popup](./modules.md##popups): the bar reads it with its DBusMenu client
  (`GetLayout`, `Event`, `AboutToShow`, the `LayoutUpdated` and
  `ItemsPropertiesUpdated` signals, bounded like the rest of the client)
  and draws one row per item. A row click sends `Event(id, "clicked",
  ...)` to the item and closes the menu; a submenu row drills a level
  deeper in the same popup (a `< Back` row on top walks back out: levels
  nest in place rather than flatten, so every level of a deep tree stays
  addressable); a layout update while open re-fills it, and the item
  vanishing closes it. A level longer than the popup holds (16 widgets,
  less the back row) is cut, the extras dropped silently like any
  module's; a scrolled popup pans within its rows, and a row wider than
  the popup is cut with an ellipsis.
  An item with no menu to read is asked with `ContextMenu(0, 0)` instead
  (a bar has no screen coordinates to give); an item without that method
  ignores the call. Labels keep their accelerators stripped (a lone `_`
  marks the shortcut and is not drawn, `__` is a literal underscore).
  Toggles show their state as text (`[x] `/`[ ] ` for a checkmark, `(o) `/
  `( ) ` for a radio: the popup has no checkmark widget, and text is
  always in the font). Separators are blank rows between groups, disabled
  rows plain text (neither is interactive). Icons in menu items are not
  drawn in this version. Without the `popup` feature the DBusMenu half
  is refused saying so (`ContextMenu` still goes out). Real items read
  menus the same way, checked live 2026-10-07 against CopyQ 16.0.0,
  KeePassXC 2.7.12 and Joplin 3.6.16 (Electron): a Qt reply carries the
  out-args bare (`u(ia{sv}av)`, not wrapped in a struct — the bar once
  demanded the wrapped form and every real menu stayed shut); rows that
  omit `enabled`/`visible` default to shown. A row click sends
  `Event(id, "clicked", variant int32 0, timestamp 0)` and closes the
  menu; the zeros drive real actions (a CopyQ row set its clipboard,
  Joplin's Quit quit) — no item read the data or the timestamp.
  Ayatana items (pasystray 0.8.2) expose neither `Activate` nor
  `ContextMenu` and send no pixmap, so a left click does nothing and,
  until themed icons land, their menus cannot open (no span to open
  from). Electron serves its menu from a second connection of the same
  process (replies arrive from another unique name) and answers no
  introspection: the bar never introspects, and calls addressed to a
  well-known name accept any sender, so both work.
- **`query`** reports `{ "watcher": "owner" | "host", "items": [{ "id",
  "title", "status", "shown" }] }` while any item is tracked, and nothing
  while none is.
- **Margin and keys.** `[tray]` takes `margin` and the five
  [interaction keys](./modules.md##pointer-input) (`on-click`, `on-right-click`,
  `on-middle-click`, `on-scroll-up`, `on-scroll-down`); the module has no
  options of its own. A binding replaces the default for its trigger, as
  on every module.
- **Bounds, for a hostile or broken item.** Everything an item says is
  untrusted bytes from a same-user peer: a message past 1 MiB (the spec
  allows 128 MiB, and SNI cannot ask an item for a size, so a 512 by 512
  pixmap is just over) is skipped whole as it arrives: the answer is
  dropped, the item keeps its last state and is read again at its next
  signal, and the connection lives; a header that is no message, or past
  128 MiB, ends the connection. Nesting past 32 and pixmaps past 256
  pixels a side are refused the same way (the entry, or the answer, is
  dropped). A flood of signals (the match rule has no sender, so any peer
   may send what the bar listens for) is read up to 1 MiB + 64 KiB staged
  (the read watermark: one capped message and a read's worth) and the
  rest left in the socket, 256 events a wake with the bar's other sources
  between, and costs the connection nothing; while an over-cap message
  is discarded, one turn reads at most 256 KiB more (the poll is woken
  for the rest), so a sender that outruns the reader holds one turn,
  not the bar. Titles are cut to 128 bytes
  with controls stripped; themed names past 128 bytes (after one
  accepted trailing `.png` is stripped), with a `/` or
  starting with a dot, are refused without touching the disk, and an
  `IconThemePath` that is not absolute or holds `..` is ignored; a
  theme file is read only when it resolves inside its base
  (symlink escapes refused), opened `O_NONBLOCK`/`O_NOFOLLOW` and taken
  only when the opened fd is a regular file under the cap (a path
  swapped to a FIFO, device or symlink mid-lookup can never block the
  bus turn), at most 8 MiB, decoded under a 16 MiB
  budget and a 512-pixel, 1 M-pixel header check, then stored at most
  64 a side (larger decodes are downscaled once: 512 KiB over the whole
  tray at most); at most 32 items, 8 from one service or one registrant, 8 pixmap
  entries each; one `GetAll` in flight per item however many signals it
  sends; a menu's layout is read bounded the same way (at most 8 levels,
  64 nodes including the root — 63 drawable rows — one `GetLayout` in flight, re-read no oftener than every
  50 ms however it floods `LayoutUpdated`), and update signals are
  accepted only from the item that owns the open menu; a call nobody answers is forgotten after 30 seconds when its slot
  is wanted (no bus times a call out by default, measured on a stock
  `dbus-daemon` and on `dbus-broker`; only a client library does). A bus that stops reading drops the connection, and
  not the bar. The bus
  set-up (auth and `Hello`) is blocking, bounded to 2 seconds in total.
  The parser is fuzzed (`crates/scootbar/fuzz`, target `dbus`).
- **Cost.** One fd (the bus socket, with `OUT` only while a write or a
  staged message waits), or one inotify fd while there is no bus at a
  `unix:path=` address (one retry timer polling every 30 seconds at a
  `unix:abstract=` address, which has no directory to watch), and a
  one-shot timer only while an item waits out its 50 ms floor between
  reads or after the bus kept dropping the bar. Measured (dev VM, one
  60 s idle window per row, `unix:path=` addresses): **zero wakeups** with
  no bus, with a bus and
  no items, and with one and with eight items on it; RSS 4156 kB with the
   tray alone and no bus or no items, 4352 kB with one item, 4388 kB with
   eight (differences under about 130 kB are within one run's resolution;
   level with the same tree's `main` in every row). The binary: **no byte
   on disk and +2,496 B of `.text` (+0.2%)** against `main` (2,036,448 B
   on disk both sides). An item that
  re-announces its icon continuously is read at most every 50 ms (0.7% of
  a core measured, against 9.7% with no floor). The table and its method
  are in the
  [resource ratchet](https://github.com/scoot-sh/scoot/tree/main/docs/scootbar/backlog/lightest.md#m6-tray-and-the-d-bus-client-module-level-cost-measured-2026-10-02);
  the hardening and link rows are in
  [the same file](https://github.com/scoot-sh/scoot/tree/main/docs/scootbar/backlog/lightest.md#m6-tray-hardening-and-one-bus-lifecycle-measured-2026-10-03).



## Media

What the players on the session bus are playing, and play/pause, next and
previous, spoken over MPRIS (`org.mpris.MediaPlayer2.*`: mpv, VLC,
Spotify, Firefox, Chromium and most others) through the bar's own D-Bus
client (see [Tray](./modules.md##tray)): no `zbus`, no libdbus, no thread, no polling. A
Cargo feature (`media`), on by default; the smallest build
(`--no-default-features`) has none of it.

- **What it shows** is `artist - title` (or whichever of the two the player
  sent, or the player's name when it sent neither) with a play or pause
  icon for the state — the built-ins, or `media.icon-playing` and
  `media.icon-paused` when configured (falling back to `media.icon`;
  `media.show-text = false` draws only the icon: a stopped player shows
  nothing, so there is no third key — see [Per-state and per-level
  icons](./modules.md##per-state-and-per-level-icons)) — cut to `max-width` with an ellipsis measured in
  pixels, in the `muted` class while paused. The tooltip over the module (after the bar's
  `tooltip-delay`, as every module's; see [Tooltips](./modules.md##tooltips)) is the uncut
  line with the player's name: `mpv (playing): Ada - Song`. Several artists are joined with a
  comma. **A stopped player shows nothing**, and with no player the module
  takes no space; start playback from the player.
- **Which player.** Several may run. Of the ones that are playing or
  paused, `player` (a short name: `spotify` for
  `org.mpris.MediaPlayer2.spotify`, and for its second copies, which a
  player names `.instance` and a suffix: `.instance1234`, `.instance-abc`) is
  shown if it is among them; else the one that most recently started
  playing; else, when none plays, the one that played last; a tie (players
  that never played) is broken by name, so the choice is the same every
  run. The controls go to the player shown. A connection that owns two MPRIS
  names is one player (the first name seen; if it releases that one, the
  other takes over); at most 8 are held (see Bounds for what a ninth does).
- **Controls, with no binding at all**: a click is `play-pause`, a right
  click or a scroll down `next`, a middle click or a scroll up `previous`;
  nothing happens when no player is shown. The module's actions are
  `play-pause`, `next` and `previous`, none taking a number
  (`scootbar msg invoke media next`). Each is one call to the player's
  connection that wants no reply: the track changes on the bar when the
  player says so, as a signal. An action is refused saying why when there
  is no bus, no player playing or paused, or the player says it cannot
  (`CanControl`, `CanGoNext`, `CanGoPrevious` false). **A skip within 250 ms
  of the last is refused** (a scroll arrives at up to sixty actions a
  second, and a scroll of thirty steps is one skip, not thirty; a scroll
  that hits the limit is one line on stderr a second at most, as any failing
  action): an agent that wants two skips waits for the first to show in
  `query`.
- **`query`** reports `{ "player", "bus_name", "status", "title", "artist",
  "players": [{ "bus_name", "status" }] }` for the player shown (`status` is
  `playing` or `paused`; `players` lists every one held, stopped included),
  and nothing while none is.
- **Options.** `[media]` takes `player` (a name as it is on the bus:
  letters, digits, `_`, `-` and `.`), `max-width` (default 320 logical
  pixels, 1 to 4096), `margin` and the five [interaction keys](./modules.md##pointer-input).
  The playback position and the volume are never shown (the position has no
  change signal, so showing it would need a timer), and the art URL a player
  names is never fetched.
- **A run of changes is drawn ten times a second at most.** The first
  change after a quiet spell (a new track) is drawn in the turn it arrives;
  changes that follow within 100 ms (a title, a state flipping between
  playing and paused, another player becoming the one shown) wait for one
  timer, and the bar then shows the latest, however many came; the last
  change is never lost. The module appearing (a first player) or emptying
  (the last one gone or stopped, the bus lost) is never held. (The window
  title's rule, for the same reason: a player that rewrites its title or
  flaps its state hundreds of times a second is a bug or a title that
  carries progress.)
- **Idle cost: nothing.** The bus does the filtering: the bar asks for
  `NameOwnerChanged` of the `org.mpris.MediaPlayer2` namespace and for
  `PropertiesChanged` of the Player interface on the one MPRIS object, so an
  app unrelated to media coming or going, or a player's `Seeked` or
  position, never wakes the bar (tested against a real `dbus-daemon`). A
  track change is one signal carrying the value (no round trip); a player
  that only invalidates a property is read once, no oftener than every
  50 ms. With no bus the module waits for it (one inotify watch on the
  bus socket's directory for a `unix:path=` address; one retry timer
  polling the name every 30 seconds for a `unix:abstract=` address, which
  has no directory to watch); a bus that goes away drops every player at
  once and dials
  once more, and one that keeps dropping the bar is left alone for 30
  seconds, said once on stderr and again at each 30 s retry that dies the
  same way (the [tray's](./modules.md##tray) rules, in `src/dbus/link.rs`).
- **Bounds, for a hostile or broken player.** Anything on the session bus
  can claim a player name and say anything. What the code guarantees: it
  cannot crash or hang the bar or another player, or grow the bar without
   bound; only the bus's own `NameOwnerChanged` is believed, and a
   `PropertiesChanged` only from the connection that owns a held name. (Method
   replies are matched by serial *and* sender: measured 2026-10-03,
   `dbus-daemon` 1.16.2 delivers an unsolicited reply from a peer that was
   not the callee, while `dbus-broker` 37 does not, so a reply whose sender
   is not the callee is refused whenever the callee is known — the bus's
   own calls, or a unique name's. What remains is a call made of a
   well-known name, whose holder the client cannot know: a forged answer
   to one of those is accepted, like any peer's own claim to the name.) A hostile peer costs the bar the work of its own
  signals and no more (a flood of positions about 0.6% of a core at 500 a
  second, measured; title and state changes are drawn ten times a second at
  most); it holds at most one of 8 slots, one a connection. What it *can*
  still do is cost visibility, boundedly: connections that keep 8 live
  players (playing or paused) fill the table, and 16 more names announcing
  after them fill the waiting list, so a player that arrives behind all of
  those is not shown until a slot frees, and the oldest waiting name is
  forgotten past 16; that is a loss of what the module shows, never of the
  bar. The rule: at most 8 players are held, one a connection. When all 8
  are held a newcomer takes the place of the oldest *stopped* player whose
  read has been answered or has errored (it shows nothing; it goes to the
  waiting list flagged as evicted), else, with no such player, it waits in
  the list of 16 names (a 17th forgets the oldest; a name is dropped from
  it when it loses its owner and re-keyed when it changes hands). A freed
  slot gives each waiting name one attempt to be held (bounded: nothing
  re-lists the bus), and a held player that stops swaps in one waiting name
  that was not itself evicted; an evicted name comes back only through a
  freed slot, so an evicted player that starts playing is not seen until
  some held player is removed (this matters at 9 or more MPRIS names). The second
  name of a connection waits in the same list, so a connection that releases
  one keeps its player under the other. A connection owning hundreds of
  names is asked about in a window of 8 at a time, and each is held or waits
  by the same rule. Titles and artists are cleaned (controls stripped) and
  cut to 120 bytes where they are stored. A message past 1 MiB is skipped
  whole and the player keeps its last state; an answer that does not parse
  is dropped whole; a read that errors (a timeout, or `UnknownObject` from a
  player that has the name before it exports the object) leaves the player
  held, shown as nothing, and read again at its next signal; one that never
  answers is forgotten after 30 seconds when its slot is wanted; a property
  of the wrong type is skipped alone; dictionaries past 128 entries are
  refused. (`ListNames` is read to 4096 names before the MPRIS filter: a bus
  holding more can hide a player from the start-up listing, though not from
  its `NameOwnerChanged`.) The parser (`src/dbus/mpris.rs`, `std` only) is
  fuzzed with the D-Bus client's (`crates/scootbar/fuzz`, target `dbus`), and
  checked against what sd-bus marshals.
- **Cost.** One fd (the bus socket, with `OUT` only while a write or a
  staged message waits), or one inotify fd while there is no bus at a
  `unix:path=` address (one retry timer polling every 30 seconds at a
  `unix:abstract=` address, which has no directory to watch), and a
  one-shot timer only while a player waits out its 50 ms floor between reads,
  while a change of title waits out its 100 ms between draws, or (30 s) after
  the bus kept dropping the bar.
  Measured (dev VM, one 60 s idle window per row, `unix:path=` addresses):
  **zero wakeups** with no
  bus, with a bus and no player, with one player paused, with one playing,
  with eight playing, and with a real mpv (its MPRIS script) playing a file;
  one thread throughout; RSS 4156 kB with a bus and no player, 4388 kB with
  one playing player, 4392 kB with eight (differences under about 130 kB
  are within one run's resolution; the clock alone is 4236 kB). A player
  that signals constantly pays for itself and no more: a stub signalling its
  position 500 times a second for 20 s (10,143 signals) cost the bar 0.6% of
  a core, one sending a new title 480 times a second 0.85%, and one flipping
  between playing and paused 500 times a second 0.75%, RSS flat in all three
  (the tests pin that a position draws and reads nothing, and that title and
  state changes are drawn ten times a second at most). A player that
  re-sends unchanged metadata (mpv playing its synthetic `lavfi` source
  does, once a second) wakes the bar once a second and draws nothing. The
  binary: **+65,536 B on disk (1,905,376 to 1,970,912, +3.4%) and +52,912 B
  of loaded sections (+2.9%, of which `.text` +45,376 B)** against `main`;
  the feature built but off is +3,096 B loaded and no more on disk. The
  maintainer waived this row on 2026-10-03 (this row only) and `media`
  stays in `default`. The table and its method are in the
  [resource ratchet](https://github.com/scoot-sh/scoot/tree/main/docs/scootbar/backlog/lightest.md#m6-media-module-level-cost-measured-2026-10-03).



## Bluetooth

The adapter's power and the connected devices, spoken over BlueZ
(`org.bluez`) on the **system bus** through the bar's own D-Bus client
(see [Tray](./modules.md##tray)): no `zbus`, no libdbus, no thread, no polling. The
system bus is `DBUS_SYSTEM_BUS_ADDRESS` when it names a filesystem path,
else `/run/dbus/system_bus_socket` (`src/dbus/conn.rs`); the client
authenticates the same `EXTERNAL` way on both. A Cargo feature
(`bluetooth`), on by default; the smallest build (`--no-default-features`)
has none of it.

- **What it shows** is the first connected device's name in path order
  (with its charge when BlueZ reports one: `Headset 72%`), `on` while an
  adapter is powered with nothing connected, and `off`, in the `muted`
  class, while every adapter is off, with an icon per state when
  configured (`bluetooth.icon-off`, `bluetooth.icon-on` and
  `bluetooth.icon-connected`, falling back to `bluetooth.icon`;
  `bluetooth.show-text = false` draws only the icon — see [Per-state and
  per-level icons](./modules.md##per-state-and-per-level-icons)). Adapter power dominates a device that
  still claims to be connected (its disconnect is on its way). With no
  adapter, or no BlueZ at all, the module shows nothing and takes no
  space. The tooltip lists every connected device with its charge.
- **Which device.** Several may be connected; the one shown is the first
  in path order, so the choice is the same every run. The toggle goes to
  the first adapter in path order.
- **A click toggles the first adapter's power, with no binding at all**:
  one `Set` of `Adapter1.Powered` that wants no reply (the state changes
  on the bar when BlueZ says so, as a signal); nothing happens with no
  adapter. The module's actions are `toggle` and `menu`, neither taking a
  number (`scootbar msg invoke bluetooth toggle`).   `menu` opens the
  picker, a dmenu-style command fed the device list (see below); it is
  refused naming why with no command configured, no system bus, no
  adapter, or no devices seen yet.
- **Picking a device** is `bluetooth.menu-command`, spawned with the
  device list on stdin (one per line, a connected device marked
  `(connected)`), a dmenu-style launcher fed from the held set, e.g.
  `menu-command = ["sh", "-c", "fuzzel --dmenu | ..."]`. Connecting is the
  command's own business; the bar never reads the choice back. The picker
  is the interim path, as the network module's is: a native list waits for
  the popups to grow one (see the network module's entry).
- **`query`** reports `{"state": "connected", "adapters": 1, "powered":
  true, "connected": 1, "device": "Headset", "battery": 72}` (`state` is
  `off`, `on` or `connected`; `device` is the device shown; `battery`
  only when BlueZ reports one), and nothing while there is no adapter.
- **Options.** `[bluetooth]` takes `menu-command` (no empty argument),
  `margin` and the five [interaction keys](./modules.md##pointer-input). The charge is
  shown only when BlueZ reports it (`Battery1`); it is never polled.
- **A run of changes is drawn ten times a second at most**, as the media
  module's: the first change after a quiet spell is drawn in the turn it
  arrives, the rest wait for one 100 ms timer; the module appearing (a
  first adapter) or emptying (the last one gone, BlueZ leaving, the bus
  lost) is never held.
- **Idle cost: nothing.** The bus does the filtering: the bar asks for
  `NameOwnerChanged` of exactly `org.bluez` and for the object-manager
  and `PropertiesChanged` signals under `/org/bluez`, so anything else on
  the system bus never wakes the bar (tested against a real
  `dbus-daemon`). A power or connection change is one signal carrying the
  value (no round trip); an object whose signal only invalidates a shown
  property is read once (`GetAll`), no oftener than every 50 ms. With no
  system bus the module waits on the socket's directory (one inotify
  watch); a bus that goes away drops everything at once and dials once
  more, and one that keeps dropping the bar is left alone for 30 seconds
  (the [tray's](./modules.md##tray) rules, in `src/dbus/link.rs`).
- **Bounds, for a hostile or missing BlueZ.** Anything on the system bus
  can own `org.bluez` when BlueZ itself is absent and say anything. What
  the code guarantees: it cannot crash or hang the bar or grow it without
  bound; only the bus's own `NameOwnerChanged` is believed, every other
  signal only from the tracked owner of `org.bluez` (checked per signal),
  and only for a held path (a signal for an unknown path re-reads the set
  once on a 50 ms timer instead of trusting it). (Method replies are
  matched by serial *and* sender: measured 2026-10-03, `dbus-daemon`
  1.16.2 delivers an unsolicited reply from a peer that was not the
  callee, while `dbus-broker` 37 does not, so a reply whose sender is not
  the callee is refused whenever the callee is known — the bus's own
  calls, or a unique name's. What remains is a call made of the
  well-known `org.bluez`, whose holder the client cannot know: a forged
  answer to one of those is accepted, like any peer's own claim to the
  name.) A hostile peer costs the bar the
  work of its own signals and no more (connect/disconnect storms are drawn
  ten times a second at most); it holds at most one of 8 adapter slots or
  64 device slots, and a newcomer to a full room is ignored, said once.
  What it *can* still do is cost visibility, boundedly: full tables hide a
  later object until a slot frees. Names (`Name`, else `Alias`, else the
  path's last element) are cleaned (controls stripped) and cut to 120 bytes
  where they are stored. A `GetManagedObjects` answer past 1 MiB is skipped
  whole and the module keeps showing its last state: one retry on the
  coalesce timer, then it reads again at the next signal (never asking at
  once for the same oversized answer in a loop); an answer that does not
  parse is dropped whole; a read that
  errors leaves the last state and is read again at the next signal; one
  that never answers is forgotten after 30 seconds when its slot is
  wanted; a property of the wrong type is skipped alone; dictionaries past
  128 entries, interfaces past 32 of one object, and overlong name lists
  are refused. The parser (`src/dbus/bluez.rs`, `std` only) is fuzzed with
  the D-Bus client's (`crates/scootbar/fuzz`, target `dbus`), and the
  module is exercised against a scripted bus and a fake BlueZ on a real
  `dbus-daemon`.
- **Cost.** One fd (the bus socket, with `OUT` only while a write or a
  staged message waits), or one inotify fd while there is no bus, and a
  one-shot timer only while an object waits out its 50 ms floor between
  reads, while a set re-read waits out its own, or while a change of text
  waits out its 100 ms between draws, or (30 s) after the bus kept
  dropping the bar; a pidfd only while the picker runs.
  Measured (dev VM, one 60 s idle window per row): **zero wakeups** with no
  bus, with a bus and no BlueZ, and with an idle BlueZ (adapter on, one
  device connected, no traffic: the bar shows `Headset 72%` throughout);
  one thread throughout; RSS 4168 kB with a bus and no BlueZ, 4388 kB with
  the idle BlueZ (placing the module costs the font every module needs:
  3880 kB with no module placed, 4312 kB with the clock). A burst of
  10,000 connection signals in under a second (an independent raw-socket
  peer, `bench/m6-bluetooth-vm/scripts/bluez.pl`) costs about 2,000
  wakeups and 0.03 CPU-seconds, RSS flat, and then silence: the draws are
  held at ten a second and identical signals draw nothing (the unit test
  pins at most two draws for 400 alternating flips). The binary:
  **+65,536 B on disk (1,970,912 to 2,036,448, +3.3%) and +66,224 B of
  loaded sections (+3.6%, of which `.text` +58,400 B)** against `main`;
  the feature built but off is +3,032 B loaded and no more on disk. The
  size row is the same shape as every module before it, and the rule's own
   exception covers only a row the module adds, so it is a regression for
   the maintainer to waive or not. The table and its method are in the
   [resource ratchet](https://github.com/scoot-sh/scoot/tree/main/docs/scootbar/backlog/lightest.md#m6-bluetooth-module-level-cost-measured-2026-10-03).



## Power

Lock, log out, suspend, reboot and shut down from a popup menu with a
confirm step, so a stray click never ends the session (a bare
`[button.power]` with `on-click = { scoot = "quit" }` does that today, on
one click). A Cargo feature (`power`), on by default.

- **What it shows** is one icon and no text. Without an icon the module
  shows nothing and takes no space: set one (the example is MDI power,
  U+F0425, in a Nerd Font):

```toml
[power]
icon = "\U000F0425"
on-click = "popup"
```

  A click bound to `popup` opens the menu (as the volume slider's: with no
  binding a click does nothing). The tooltip names the rows shown, so a
  hover says what a click offers.

- **The menu** is one row per action — Lock, Log out, Suspend, Reboot,
  Shut down — each with an optional glyph before its label (`icon-lock`,
  `icon-logout`, `icon-suspend`, `icon-reboot`, `icon-poweroff`, each one
  glyph like `icon` itself; a row with none shows its label alone).
- **Confirm.** Lock runs at once; every other row arms on its first click
  (only that row: its label becomes "…? Click again") and performs on a
  second click on the same row within 5 seconds, closing the popup.
  Clicking another row re-arms to it, and waiting disarms. The arm
  survives refills and reopens (arming lengthens the row, which resizes
  the popup, which reopens it — disarming on a fill would make the arm
  invisible): reopening within the window shows the armed row with its
  explicit confirm label, never a hidden trap, and the window bounds any
  staleness.
- **What each row does.** Every row is overridable with a `*-command`
  argv list, run directly and never through a shell (`lock-command`,
  `logout-command`, `suspend-command`, `reboot-command`,
  `poweroff-command`, each at most 32 arguments of 4096 bytes, no NUL),
  and hideable with `rows` (a subset of `lock`, `logout`, `suspend`,
  `reboot`, `poweroff`; all of them when absent):
  - Lock runs `lock-command`. There is no default and the row is hidden
    without one: no locker fits every session, and guessing would fail
    or run the wrong one.
  - Log out runs `logout-command` when set, else quits scoot over its
    control socket (the button modules' path). Without a command and
    with scoot unreachable the row is absent from the menu, and an
    invoke of it is refused aloud, rather than flapping with the socket.
  - Suspend, reboot and shut down run their command when set, else call
    logind over the system bus (`Suspend`/`Reboot`/`PowerOff` with
    `interactive: true`, so polkit decides). A row whose `CanSuspend`,
    `CanReboot` or `CanPowerOff` answers `no` or `na` is hidden (asked
    when the popup opens — answers older than a minute are re-asked —
    not per frame or per refill); `yes`, `challenge` (the action call
    drives authentication), an unknown answer, or no bus yet shows it. A refused call is kept as the module's last error — said on
    stderr when it arrives, shown as the popup's first line, in the
    tooltip and in `query` — never silently.
- **An agent's `invoke` follows the same two steps** (`scootbar msg
  invoke power logout` arms; a second within 5 seconds performs). A
  single invoke never ends the session: the confirm guards the
  pointer's stray click, and the programmatic caller is already explicit
  — but a buggy agent's stray single call is the same lost work, so the
  menu does not trust it either. The direct path for an agent that means
  it stays `scoot msg action quit`, one explicit call with no confirm.
- **`query`** reports the rows shown, the armed one if any, and the last
  failure: `{"rows": ["lock", "logout", "reboot"], "armed": "reboot",
  "error": "logind refused reboot: ..."}`.
- **Options.** `[power]` takes `icon` (plus `icon-path`, `icon-viewbox`
  and `icon-image`, at most one, as the clock's), the five per-row
  glyphs, `rows`, the five `*-command` lists, `margin` and the five
  [interaction keys](./modules.md##pointer-input).
- **Idle cost: nothing while closed.** The system-bus connection is made
  when the popup first opens (or an invoke first needs logind), never at
  start: a bar whose menu is never opened holds no bus fd and makes no
  round trips (measured: zero sources before first use). Afterwards one
  connection stays, woken only by its own replies; no match rules are
  installed, so nothing else on the system bus wakes the bar.
- **Bounds, for a hostile logind.** On a bus without logind anything can
  own `org.freedesktop.login1` and say anything. What the code
  guarantees: it cannot crash or hang the bar (a reply that does not
  parse keeps the last state, said once per connection; one that errors
  or never comes keeps what was shown, or records the action's error);
  an answer other than `yes`/`challenge`/`no`/`na` never hides a row;
  the error text kept is cut to 256 bytes. Replies are matched by serial
  and sender like every other call.




## Pointer input

Clicks, scrolls and hover, on every module. The bar never takes the keyboard
outside a [popup](./modules.md##popups)'s lifetime (its layer surface asks for no
keyboard interactivity, so it cannot disturb focus; a popup that grabbed
takes it, for Escape, arrows and Enter, while it is open), and **touch is ignored**: the bar binds the seat's pointer only, so
a touch screen's taps reach nothing here (until a touch design exists,
they are not translated into clicks).

**Interaction keys.** Each module's table takes five keys, each holding one
action:

| Key | Runs on |
| --- | --- |
| `on-click` | a left click |
| `on-right-click` | a right click |
| `on-middle-click` | a middle click |
| `on-scroll-up` | the wheel or a two-finger swipe up |
| `on-scroll-down` | down |

A value is one of:

```toml
on-scroll-down = "next"                          # an action the module defines
on-middle-click = "activate 3"                   # ... with a whole number
on-click = { exec = ["foot", "-e", "btop"] }     # a command, run directly
# on-click = { scoot = "quit" }                  # ...or a request to scoot's control socket
```

- **A module's own actions** are named, checked when the file is read (a
  typo is a refusal naming the key and listing what the module has), and
  optionally take one whole number. The `clock` has none; `workspaces` has
  `activate N` (switch to the workspace showing number `N` on that bar's
  output, the first if two show it), `activate-position N` (the `N`th as
  drawn, 1 first: what a click on a number means, exact even where two
  items show one number, such as an adopted `2 DP-1` beside a native 2),
  `previous` and `next` (move the active one, stopping at the ends; a
  scroll of several notches moves that many places).
- **`exec`** is an array, the command then its arguments, at most 32 of
  them of at most 4096 bytes each, none holding a NUL. **It is never run
  through a shell**; write `["sh", "-c", "..."]` to use one, and the
  quoting is yours. A string instead of an array is a refusal that says
  so. The command's stdin, stdout and stderr are `/dev/null`, it leads its
  own process group, and **it inherits none of the file descriptors the
  bar opens** (every one is close-on-exec). What was open in the bar when it
  *started* is not the bar's: a launcher (a shell's `exec 4<file`, a
  service manager, a CI runner) that leaves a descriptor open without
  close-on-exec hands it to the bar, and the bar hands it on to every
  command it launches, as to any child. Its environment is the bar's. It is reaped the moment it
  exits (no zombie, no timer), and at most 8 launched commands run at once:
  a ninth is refused with a line on stderr, not queued, so a hung command
  and a flood of clicks cannot fill the process table. The bar does not
  supervise what it launched: it lives as long as it likes (see
  [the unit's `KillMode`](./index.md) for what a
  restart of the bar does to it).
- **`{ scoot = "quit" }`** asks scoot to end the session over its control
  socket (`SCOOT_SOCKET`, else `$XDG_RUNTIME_DIR/scoot.sock`) with no
  process spawn: one line, with a 250 ms bound each way on a fresh
  connection, so a wedged scoot cannot hang the bar. Under another
  compositor, or with no session, it says it cannot reach scoot on stderr
  and does nothing. `quit` is the only value.
- **A key you do not set keeps the module's default**: the workspaces
  module's left click on a number switches to it (as it always did), and
  other modules have defaults of their own, which their sections list (the
  [media module's](./modules.md##media), for one). A binding replaces the default.
- **A failing action** (a program that is not there, a full table) is one
  line on stderr, at most one a second, with a count of the ones held back.
  The bar carries on.

**What a click is.** A button *press* arms the module under the pointer; the
*release* runs the action only if the pointer is still over that same
module. A release anywhere else (the pointer slid off, left the bar, or
the module went away meanwhile: a reload, a module that now shows nothing)
does nothing and leaves nothing armed. A second button pressed while one
is held cancels both. Buttons other than left, right and middle (back,
forward, touch) are ignored. A click lands on the layout that is on screen
(what the last draw committed), so a click during a redraw goes to what you
saw.

**What a scroll is.** Vertical scroll only; horizontal scroll is ignored.
A wheel notch is one step (`axis_value120`, or `axis_discrete` on an older
compositor), and a smooth scroll (a touchpad) adds up to steps at 15 pixels
a step, the remainder carried between events and dropped when the direction
reverses, the scroll ends or the pointer leaves. Down is `on-scroll-down`.
**However fast the device sends events, a scroll binding runs at most once
a frame (16 ms)**, carrying the steps that piled up (at most 32: a flood
past that is dropped, not queued): a module action moves that many
places, and an `exec` command runs once however many steps it covers. While
steps wait the loop sleeps only until the frame is due; with nothing
waiting it sleeps as before.

**Hover.** A module with a binding is drawn in the `hover` color while
the pointer is over it, and only that module's span is redrawn, on that
output's bar alone; moving between modules redraws the one left and the one
entered. A module with no binding is not tinted. The workspaces module
tints its active pill the same way (its pill is drawn by the module
itself, so the tint lands there rather than over its text). Unset,
`hover` follows a custom `accent`; setting it pins the tint.

**Cost.** The bar asks the seat for a pointer only while a placed module
has a binding or a default of its own (today: the workspaces module's
click, the window title's click and the media module's).
A clock-only bar with no bindings never takes the pointer, and costs what
it did before there was any input. A reload that adds or removes bindings
takes or drops it. A motion event stores two numbers; no pointer event,
hover repaint or module action allocates (tests count the allocations).
Launching an `exec` command does, as any process spawn must.



## Popups

A module's **popup** is a small panel drawn in an `xdg_popup` parented to the
bar's layer surface (`zwlr_layer_surface_v1.get_popup`), opened under the
module and gone when closed: nothing of it exists while it is not open, so
a bar that never opens one costs what it did before. The `popup` Cargo
feature (on by default) is the code; a build without it has none of it.

**Opt-in.** Nothing opens a popup until the config binds one. The first
consumer is the volume module (and `microphone`, which shares its code):

```toml
[volume]
on-click = "popup"      # the slider, instead of the default mute
```

Any of the click triggers takes it (`on-right-click = "popup"` keeps the
click for mute); a scroll cannot (it carries no input serial the popup grab
needs), and says so on stderr. `scootbar msg invoke volume popup` opens it
too, **with no grab** (an agent has no input event to grab with): it stays
until invoked again, or any of the endings below, and takes no keyboard.
The network module's picker is unchanged by default (`menu-command` and
the dmenu-style launcher are still how a click connects); `on-click =
"popup"` on the network module opens its native list instead (see
[Network](./modules.md##network)).

- **What the volume popup shows**: the device's name and level (`Built-in
  Audio  49%`), a slider from 0 to `max-volume`, and a Mute (Unmute) button.
  Pressing the slider sets that level and dragging it follows the pointer, one
  `set N` action per new value (the module coalesces a fast drag into the
  latest, one request in flight); the button runs `toggle-mute` on release.
  It follows the module, so a level changed elsewhere moves the slider, and it
   is drawn in the bar's own colors (the `bg`, `fg`, `accent` and `dim`
   tokens, one pixel frame) at the output's real scale.
- **Shape.** A popup is square until the config rounds it: `[bar]
  popup-radius` rounds its four corners, in logical pixels, 0 to 512 and cut
  back per popup to what it holds; unset it follows the bar's own `radius`.
  The corners are transparent (the popup's surface is the bar's own client
  surface, so the compositor's window rounding does not apply: the bar draws
  the arc itself). The border follows the arc at the same width (one logical
  pixel at every scale), and the rows are clipped to the inside arc, so a
  hover fill, a glyph or a slider's end never squares a corner. The corner
  tables are built once per open popup and read every frame it is drawn (no
  allocation while open); the buffers are `ARGB8888` only while rounded (a
  square popup stays `XRGB8888`, as before). The compositor is told the rest
  is opaque, and the surface's input shape is the rounded one, as the bar's
  own is. Tooltips round the same way. Measured in [the resource
  ratchet](https://github.com/scoot-sh/scoot/tree/main/docs/scootbar/backlog/lightest.md#m6-popup-corners).
- **What the network list shows**: one row per named network in scan
  order (unnamed ones are not rows), each starting with its strength
  glyph where `icon-wifi` names four glyphs and the scan carries a
  signal, and the associated one selected. A wheel over the list
  scrolls it a row a notch where it is taller than what the compositor
  configures for it; a row too wide is cut with an ellipsis, as
  a window title's is. **Up and Down move the hovered row** (clamped at
  the ends, never wrapping; text rows are skipped), the scroll following
  so the hovered row stays visible, and **Enter connects to it**, exactly
  as a click would (closing the popup first where `closes` says so). A
  scan that changes the row count while the list is open reopens it at
  the new size (above), losing the scroll position. Selecting a row
  closes the popup and runs `connect N` on the network module. Without
  a grab (an agent's `invoke`-opened popup) there is no keyboard: the
  list is pointer-only there.
- **Opened on the press, not the release.** The one exception to "clicks
  fire on release" ([pointer input](./modules.md##pointer-input)): a compositor may refuse
  a popup grab whose serial is not a button still held (the protocol allows
  it, and some compositors check), and a refused grab never sees a click
  outside or Escape. The release after it does nothing. scoot and sway were
  measured to accept either, so on those the choice is invisible.
- **Where it goes.** Anchored to the module's span on the bar, centered under
  it (above, on a bottom bar), and the compositor slides it along the bar and
  flips it across it where the output's edge would cut it.
- **A refill that changes the size reopens it.** The content is refilled
  whenever something changed, and where its computed size differs from the
  surface it opened at — a tray menu opens on its `...` line and fills a
  turn later, a network scan adds or drops rows — the popup closes and
  opens again at the new size with the grab serial the open earned, so the
  grab survives it. Hover and the scroll position do not survive: the rows
  moved anyway. A same-size refill keeps the surface it has. The reopened
  popup reads the same content and revision, so this fires once per size
  change, never in a loop.
- **It closes** on: a click anywhere outside it (the compositor's `popup_done`),
  **Escape**, a press on the bar (so a second click on the module toggles it,
  and a click on another module closes it without acting), its module
  having nothing to show (the sound server went away) or leaving the bar, its
  output being unplugged, the bar being hidden (`msg hide`) or made again,
  a scale change, a `reload`, and, for a popup that grabbed, the session
  locking (scoot dismisses popup grabs on lock; one opened with `invoke` has
  no grab and is not dismissed by it, and is not drawn over the lock screen). Every one leaves the bar running and
  says nothing on stderr.
- **The keyboard.** The bar's layer surface asks for no keyboard and still
  does not. A `wl_keyboard` is taken from the seat **only while a popup that
  grabbed is open** (the grab is what gives the popup the keyboard), and
  released with it: **Escape** closes the popup, **Up/Down** move the hover
  among its button rows (clamped at the first and last, the scroll following
  so the hovered row stays visible), and **Enter** activates the hovered row
  with its `closes` flag honored, exactly as a release over it would.
  Anything else typed is ignored, and no keymap is ever read (the codes are
  matched directly, so the bar needs no xkb). Without a grab (an agent's
  `invoke`-opened popup) there is no keyboard at all. Keybindings of the
  compositor still win.
- **Limits.** One popup at a time (a [tooltip](./modules.md##tooltips) is not one: it closes
  when a popup opens, and none shows while one is open). A drag ends when the pointer leaves the
  popup (the compositor's popup grab moves the pointer's focus off it, so
  nothing more of the drag arrives); past the slider's ends but still over the
  popup it clamps to them. Up/Down move the hover among the button rows and
  Enter activates the hovered one (above). A compositor
  without `xdg_wm_base` refuses `popup` (said on stderr, or to `invoke`);
  the global is bound while some binding in the config names `popup`, and
  by an `invoke` for a bar with none; that one stays bound until the next
  `reload` (the binds are re-decided on a reload and on registry events, not
  when the popup closes). A bar that never opts in and never invokes binds
  nothing new, and a click is the mute it always was.
- **Cost.** Opening and closing are the only costs: one surface, a
  positioner, two `wl_shm` buffers made on demand and dropped on close, and a
  keyboard. An open popup that nothing changes makes no wakeups and no system
  calls; redraws are one per turn of the loop, however many pointer motions
  arrived, and the popup's own code (content refill, layout, pointer state,
  paint) allocates nothing once it is open. A slider drag still goes through
  the volume module's `set` action, which builds one request per distinct value
  (coalesced to one in flight, as a scroll does). Measured in [the
  resource ratchet](https://github.com/scoot-sh/scoot/tree/main/docs/scootbar/backlog/resolved/popups-done.md#evidence).
- **Writing one** is a module's [`Module::popup`](https://github.com/scoot-sh/scoot/tree/main/crates/scootbar/src/modules/mod.rs)
  (the content: text, a slider, buttons; a list is a column of buttons) and
  the actions its widgets name, which are ordinary module actions, so
  everything a popup does is something a binding or `invoke` could do.



## Tooltips

A module's `tooltip` (the line a module's view carries beside its text) is
shown in a small panel under the module after the pointer has rested on it for
`bar.tooltip-delay` milliseconds, and gone when the pointer leaves. It is the
[popup](./modules.md##popups) machinery without a grab, so the `popup` Cargo feature (on by
default) is also the code of tooltips, and a build without it has no
tooltips and refuses the `tooltip-delay` key.

```toml
[bar]
tooltip-delay = 300     # ms, 0 to 10000; 0 turns tooltips off (default 500)
```

There is no flag for it: the config file only.

- **Which modules have one**, with nothing new read to make it: each module's
  tooltip is the one its view already carried (the window title's full title,
  uncut by the span; the network's interface, SSID and signal; the battery's
  `Charging 80%`; the volume and microphone's device and level; the
  brightness's device; the tray's item titles; the media module's player and line; a `push` or `exec` module's
  `tooltip` key, [the update payload](./modules.md##the-update-payload)). A module with no
  tooltip, or whose tooltip is empty right now, shows none and arms nothing:
  the clock, workspaces and `button` modules have none. A new module's tooltip
  is `tooltip_mut` in its `view` and `Module::tooltips` saying so.
- **When.** The delay runs from the pointer entering the module (motion
  within it does not restart it), and a pointer that crosses a module and
  moves on before the delay shows nothing. Moving to another module hides the
  tooltip and starts that module's delay over.
- **It goes** on: the pointer leaving the module or the bar, **any press**
  (the press then acts as it always does: a tooltip is not a popup, and a
  click is never spent closing one), **any scroll**, a [popup](./modules.md##popups)
  opening, the module's tooltip going empty, its module leaving the bar, its
  output being unplugged, the bar hidden or made again, a
  `reload`, and the session locking (the compositor takes it, and it is never
  drawn over the lock screen). After a press, a scroll or the compositor's
  taking it away it does **not** come back until the pointer has left that
  module. A scale change draws it again at the new scale.
- **One popup at a time, and a click popup wins.** While a popup is open no
  tooltip shows; a popup opening takes a tooltip down. A pointer that rested
  on a module while a popup was open, once it closes, gets that module's
  tooltip after the delay (unless the popup closed on the very module whose
  tooltip had been due or shown, which stays dismissed until the pointer
  leaves).
- **It never takes the keyboard, a grab or a click.** No `wl_keyboard`, no
  `xdg_popup.grab`, and an empty input region, so the pointer never enters it
  and nothing under it is hidden from a click.
- **Where it goes and how big.** Anchored to the module's span like a popup,
  centered under it (above, on a bottom bar), slid along the bar and flipped
   across it by the compositor where the output's edge would cut it. Text is
   wrapped at spaces at 30 ems (never wider than the bar), at most six lines,
   the last ending in an ellipsis (`…`) where it was cut; a word longer than
   the line is broken where it fills it. A newline in a tooltip
   breaks a line (the `push` and `exec` payloads turn control characters,
   newlines included, into spaces, so theirs wrap only). The frame, shape and
   colors are the popup's.
- **While it is shown** a changed tooltip text (a clock-like tooltip) redraws it
  in place when the new text fits the size it opened at, damage limited to the
  tooltip's own surface; a text that needs more room, or its module moving
  along the bar (a neighbor's text grew), makes it again at once with no
  delay.
- **Cost.** With nothing hovered there is nothing: no timer file descriptor (the
  delay is the loop's `poll` timeout, set only while the pointer rests on a
  module with a tooltip that has not shown) and no wakeup. The bar takes a
  `wl_pointer` for tooltips only when tooltips are on and a placed module can
  have one (a bar of a clock alone takes none for them), and binds
  `xdg_wm_base` when the first tooltip shows (kept until the next `reload` or
  registry event, as an `invoke`'s is), so a bar nobody hovers binds nothing
  new. Showing one is a surface, a positioner, an empty region and two
  `wl_shm` buffers made then and dropped when it goes; a shown tooltip makes
  no wakeups and no system calls. A failure to show one (no `xdg_wm_base`, no
  buffer) is silent: a hover is not a request. Numbers: [the resource
  ratchet](https://github.com/scoot-sh/scoot/tree/main/docs/scootbar/backlog/resolved/tooltips-done.md#evidence).



## Button, push and exec modules

Three modules that extend the bar without writing Rust. Each is defined by a
table named for its kind and **a name of your choosing**, and the lists then
place it by that name like any built-in module:

```toml
left   = ["workspaces", "launcher"]
right  = ["weather", "status", "clock"]

[button.launcher]
icon = "\U000f0e65"                         # or icon-path, icon-image: as the clock's
text = "Apps"                               # shown after the icon
on-click = { exec = ["scootlaunch"] }

[exec.weather]
command = ["sh", "-c", "while :; do curl -s 'wttr.in?format=1'; sleep 600; done"]
format = "text"                             # or "json"
placeholder = "..."                         # until the first line

[push.status]
placeholder = ""
```

A name is 1 to 32 letters, digits, `-` or `_` starting with a letter or digit,
is not a built-in module's id (`clock`, `workspaces`) and is unique across the
three kinds; at most 32 modules are defined. Every table also takes `margin`
(as the clock's) and the five [interaction keys](./modules.md##pointer-input), so any of
them can run a command or send `{ scoot = "quit" }` on a click or a scroll.
A table no list names is never started and costs nothing. These modules are
named in the config file's lists only: `--left`, `--center` and `--right`
take the built-in ids. A bad table is refused naming its dotted key
(`exec.weather.command`), and a bad `reload` changes nothing.



### `button`

An icon and/or text that never changes: a launcher, a power menu, a toggle.
`text` and the three icon keys (`icon`, `icon-path` with `icon-viewbox`,
`icon-image`, at most one) are the clock's, with the same refusals. A button
with neither shows nothing and takes no space. It has no fd and no wakeup.
`on-click = { exec = ["scootlaunch"] }` is the launcher and
`on-click = { scoot = "quit" }` log out, in the config
that is the whole of them.



### `push`

A place anything can write to with `scootbar msg set ID VALUE`, costing
nothing until it is: no fd of its own beyond the control socket, no timer, no
thread. `VALUE` is JSON, the [payload](./modules.md##the-update-payload) below:

```sh
scootbar msg set status '{"text": "build ok", "class": "normal"}'
scootbar msg set status '"3 new mails"'      # a JSON string is the text alone
scootbar msg set status null                 # clears it (the module takes no space)
```

`set` only changes what the module shows; it cannot run anything. A value
that is refused (too long, not JSON, a class that does not exist, a
`version` this bar does not speak) leaves what was shown as it was and is
answered by name. Several `set`s that arrive in one turn of the loop are
drawn once; one that changes nothing is not drawn.

A static `icon` (one glyph, or `icon-path` with `icon-viewbox`, or
`icon-image`, at most one, as the [clock's](./modules.md##icons)) stands before the
text, and `show-text = false` draws only the icon
(see [Per-state and per-level icons](./modules.md##per-state-and-per-level-icons)).
An update's own `icon` is drawn instead of the static one while set, so a
script can change it per update (a VPN that drops); with neither the
module shows text alone, as before.



### `exec`

Runs a command and shows what it prints, one line per update. **It streams,
it does not poll**: the command runs once and the bar waits on its output, so
a script that can wait for an event prints when it has one and the bar does
nothing in between. There is **no `interval` key**, on purpose: a script
that has to poll writes `while :; do ...; sleep 60; done` (print first, so
the module shows its line at once, not after the first sleep), which puts the
cost (a process every minute, whatever it spawns) in a script you can see,
not in a bar option. A command that prints once and exits (`["date"]`) is a
poll too: the restart rule below runs it again, backing off to once a
minute, so write the loop yourself when you want a different rhythm. `command` is an array, the program and its arguments, **never
run through a shell** (write `["sh", "-c", "..."]` to use one), at most 32
arguments of at most 4096 bytes. `format` says how a line is read: `text`
(the line is the text, the default) or `json` (one object per line, below).
A static `icon` (one glyph, or `icon-path` with `icon-viewbox`, or
`icon-image`, at most one, as the [clock's](./modules.md##icons)) stands before the
text, and `show-text = false` draws only the icon
(see [Per-state and per-level icons](./modules.md##per-state-and-per-level-icons)).
A JSON line's own `icon` is drawn instead of the static one while set; a
text line carries none, so it shows the static one. With neither the
module shows text alone, as before.

- **What is shown** is the last line of what the bar read at once (an
  update is a state, so earlier ones in the same read are already stale).
  A line that is not valid (JSON mode: not JSON, a bad key, a newer
  `version`) is ignored with one line on stderr, at most one a second, and
  what was shown stays. A blank line shows nothing.
- **Bounds**: a line longer than 4096 bytes is **dropped whole**, never
  truncated, with one warning a second at most: the bar holds one line at
  most however much the command prints. While output keeps coming the bar
  reads at most 4 KiB once every 16 ms and does not even poll the pipe in
  between, so a command printing as fast as it can fills the pipe and
  blocks (the kernel's back-pressure) and costs the bar about 60 small
  reads a second and no memory; one that prints once a minute costs
  nothing between lines.
- **Restarts**: when the command exits it is started again after 1 s, then 2,
  4, ... up to 60 s; a run that lasted 30 s or more starts the sequence
  over. A command that cannot start (no such program) takes the same path,
  and the one warning says why: `exit status: 127, command not found`, or
  `126, not executable`. Status `125` is the bar's for any other reason it
  could not run the command (and then a line before says why), but programs
  exit 125 themselves (`docker run`, GNU `timeout`, `env` and `nice`, `git
  bisect run`), so the warning says only that it is the command's own or the
  bar's and that any line above says why.
  Each restart is said on stderr, throttled to one warning a second per
  module, so a restart that follows another warning within the second is
  silent.
- **Children**: reaped the moment they exit (a pidfd wakes the bar, no
  timer, no zombie). Its stdin is `/dev/null`, its stderr is the bar's own
  (its complaints reach the journal), it leads its own process group, and it
  inherits none of the file descriptors the bar opens (one a launcher left
  open when it started the bar reaches it, as it does any child).
  **The whole process group is
  killed when the module goes**: on a reload that changed its table or
  removed it (one whose table is unchanged keeps its child, pipe, timer
  and shown output, wherever the lists place it now), and when the
  command exits, so a worker it backgrounded does not pile up across
  restarts. **The command ends with the bar, however the bar ends**: on a
  clean exit with the group kill, and on `SIGTERM`, `SIGINT`, `SIGHUP`,
  `SIGKILL`, a crash or the out-of-memory killer by the kernel's
  parent-death signal (`SIGKILL`), which the bar arms by starting the
  command through itself (`scootbar` re-executes as a tiny guard and
  becomes the command, so there is no extra process). What that does **not**
  reach is what the command started in turn: a shell loop dies, and the
  `sleep 60` it was in the middle of runs out its sleep (its next write to
  the closed pipe ends a writer such as `date` or `curl` with `SIGPIPE`), and
  a worker the command backgrounded and left is not touched. A command that
  is a set-user-id program is not covered either (the kernel clears the
  signal on such an `exec`). Commands a pointer binding starts are not
  guarded: a launched application outlives a bar restart on purpose.
  **The guard needs `/proc`** (it is `/proc/self/exe`): where `/proc` is not
  mounted no `exec` module can start, and the warning is ``cannot start
  `/proc/self/exe` to run `sh`: No such file or directory (the bar runs each
  command through itself, which needs /proc mounted)``.
- **Count**: at most 8 `exec` modules are placed (on any output); each
  holds a child, a `timerfd` and at most two polled fds.
- **Start-up**: a command is started right after the bar's first frame, not
  before it, so a slow `fork` never delays the bar.



#### CPU, memory and pressure-stall recipes

There is no built-in CPU, memory, temperature or disk module, on purpose:
`/proc/stat`, `/proc/meminfo`, thermal zones and `statvfs` are state with
no change notification, so the only way to show them live is to sample on a
timer ([the decision](https://github.com/scoot-sh/scoot/tree/main/docs/scootbar/backlog/system-stats-decision.md)).
Each of these is one `exec` module instead: a user who wants a CPU readout
writes the loop with the interval they choose, and pays for it knowingly.
Place them by name like any module:

```toml
right = ["cpu", "mem", "pressure", "clock"]
```

Edits apply on `scootbar msg reload`: changing a recipe's table restarts
that script (its line clears until it prints again), while an unchanged one
keeps running across the reload.

**CPU use**, from `/proc/stat`: two samples a second apart, shown once a
minute (`CPU 42%`). The loop prints first and sleeps after, so the module
shows its line at once instead of after the first sleep; the one-second
sample means the first line lands about a second after the bar starts:

```toml
[exec.cpu]
command = ["sh", "-c", "while :; do set -- $(awk '/^cpu /{print $2+$3+$4, $2+$3+$4+$5}' /proc/stat); sleep 1; awk -v b1=\"$1\" -v t1=\"$2\" '/^cpu /{t=$2+$3+$4+$5; u=(t>t1)?($2+$3+$4-b1)*100/(t-t1):0; if(u<0)u=0; printf \"CPU %.0f%%\\n\", u}' /proc/stat; sleep 60; done"]
format = "text"
placeholder = "..."
```

- **Cost.** Two `awk` reads a minute and two sleeps: a few short-lived
  processes (7 ms of CPU a minute measured), and the bar itself wakes only
  when a line arrives (once a minute) plus the compositor's release. Sampling every second instead
  would cost sixty times the spawns for a number that jitters: keep it
  coarse.

**Memory use**, from `/proc/meminfo`: the used share of what is there
(`MEM 75%`), shown once a minute. `MemAvailable` where the kernel has it,
`MemFree` where it does not:

```toml
[exec.mem]
command = ["sh", "-c", "while :; do awk '/^MemTotal:/{t=$2} /^MemFree:/{f=$2} /^MemAvailable:/{a=$2} END{if(a==\"\")a=f; if(t>0) printf \"MEM %.0f%%\\n\", (t-a)*100/t}' /proc/meminfo; sleep 60; done"]
format = "text"
placeholder = "..."
```

- **Cost.** One `awk` a minute (1 ms of CPU measured); the bar wakes once a minute with it. An
  unreadable `meminfo` prints nothing that tick and the module keeps its
  last line.

**Pressure stalls**, from `/proc/pressure/memory`: no timer at all. PSI
takes a stall-threshold trigger that `poll(2)` waits on, so this script
prints `pressure ok` once, then blocks in the kernel and prints one line
per stall (`stall memory 0.35`), at most one a minute while pressure lasts.
The trigger's window must be a multiple of 2 seconds: anything else is
refused with `EINVAL` (a 1 s window lands on `pressure n/a`, never
`pressure ok`). It needs two things from the machine: `python3` on the
bar's `PATH`, and stall information in the kernel (`CONFIG_PSI=y`, on some
distributions with `psi=1` on the kernel command line).
Save it first:

```sh
mkdir -p ~/.config/scoot
cat > ~/.config/scoot/pressure.py <<'EOF'
#!/usr/bin/env python3
"""Print one line when pressure stalls pass the trigger, then wait again."""
import select
import sys
import time

SOURCE = sys.argv[1] if len(sys.argv) > 1 else "memory"
if SOURCE not in ("cpu", "memory", "io"):
    print(f"pressure.py wants cpu, memory or io, not {SOURCE!r}", file=sys.stderr)
    sys.exit(2)
PATH = f"/proc/pressure/{SOURCE}"
TRIGGER = "some 150000 2000000\n"  # 150 ms stalled in a 2 s window


def wait_forever():
    # No PSI here (or no trigger): block with no timer and no wakeups
    # until the bar ends us. Exiting instead would restart us on backoff.
    select.poll().poll(None)


try:
    f = open(PATH, "r+")
except OSError as e:
    print(f"cannot open {PATH}: {e.strerror}", file=sys.stderr, flush=True)
    print("pressure n/a", flush=True)
    wait_forever()

try:
    f.write(TRIGGER)
    f.flush()
except OSError as e:
    print(f"cannot arm {PATH}: {e.strerror}", file=sys.stderr, flush=True)
    print("pressure n/a", flush=True)
    wait_forever()

print("pressure ok", flush=True)
watcher = select.poll()
watcher.register(f, select.POLLPRI)
while True:
    watcher.poll(None)  # blocks: the kernel wakes us past the trigger
    f.seek(0)
    some = f.readline().split()
    # "some avg10=0.12 avg60=0.03 avg300=0.01 total=12345": numbers only,
    # so the line is short and carries no control characters.
    avg10 = some[1].split("=", 1)[1] if len(some) > 1 and "=" in some[1] else "?"
    print(f"stall {SOURCE} {avg10}", flush=True)
    f.seek(0)
    f.readline()  # consume, so the level trigger re-arms
    time.sleep(60)  # one line a minute at most while pressure lasts
EOF
```

```toml
[exec.pressure]
command = ["sh", "-c", "exec python3 \"$HOME/.config/scoot/pressure.py\" memory"]
format = "text"
placeholder = "..."
```

- **Cost.** One `python3` that sleeps in `poll` while nothing stalls: zero
  wakeups idle, one line per stall event. It stays Python on purpose: only
  a trigger write plus `poll` waits with no timer, which `sh`/`awk` cannot
  do, and the interpreter starts once per bar lifetime (61 ms measured,
  against 17 ms for one `awk`), never once a tick the way the per-minute
  loops above would. `memory` watches allocation
  stalls; swap it for `cpu` or `io` to watch those, and tune the
  `150000 2000000` in the script (the kernel's
  `Documentation/accounting/psi.rst` names the shape; the window must be a
  multiple of 2 seconds). Anything but those
  three is refused on stderr (exit 2) rather than opening a path it should
  not.
- **Without PSI** in the kernel (or a trigger the kernel refuses) the
  module shows `pressure n/a` and the script blocks with no timer until
  the bar ends it. A refused trigger says why first: `cannot arm
  /proc/pressure/memory: Invalid argument` on stderr.

> **Symptom:** the pressure recipe shows nothing but `...`, and the bar's
> log says `exec: python3: not found`, the command ending with `exit
> status: 127, command not found` and restarting after 1 s, 2 s, 4 s and so
> on up to a minute. The bar's environment has no `python3` on its `PATH`:
> a systemd user unit, a minimal container, and a NixOS module all start
> the bar with a small `PATH` that omits it. Give the unit the path (for
> example `Environment=PATH=/run/current-system/sw/bin:/usr/bin:/bin`
> naming the directory that holds `python3`), or the NixOS module the
> package (add `pkgs.python3` to the module's path so `python3` resolves).
> The CPU and memory recipes need no Python: they are `sh`/`awk` on
> `/proc` already, which is why only this recipe names the prerequisite.

**Bounds.** Every recipe prints only numbers its own `printf` formats (a
dozen bytes, no control characters), far inside what the bar takes: a line
past 4096 bytes is dropped whole, text and tooltip are cut at 256 bytes on
a character boundary, and every control character becomes a space
([above](./modules.md##exec)). A `format = "json"` recipe gets the same
treatment per line ([below](./modules.md##the-update-payload)).



### The update payload

What a `push` takes and an `exec` in `json` mode prints per line, scootbar's
own and **deliberately not Waybar's** (no `alt`, no `percentage`, no class
lists, nothing to translate), version 1:

```json
{"version": 1, "text": "72%", "class": "warn", "tooltip": "battery low", "icon": "󰂁"}
```

Every key is optional. `text` and `tooltip` are strings, `class` is one of
`normal`, `warn`, `urgent` or `muted` (colored by the theme's tokens: the
`urgent` and `dim` colors and so on), `icon` is exactly one character (a
glyph from a symbol font, as a static `icon` key takes), drawn before the
text instead of the module's static icon while set, `version` is the shape this was
written for; a `version` above 1 is refused by name rather than half
understood, and keys it does not know are ignored, so later versions can add
some. An older bar, whose version 1 speaks no `icon`, reads an update
carrying one as if it were not there: the key is unknown to it, so it is
ignored and the text shows as before. Text and tooltip are cut at 256 bytes on a character boundary, and
every control character (a tab, a carriage return, an escape) becomes a
space, so nothing but printable text reaches the bar. A line or value
past 4096 bytes, or JSON nested more than 8 deep, is refused. The tooltip is
shown as a [tooltip](./modules.md##tooltips) where the module has one.



### Icons

An icon is drawn before a module's text, `em` device pixels on a side (the size
of the text, at the output's real scale, so it is sharp at 1.5x and never a
smaller bitmap stretched), with a space after it when text follows. Three keys
give one, at most one of them per module; the clock, `button`, `push`,
`exec`, volume,
microphone, network, battery, brightness, bluetooth, media, window-title
and power modules take them, and most of those take one glyph per state
or level besides ([below](./modules.md##per-state-and-per-level-icons)). The workspaces
module takes none: its numbers and pill are the content, and an icon would
say nothing (per-workspace icons would need the compositor to name one,
and no protocol carries any).

| Key | Takes | Drawn |
| --- | --- | --- |
| `icon` | exactly one character: a glyph from a symbol font in the [font chain](./modules.md##fallback-fonts-and-icons) | as text, in the state's color |
| `icon-path` | SVG path data, a `d` string: `M m L l H h V v C c S s Q q T t A a Z z`, up to 16 KiB and 1024 commands | filled by the bar itself, anti-aliased, **tinted from the theme token** of the module's state (`normal` `fg`, `warn` `accent`, `urgent` `urgent`, `muted` `dim`), so it follows Stylix |
| `icon-viewbox` | `"min-x min-y width height"`, only with `icon-path`; default `"0 0 24 24"` | which part of the path's plane is the icon, fitted into the square and centered |
| `icon-image` | an absolute path to a PNG file (a build with `--features icon-image`) | scaled to the size, in its own colors (not tinted) |

```toml
[clock]
# Material's "home", straight from an SVG's <path d="...">:
icon-path = "M10 20v-6h4v6h5v-8h3L12 3 2 12h3v8z"
# Font Awesome-style paths are on a 512 plane:
# icon-path = "M..."
# icon-viewbox = "0 0 512 512"
```

![A path icon at scale 1.5](../../../assets/scootbar-path-1.5x.png)

A path that is not valid is a config error naming `clock.icon-path` and the
byte it stopped at (a refused reload leaves the running bar as it was): the
whole path grammar is accepted, and nothing else, with no guessing. A viewbox
without a path, or two of the three icon keys, is an error too. A PNG is
decoded when the config is read, so a missing, unreadable, non-regular (a FIFO,
a device), huge (past 1024 x 1024 or 8 MiB), truncated or corrupt file is a
config error naming `clock.icon-image` and the file, and the file may be moved
afterward: the bar holds the picture, and drops it at the next reload. Give a
PNG **at least as large as the icon**; it is scaled by a premultiplied
bilinear/area filter and centered keeping its aspect ratio.

![A PNG icon at scale 1.5](../../../assets/scootbar-image-1.5x.png)

Without the `icon-image` feature `icon-image` is an unknown key, and the config
error says so. **SVG files are not read** (a renderer is a large dependency and
an untrusted-markup parser): put the `d` string of a one-color icon in
`icon-path`, or convert a full-color SVG to PNG ahead of time. How it works, the
costs and the decisions are in [icons.md](./configure.md#icons).

#### Per-state and per-level icons

Where one glyph cannot say it — a charge, a signal, a state — the key takes
one glyph, or one glyph per level picked by the value, the shape the
[network module](./modules.md##network) establishes. Each entry takes exactly one
character, as `icon` does; an array takes exactly the count named, each one
glyph; anything else is a config error naming the dotted key. A per-state
glyph wins over the static icon for its own state (any other state shows
the static one), and a static path or picture has no levels: per-level
vector or PNG icons are out of scope.

| Module | Keys | Picks |
| --- | --- | --- |
| `network` | `icon-ethernet`, `icon-wifi`, `icon-vpn`, `icon-offline` | the state; `icon-wifi` takes one glyph, or 4 (weakest to strongest), picked by the signal level |
| `battery` | `icon`, plus `icon-charging` and `icon-full` | `icon` takes one glyph, or 5 (empty to full, quintiles: 0–19, 20–39, 40–59, 60–79, 80–100); charging wins over the level, full over charging |
| `brightness` | `icon` | one glyph, or 4 (dim to bright, quartiles: 0–24, 25–49, 50–74, 75–100) |
| `bluetooth` | `icon-off`, `icon-on`, `icon-connected` | the state |
| `media` | `icon-playing`, `icon-paused` | the state (a stopped player shows nothing, so there is no third key) |
| `window-title` | `icon` | one static glyph, whenever a window is focused (never for the placeholder) |
| `power` | `icon`, plus `icon-lock`, `icon-logout`, `icon-suspend`, `icon-reboot`, `icon-poweroff` | one glyph per menu row, drawn before its label; a row with none shows its label alone |
| `push` | `icon` | one static glyph, drawn before the text; an update's own `icon` wins for its own update |
| `exec` | `icon` | one static glyph, drawn before the text; a JSON line's own `icon` wins for its own line (a text line carries none) |

`show-text = false` draws only the icon, with the text moved into the
tooltip — which already names what the text said on every one of these
modules (the battery's `Discharging 72%`, the brightness's device, the
bluetooth state and device list, the media line, the full window title, the
network's SSID and signal, and whatever tooltip a `push` update or an
`exec` line named, or the text itself where it named none) — so an icon can stand alone where the text is
just a value: brightness and bluetooth especially. Without any icon the
module shows text alone, as before.

#### Example: Nerd Font icons for every module

Glyphs from a symbol font in the [font chain](./modules.md##fallback-fonts-and-icons)
(Nerd Font's Material Design set; each codepoint verified against
[Pictogrammers/MDI](https://pictogrammers.com/library/mdi/)):

```toml
[bar]
fallback-fonts = ["/path/to/SymbolsNerdFont-Regular.ttf"]

[battery]
icon = ["\U000F008E", "\U000F007B", "\U000F007E", "\U000F0081", "\U000F0079"]   # battery-outline, battery-20, battery-50, battery-80, battery: empty to full
icon-charging = "\U000F0084"   # battery-charging
icon-full = "\U000F0079"       # battery

[brightness]
icon = ["\U000F00DD", "\U000F00DE", "\U000F00DF", "\U000F00E0"]   # brightness-4, brightness-5, brightness-6, brightness-7: dim to bright, a sun at every level

[bluetooth]
icon-off = "\U000F00B2"        # bluetooth-off
icon-on = "\U000F00AF"         # bluetooth
icon-connected = "\U000F00B1"  # bluetooth-connect

[media]
icon-playing = "\U000F040A"    # play
icon-paused = "\U000F03E4"     # pause

[window-title]
icon = "\U000F08C6"            # application

[network]
icon-wifi = ["\U000F091F", "\U000F0922", "\U000F0925", "\U000F0928"]   # wifi-strength-1..4: weakest to strongest

[power]
icon = "\U000F0425"   # power
```
