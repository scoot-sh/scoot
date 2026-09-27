![scoot — a cat swiping three terminal windows sideways across the screen,
trailing motion lines](docs/assets/logo.png)

# scoot

[![CI](https://github.com/scoot-sh/scoot/actions/workflows/ci.yml/badge.svg)](https://github.com/scoot-sh/scoot/actions/workflows/ci.yml)

A scrolling-tiling Wayland compositor: windows sit in columns on a strip
that scrolls sideways, a layout it owes to
[niri](https://github.com/niri-wm/niri).

![Three scoot columns: htop, vim, and a file tree stacked above a shell
querying the compositor over IPC](docs/assets/screenshot.png)

Two needs drove scoot's creation:

- **Running with no GPU and no OpenGL.** scoot renders on the CPU with
  [pixman](http://pixman.org/) by default, so it runs as a full `--tty`
  session on a KMS display (every connected monitor), including VMs and machines
  with no 3D acceleration; `--headless` with no display at all
  (`--outputs N` for several virtual screens); or `--nested` inside another
  compositor, including a GPU-less container such as
  [webtop](https://github.com/linuxserver/docker-webtop). A GPU is optional,
  not unwelcome: with one, an opt-in tier (`--renderer gles` from a
  `gpu-scanout` build) scans out from it under `--tty`, using 4–5x less
  CPU under load on an Apple M2 — see
  [docs/tty.md](docs/tty.md#which-renderer-draws-the-frames).
- **Being driven by an agent doing computer use.** One socket covers,
  among other things, layout actions (`scootctl action`), key presses
  (`key`), text typed correctly for the active keyboard layout (`type`),
  absolute pointer moves and clicks (`pointer move`, `pointer click`), PNG
  screenshots sent back on the socket with the pointer drawn in
  (`screenshot`, `--no-cursor` to omit it), waiting until the screen
  settles (`wait-idle`), and window and output queries (`windows`,
  `outputs`) in which every window reports its rectangle in the same
  coordinates `pointer click` takes. A connection can also subscribe to
  output removed/restored events (`scootctl subscribe`) instead of polling
  for them. scoot's own end-to-end test drives it
  this way. Because the socket can inject any keystroke, it lives in
  `$XDG_RUNTIME_DIR` (override with `$SCOOT_SOCKET`), is created `0600`,
  and serves only the compositor's own user. Reference:
  [docs/ipc.md](docs/ipc.md).

It is real enough to use: **confirmed working on Apple Silicon under Asahi
Linux, and daily-driven on `--tty`** (2026-09-18).

If scoot doesn't fit what you need, you should absolutely check out
[niri](https://github.com/niri-wm/niri). It's awesome, and it's what
inspired this project.

## Not yet

- **Multi-output is missing per-output configuration.** `--tty` drives
  every connected monitor at once, each its own output with its own
  scrolling strip, framebuffer, lock surface, workspace group and gamma
  control. New windows open on the output under the pointer, and
  `Super+comma`/`Super+period` (with `Shift` to carry the focused window)
  move between the first two outputs. Plugging a monitor in while the session runs adds
  an output for it. Pulling one out removes its output and moves its
  windows, as their own workspaces, onto a remaining screen after its own;
  the last screen is never taken away. When the window you were working on
  was on the monitor you pulled, that screen switches to its workspace with
  your window still focused -- nothing vanishes, and `Super+1` (or
  `Super+Ctrl+k`) takes you back to the screen's own previous workspace. A
  bar shows the adopted workspaces with their monitor's name ("2 DP-1"), and
  `scootctl windows` reports each window's workspace. What is
  not there yet:
  - **Per-output scale, mode and position.** One `[output] scale` applies
    to every screen and `--mode WxH` to every connector that offers that
    size. Screens sit side by side, left to right: in connector order at
    startup, and a monitor plugged in later goes on the right.
    `wlr-output-management` `apply`/`test` stay refused.
  - **A replugged monitor comes back with its windows.** Its workspaces move
    back from the screen they were adopted by -- the windows that are still
    open and that were not moved by hand in between, in the same order with
    the same active workspace -- and the adopting screen goes back to the
    workspace it showed before, with the focused window following its own
    window home; a standby cycle that drops the connection while you work
    on the other screen leaves that screen exactly as it was. The default
    `Super+period` binds follow it (they name the second screen, not output
    id 2). A *different* monitor
    on the same connector does not inherit them: identity is the connector
    name plus the EDID make/model/serial where one can be read.
- **XWayland is opt-in, and partial.** X11 applications run with
  `--xwayland` (or `[xwayland] enabled`) in an `xwayland` build (`cargo
  build --release --features xwayland`, with `Xwayland` on `PATH`; no flake
  output ships it yet): their windows tile, dialogs float, fullscreen
  works, and they take focus by themselves only when nothing is focused,
  when they belong to the X app in use, or when scoot started them.
  Running one extends full trust to it — an X11 client can read and cover
  other windows by design ([protocols.md](docs/protocols.md#xwayland-opt-in)).
  Copy and paste works between X and Wayland apps both ways (clipboard
  and middle-click primary; `xclip`/`xsel` and `wl-copy`/`wl-paste` see
  each other), but an X app reads or sets it only while an X window has
  the keyboard. Drag-and-drop works in every direction: X to Wayland,
  Wayland to X, X to another X app, and within one X app (moving selected
  text, say); touch drags from X apps are refused
  ([details](docs/protocols.md#clipboard-drag-and-drop-and-input-methods)).
  Not there yet: X input methods (XIM).
- **GPU scanout is opt-in.** With a real GPU it is worth trying: on an
  Apple M2 under Asahi Linux it uses **4–5x less CPU** than the default under
  load, puts the same pixels on screen, and costs 7–16 MB more memory
  ([Asahi.md](Asahi.md), Test 4). Turn it on with a `gpu-scanout` build
  (`nix build .#scoot-gpu`) and `scoot --tty --renderer gles`. There, a
  fullscreen window can be shown straight from the app's own buffer, with
  no compositing, when the display accepts that buffer. On an Apple M2 a
  fullscreen mpv goes direct and scoot uses about 60% less CPU for it.
  On a display with no cursor plane (Apple Silicon's has none) the drawn
  pointer forces compositing, so nothing goes direct while the pointer is
  visible. Even with it hidden, the app's buffer must be one the display
  takes, in layout and in size ([Asahi.md](Asahi.md), Test 5). scoot also tells a fullscreen app which buffer layouts the
  display can show that way, and Mesa's GL apps switch to one (Test 6).
  While something records or streams the screen, scoot
  composites as usual. Not there yet:
  every other window is still composited. Under
  `--headless` the GPU speedup does not apply: `gles` there still copies
  every frame back to the CPU, and on a machine without a real GPU that
  measured 18–31x *slower* than pixman. Under `--nested`, a `gpu-scanout`
  build hands each frame to the host compositor as a GPU buffer, with no
  copy back to the CPU, when the host composites on the same GPU (the
  startup log says whether it does, and why not). On an Apple M2 that
  halves the nested scoot's CPU, nested in niri as well as in scoot
  ([Asahi.md](Asahi.md), Test 8).
  pixman stays the default and the right choice without a GPU; details in
  [docs/tty.md](docs/tty.md).
- **GPU apps get their GPU's own buffer formats under `--renderer gles`** —
  the layouts the GPU prefers and the YUV formats video decoders produce,
  not only plain linear RGB. On an Apple M2 that is 54 formats, each
  offered tiled, compressed and linear, and GL and Vulkan apps pick the
  compressed layout ([Asahi.md](Asahi.md)'s Test 6).
- **GPU apps that use explicit sync (NVIDIA's driver relies on it, Mesa's
  Vulkan drivers use it where offered) are supported on the GPU tier**,
  where the GPU device supports it: scoot waits for an app's GPU to finish
  a frame before showing it, and tells the app when it may reuse a buffer.
  It is offered only there, never under pixman or `--headless`/`--nested`.
  Seen working with Mesa's Vulkan driver (`vkcube`) on an Apple M2; not
  yet with NVIDIA ([Asahi.md](Asahi.md), Test 7).
- **Config reload is live, except three restart fields.** `scootctl reload` (or `kill -HUP` on the
  compositor) re-applies the layout (gap, column widths, default column
  width), the output scale (except under `--nested`, where the host owns
  it), the
  appearance (including the cursor size, color and theme), the keybindings,
  `[floating]` and the window rules (for windows that open afterwards),
  and new `[autostart]` spawn entries (only entries the session has not seen
  run; a reloaded non-`spawn` entry is refused by name, a spawn whose program
  fails to start is refused by name and retried on the next reload, and a locked reload
  defers new entries to the first unlocked one) live; the DRM device
  (`[tty] gpu`), the renderer (`[renderer] backend`) and the XWayland knob
  (`[xwayland] enabled`) take effect on
  restart, and a reload refuses them
  with a message naming that rather than silently ignoring them.
- **No macOS adapter.** `scoot-core` is kept platform-independent so one can
  exist, but nothing drives the Accessibility API yet. On macOS you get
  `scootctl`, the remote-control client, only.

## Install

Clone, then, from the flake at the repo root:

```sh
nix build                            # ./result/bin/scoot
nix build .#scoot-gpu                    # ...with the --tty GPU scanout tier
nix build .#scootctl                 # ./result/bin/scootctl, the client alone
nix build .#scootbg                  # ./result/bin/scootbg, the wallpaper daemon (early; Linux)
nix run . -- --headless -- foot      # build and run it in one step
nix run .#scootctl -- windows        # the client, from anywhere
```

On Linux that builds the compositor. On macOS the default is `scootctl`
alone — enough to drive a compositor running in a VM. The flake
covers Apple Silicon only; an Intel Mac builds the same client with `cargo
build -p scootctl`.

To consume scoot from your own flake (the way a NixOS user actually
installs a compositor) rather than from a clone:

```nix
inputs.scoot.url = "github:scoot-sh/scoot";
# then, in a module:
environment.systemPackages = [ inputs.scoot.packages.${pkgs.system}.scoot ];
```

For the full story — that snippet plus the home-manager module
(`programs.scoot.settings` rendering `~/.config/scoot/config.toml`,
session script hook, portal backend install) and the NixOS module
(package plus an opt-in, strictly-additive login-screen session entry)
— see [docs/nix.md](docs/nix.md).

## Running

```sh
scoot --headless --width 1280 --height 800 -- foot   # start, spawn a terminal
scoot --headless --outputs 2 -- foot                 # two virtual screens, side by side
scoot --nested --width 1280 --height 800 -- foot     # inside your compositor
scoot --tty -- foot                                  # on a real DRM/KMS seat
scoot --tty --renderer gles -- foot                  # ...scanning out from the GPU

scootctl windows                                    # in another shell
scootctl action focus-column left
scootctl screenshot --out /tmp/shot.png
scootctl type "hello"
```

With a `[wallpaper]` section in the config (see [Configuring](#configuring)),
scoot starts `scootbg`, the wallpaper daemon, itself: at startup, and again
on every `scootctl reload`. It needs `scootbg` on `PATH` (or `[wallpaper]
command`); without it the session starts anyway, with a warning in the log.

`scootctl` is the client every example on this page uses; `scoot msg ...`
is the same client kept as a permanent alias on the compositor binary.
Screenshots show the pointer, the same way whichever backend and renderer
the session runs (`scootctl screenshot --no-cursor` leaves it out; `grim`
draws it with `-c`) — see [docs/ipc.md](docs/ipc.md#the-pointer-in-a-screenshot).

`--tty` needs a seat (`seatd` or logind) with a DRM device on it; everything
about device choice, hotplug and modes is in [docs/tty.md](docs/tty.md). Run
`scoot --help` for every flag, `scootctl --help` for every request and action.
`scoot --version` (or `scootctl --version`) identifies a build — its own
version plus the IPC protocol number — without starting anything.

`--headless --outputs N` (1–8) creates N virtual outputs side by side, so
per-output behaviour is testable with no second monitor: each gets its own
`wl_output`, its own place in the coordinate space, its own scrolling
strip (a window is drawn and clicked only on its own output, never over the
next one), its own composited strip, its own layer-shell zones and input, its
own lock surface, its own workspace group and output-management head — so
`scootctl screenshot --output 2` answers with the second output's own
pixels, a bar on one output reserves space only there, the pointer crosses
onto the second screen instead of trapping on the first, and a session lock
blanks every output before it confirms.
`--nested` and `--tty` warn and ignore the flag: `--nested` has one host
window, and `--tty` already makes one output per connected monitor (see
[docs/tty.md](docs/tty.md#more-than-one-monitor)). See
[docs/configuration.md](docs/configuration.md#more-than-one-output) for what a
second output does and does not do yet.

## Keys

| Combo | Action |
|---|---|
| `Super+h` / `Super+l` | Focus column left / right |
| `Super+j` / `Super+k` | Focus window down / up in the column |
| `Super+Shift+h` / `Super+Shift+l` | Move column left / right |
| `Super+Shift+j` / `Super+Shift+k` | Move window down / up |
| `Super+Alt+h` / `Super+Alt+l` | Consume or expel column left / right |
| `Super+Ctrl+j` / `Super+Ctrl+k` | Focus workspace down / up |
| `Super+Ctrl+Shift+j` / `Super+Ctrl+Shift+k` | Move window to workspace down / up |
| `Super+1`..`Super+9` | Focus workspace 1–9 directly |
| `Super+Shift+1`..`Super+Shift+9` | Move window to workspace 1–9 directly |
| `Super+comma` / `Super+period` | Focus first / second screen |
| `Super+Shift+comma` / `Super+Shift+period` | Move window to first / second screen directly |
| `Super+r` | Cycle column width |
| `Super+f` | Toggle fullscreen |
| `Super+Shift+Space` | Float the focused window, or put it back in the strip |
| `Super+Space` | Move focus between floating windows and the strip |
| `Super` + left-drag | Move a floating window (the modifier is `[floating] modifier`) |
| `Super` + right-drag | Resize a floating window from the nearest edge or corner |
| `Super+q` | Close focused window |
| `Super+Return` | Spawn `foot` |
| `Super+Shift+e` | Quit |
| `Ctrl+Alt+F1`..`F12` | Switch VT (`--tty` only) |

## Configuring

Drop a TOML file at `~/.config/scoot/config.toml`, or pass `--config PATH`.
Everything in it is optional. No file yet? `scoot --print-default-config >
~/.config/scoot/config.toml` writes a commented starting one from the live
defaults (or `scoot --print-default-config --write` to place it there
directly — it refuses rather than overwriting anything already there):

```toml
[layout]
gap = 8
column_widths = [0.25, 0.5, 0.75, 1.0]

[appearance]
focus_ring_active_color = "#ffaa00"
background_color = "#101014"

[binds]
"super+t" = "spawn foot"
"ctrl+alt+space" = "spawn wofi --show drun"

[autostart]
commands = ["spawn waybar"]

# The wallpaper: scoot runs scootbg (install it; the Nix modules do) with
# this at startup and on every reload. `scootbg set` overrides it until you
# next change the section.
[wallpaper]
image = "~/Pictures/hills.jpg"   # or: color = "#1e1e2e"

# Float this app's windows above the strip instead of giving them a column
# (pavucontrol's app id is org.pulseaudio.pavucontrol; `scootctl windows`
# shows any window's).
[[window_rule]]
match_app_id = "*pavucontrol"
float = true
```

How a session starts its programs — a session script (`scoot -- ...`) vs
`[autostart]` — is in
[docs/configuration.md](docs/configuration.md#starting-a-session).
`[wallpaper]` (its keys, what wins over `scootbg set`, what happens when
scootbg is missing) is in
[docs/configuration.md](docs/configuration.md#wallpaper).

Every table, field, default and failure mode is in
[docs/configuration.md](docs/configuration.md). Almost nothing in it can stop
scoot starting — a bad value is logged and falls back to the default.
The two exceptions are deliberate, because guessing would be worse than
refusing: `[tty] gpu` naming a device that will not open, and
`[renderer] backend = "gles"` on a box with no working EGL.

## What works

| If you want to… | scoot has | Detail |
| --- | --- | --- |
| Run a bar, dock, wallpaper, launcher or notification daemon | `wlr-layer-shell-v1`, with keyboard focus | [protocols.md](docs/protocols.md#layer-shell-bars-wallpapers-launchers) |
| Have dialogs and pop-ups float instead of taking a column | confirmation dialogs, file pickers and other transient or fixed-size windows float centred above the strip automatically; `[[window_rule]]` floats (or keeps tiled) any app by app id or title; `Super+Shift+Space` or `scootctl action toggle-floating` toggles one. Drag one with `Super`+left, resize it with `Super`+right, or by its own titlebar and borders; agents use `scootctl action move-floating`/`resize-floating` | [configuration.md](docs/configuration.md#floating) |
| Watch a video or play a game fullscreen | the app's own fullscreen button, `Super+f`, a taskbar, or `scootctl action toggle-fullscreen` — edge to edge, bar hidden; notifications on the `overlay` layer stay on top (mako defaults to `top`: set `layer=overlay`) | [protocols.md](docs/protocols.md#fullscreen) |
| List and switch workspaces from a bar | `ext-workspace-v1` | [protocols.md](docs/protocols.md#workspaces-ext-workspace-v1) |
| List, focus and close windows from a taskbar | `ext-foreign-toplevel-list-v1` **and** the wlr one | [protocols.md](docs/protocols.md#window-lists-two-protocols) |
| Lock the screen | `ext-session-lock-v1`, compositor-enforced | [protocols.md](docs/protocols.md#screen-locking-ext-session-lock-v1) |
| Auto-lock or dim on idle | `ext-idle-notify-v1` + `idle-inhibit-v1` (`swayidle`) | [protocols.md](docs/protocols.md#idle-detection) |
| Screenshot or screen-share | `ext-image-copy-capture-v1` (`grim`; the pointer when asked, `grim -c`), plus `scootctl screenshot` | [protocols.md](docs/protocols.md#screen-capture-ext-image-copy-capture-v1) |
| Run a GPU-rendering client, with or without a GPU on the compositor | `zwp_linux_dmabuf_v1`, formats taken from whichever renderer is active (the GPU driver's own, YUV included, under `gles`) | [protocols.md](docs/protocols.md#gpu-rendering-clients-zwp_linux_dmabuf_v1) |
| Run a GPU app that uses explicit sync (NVIDIA, Vulkan) | `linux-drm-syncobj-v1`, on the `--tty` GPU tier only, where the device supports it | [protocols.md](docs/protocols.md#explicit-sync-linux-drm-syncobj-v1) |
| Clipboard manager, middle-click paste | `wlr-`/`ext-data-control`, `primary-selection-v1` | [protocols.md](docs/protocols.md#clipboard-and-primary-selection) |
| Night light | `wlr-gamma-control-v1` (`wlsunset`, `gammastep`) | [protocols.md](docs/protocols.md#night-light-wlr-gamma-control-v1) |
| A HiDPI display | `[output] scale`, integer and fractional | [protocols.md](docs/protocols.md#output-scaling) |
| An IME or on-screen keyboard | `text-input-v3` + `input-method-v2` | [protocols.md](docs/protocols.md#input-methods-text-input-v3-input-method-v2) |
| A drawing tablet | `tablet-v2` — tools yes, pads no | [protocols.md](docs/protocols.md#drawing-tablets-tablet-v2) |
| Read display modes (`wlr-randr`, Settings → Display) | `wlr-output-management-v1`, **read-only** | [protocols.md](docs/protocols.md#display-information-wlr-output-management-v1) |
| Drive the session from a script or an agent | the control socket: input injection, screenshots, introspection | [ipc.md](docs/ipc.md) |
| Run scoot inside another compositor (webtop, a nested test session) | `--nested`, following the host window's size as it changes | [configuration.md](docs/configuration.md#command-line-flags) |
| Run X11 applications | `--xwayland` / `[xwayland] enabled` (opt-in; X clients are fully trusted by design); clipboard and primary selection cross both ways while an X window is focused; drag-and-drop works in every direction between X and Wayland apps, X to X and within one X app included (not from touch); no XIM | [protocols.md](docs/protocols.md#xwayland-opt-in) |

The full protocol/version table, and the ones that are deliberately absent,
are at the top of [docs/protocols.md](docs/protocols.md).

## scootbg (early)

`scootbg` is scoot's wallpaper daemon, a separate binary and package that
works on any compositor with `wlr-layer-shell`. **It is early: it shows a
solid color or an image (PNG, JPEG, WebP) on every output, or on one, and
shows it again when the daemon next starts.** In scoot, a `[wallpaper]`
section in the config is all it takes (see [Configuring](#configuring));
scoot runs `scootbg apply-config` with it. What works today:

```sh
scootbg daemon                        # connect to $WAYLAND_DISPLAY and serve the control socket
scootbg daemon --profile sway         # restore and save the `sway` profile's state instead
scootbg daemon --no-restore           # start with nothing shown (the state is kept)
scootbg set '#1e1e2e'                 # every output, including ones plugged in later
scootbg set '#101014' --output DP-2   # one output, by connector name (as `query` lists them)
scootbg set ~/Pictures/hills.jpg      # an image, covering every output (--mode fill)
scootbg set ./#draft.png              # a file whose name starts with '#'
scootbg set city.png --output DP-2 --mode fit --fill '#101014'
scootbg set tile.png --mode tile --filter nearest
scootbg clear                         # back to the compositor's own background
scootbg clear --output DP-2           # ... on one output
scootbg query                         # each output, its surface and what it shows, one JSON line
scootbg version                       # the running daemon's version and protocol
scootbg kill                          # stop it; returns once a new daemon can start
scootbg apply-config --profile scoot '{"color":"#1e1e2e"}'
                                      # what scoot runs with its [wallpaper] section (below)
scootbg --help                        # and `scootbg COMMAND --help`
```

**Colors** are `#rrggbb`, six hex digits in either case; quote them, since
the shell reads `#` as a comment. No `#rgb` shorthand, no alpha (a
wallpaper is opaque). **Anything not starting with `#` is an image path**,
so a file whose name starts with `#` is given as `./#name.png`. The path is
made absolute before it is sent (the daemon's working directory is not
yours) and must be valid UTF-8. PNG, JPEG and WebP are read, told apart by
content, not by name (an animated PNG or WebP shows its first frame);
transparency is shown over the fill color, and EXIF orientation is
applied (a JPEG's, a WebP's, or a PNG's `eXIf` chunk). `--mode` fits it to each output:

| `--mode` | |
|---|---|
| `fill` (default) | cover the output, cropping what overflows, centred |
| `fit` | all of it, as large as fits, centred, the rest in `--fill` |
| `stretch` | the output's size, whatever the aspect |
| `center` | unscaled, centred: cropped if larger, the rest in `--fill` |
| `tile` | unscaled, repeated from the top-left corner |

`--fill '#rrggbb'` is the color around a fitted or centred image (default
`#000000`), `--filter lanczos3|catmull-rom|bilinear|nearest` the scaling
filter (default `lanczos3`; `nearest` keeps pixel art hard). `--mode`,
`--fill` and `--filter` with a color are a usage error. The image is
decoded and scaled on a worker thread and the decoded pixels are dropped
once drawn. Outputs of one size showing one image share one buffer's
memory (32.4 MB at 4K, however many outputs show it). An output plugged
in later shares the pixels of an output of its size already showing the
image, with no decode; otherwise it, like a new scale that needs a new
size, reads the file again. Images over 16384×16384 pixels are refused.

**Images are drawn at each output's real device pixels, fractional scales
included.** On a compositor with `wp_fractional_scale_v1` and
`wp_viewporter` (scoot, sway, and most others) the buffer is the surface's
logical size times the scale the compositor asks for, rounded as that
protocol says: 1601×1001 for scoot's 1067×667 surface at 1.5 on a
1600×1000 output, which scoot draws one buffer pixel to one device pixel
(the last column and row fall off the edge). An image the size of the
output with `--mode center` comes out exact to the pixel. Without those
protocols it is drawn at the integer scale (the larger of `wl_surface`'s
preferred buffer scale and `wl_output`'s) and the compositor scales it
down: sharp, but larger than the output (2134×1334 there). The same
happens for a moment when the compositor has not yet told a surface its
new scale (sway does not while nothing is shown on it): the image is on
screen when `set` returns, drawn larger and scaled down, never stretched,
and redrawn exact once the compositor sends the scale. A compositor scale that is
not a multiple of 1/120 (1.33, say) cannot be drawn exactly by any client,
since the protocol cannot say it.

`set` with no `--output` replaces every choice, per-output ones included;
with `--output NAME` it sets that output only, and the choice is kept by
name, so the monitor shows it again when it is unplugged and plugged back
in. A name no output has right now is an error (exit 1) and changes
nothing. `clear` does the same with nothing to show.

**`set` and `clear` return once it is on screen:** every targeted output
shows the change and the compositor has processed it (a `wl_display.sync`
round trip after the commits), so a screenshot taken straight after shows
it. An output unplugged meanwhile is left out of that wait; one whose
surface is not configured yet is waited for, but only until a round trip
after scootbg made that surface: one the compositor has not configured
by then no longer holds up the reply (the daemon says so on stderr) and
is drawn when it is; one scootbg has given up on
(`gave-up` in `query`, said on stderr) is left out and shows nothing, and
`set` still exits 0. They print nothing on
success. **An image that cannot be shown** (no such file, not a regular
file, not a PNG/JPEG/WebP, too large, truncated or corrupt) is an error
saying why, and every output keeps what it showed. **The newest request
wins**: a `set` or `clear` sent while an earlier image is still decoding is
never undone when that image finishes; the earlier `set` changes nothing
and returns 0 once the newer one is on screen, as a replaced color's `set`
does. Exit status: 0 done; 1 no daemon running,
unknown output, image that cannot be shown, or drawing failed (the
daemon's stderr says why); 2 usage error, a malformed color or an unknown
`--mode`/`--filter` included.

On a compositor with `wp_single_pixel_buffer_manager_v1` and
`wp_viewporter` (scoot, sway, and most others) a color costs no shared
memory at all: one single-pixel buffer scaled to the output. Without
single-pixel buffers it is a 1×1 `wl_shm` buffer under the viewport, and
without a viewporter a full-size buffer. A static color asks for no frame
callbacks and wakes the daemon for nothing.

Once on screen a wallpaper costs no CPU (no wakeups in a minute, measured)
and one buffer per output at most: about 4.0 MB RSS with a color, 36.8 MB
with an image on a 4K output, and the same 36.8 MB with two 4K outputs
showing it. The measured budget is in
[`docs/scootbg/README.md`](docs/scootbg/README.md#the-resource-budget).

One daemon per display: its socket is
`$XDG_RUNTIME_DIR/scootbg-NAME.sock`, `NAME` being the last component of
`$WAYLAND_DISPLAY`. A second `scootbg daemon` refuses while one runs, and
a socket left by a dead one is replaced. It exits 0 on `kill` and 1 when
the compositor goes away, removing its socket either way. A signal
(SIGTERM, Ctrl-C) ends it on the spot and leaves the socket file; the other
commands then say no daemon is running (exit 1), and the next
`scootbg daemon` replaces the file. Every command exits 2 on a usage
error (an unknown command or argument, or a bad `--profile` name).

**`scootbg apply-config [--profile NAME] JSON`** is how a config, rather
than a person, sets the wallpaper: scoot runs it at start-up and on
every reload with its `[wallpaper]` section as JSON (`{}` once the section
is gone). The JSON is one object: `image` (an absolute path) or `color`
(`#rrggbb`), with `mode`/`fill`/`filter` for an image as `set` takes them;
`output` holding a table of the same keys per connector name (each table
stands alone: an output's image does not take the top level's `mode`);
and `command` (scoot's, ignored). Anything else (an unknown key, a key
given twice, `null`, a relative path, a bad color, mode or filter, over
256 outputs or 63 KiB) is refused with exit 2, so a typo is never a
setting silently not applied.

- **Whichever you changed last wins.** The section is applied only if it
  changed since the last `apply-config` for that profile (a SHA-256
  fingerprint of it, `command` left out, kept in the profile's state
  file). So a `scootbg set` made afterwards keeps showing across restarts
  and reloads until the section itself changes. The one order it cannot
  see: a section removed while scoot is not running and re-added unchanged
  before the next start counts as unchanged.
- **It starts the daemon when none runs**, detached (a session of its own,
  its stderr `apply-config`'s: send that to a file or a log, not a pipe
  read to its end), starting from the section, so the saved state never
  flashes first. Two started at once make one daemon, settled by its lock.
  With no daemon and an empty section it starts none, and records the
  clear in the profile's state instead.
- **A running daemon adopts the profile** it is sent, whatever
  `--profile` it started with, so an `[autostart]` `scootbg daemon` ends up
  on scoot's profile too. `scootbg query` reports the profile in use
  (`"profile"`, after `"saving"`).
- **Another build is reported:** a daemon of another version is a
  warning (the section is still sent), one of another protocol or too old
  for `apply-config` an error.
- It returns once every output shows it, as `set` does, and prints nothing
  on success. Exit status: 0 applied or unchanged, and shown; 1 no daemon
  could be started or reached (5 s), no reply (30 s), the daemon closed
  the connection before answering (`scootbg kill`, a crash), another
  protocol, a daemon too old for `apply-config`, an image in the section
  that is not a file (the rest is applied; reported by every
  `apply-config` until the file is back, and then shown), drawing failed,
  or the state file could not be written; 2 a usage error.

The schema, the fingerprint's exact encoding, the precedence table and
the protocol are in
[docs/scootbg/README.md](docs/scootbg/README.md#apply-config-scoots-wallpaper-section).

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
  is restored once the file is back.
- **`--no-restore`** starts with nothing shown. The state is still read,
  and a `set` or `clear` then updates it as usual, keeping the rest.
- **A state file it cannot use never stops the daemon.** A malformed line
  (or one whose path is not UTF-8) is skipped with a warning and the rest
  is used; a file that is not one (no `scootbg-state 1` first line, over
  256 KiB) restores nothing and is replaced at the next save; a newer
  scootbg's format (a later version) restores nothing and is never
  written over; an unreadable one restores nothing, and nothing is saved
  over it. In those two cases **saving is off until the daemon
  restarts**: `set` still works on screen, stderr says at start-up which
  file to remove or fix to save again, and `scootbg query` reports
  `"saving":false`. The format is a documented, versioned line format
  ([docs/scootbg/README.md](docs/scootbg/README.md#restore)).
- **A restored image is decoded once**, when its outputs are configured:
  about 450 ms to a 6000×4000 JPEG on screen on a 4K output, the same as a
  `set` on a running daemon. So is an image `set` sent the moment the
  daemon starts: it waits for the outputs' `configure` (bounded by a
  round trip already sent) rather than decode once to check the file and
  again to draw it.

Outputs are tracked as they come and go: each gets one surface on the
`background` layer (namespace `wallpaper`), covering the whole output,
reserving no space and taking no input. With no outputs at all the daemon
simply waits. `scootbg query` answers, on one line (wrapped here):

```text
{"type":"outputs","outputs":[{"name":"DP-1","description":"...",
 "mode":{"width":3840,"height":2160},"scale":2,"transform":"normal",
 "logical":{"width":2560,"height":1440},
 "surface":{"state":"configured","size":{"width":2560,"height":1440},
            "scale":1.5,"pixels":{"width":3840,"height":2160}},
 "draw_failed":false,"shows":{"color":"#1e1e2e"}}],"saving":true,
 "profile":"default"}
```

Every key is always present, `null` when not known yet. `mode` is in
device pixels. `scale` is `wl_output`'s integer scale (a fractional one
rounded up), and `transform` is `wl_output`'s, counter-clockwise.
`logical` is the output's size in logical pixels: exact once the surface is
configured, before that worked out from the mode and the best scale known
(within a pixel). `surface.scale` is the scale an image, or a color on the
full-size fallback, is drawn at on that surface (`1.5` from
`wp_fractional_scale_v1`, else an integer; a color on a single-pixel or
1×1 buffer is always drawn at 1) and `surface.pixels` the size in device
pixels an image is drawn at, both `null` until the surface is
configured. `surface.state` is `waiting` (the
output has not reported itself yet), `pending`, `configured` (with `size`),
`closed` (the compositor closed it; scootbg makes it again, once) or
`gave-up` (closed a second time over the output's life: scootbg has
stopped trying on that output, says so on stderr, and the other outputs
are unaffected; it lasts until that output is unplugged and plugged back
in, which makes it a new output). `shows` is what the output has on
screen: `{"color":"#rrggbb"}` (lowercase),
`{"image":"/abs/path","mode":"fill","fill":"#rrggbb","filter":"lanczos3"}`,
or `null` for nothing. `draw_failed` is `true` when the last attempt to
draw what the output should show failed (an image that exists but cannot
be decoded, a buffer too large; stderr says why), which tells that `null`
apart from a `clear`; the next request for the output, or a new size,
retries. `saving` (after the list) is `false` while `set` and `clear` are
not saved for the next start (see above), and `profile` is the profile
whose state is restored and saved. New keys
may be added; none changes meaning within protocol 1. If the daemon cannot
accept clients (out of file descriptors, say), it keeps the wallpaper up,
says so once on stderr, and retries every second rather than exit. The
control protocol itself (one JSON object per line, for scripts that skip
the CLI) is in [docs/scootbg/README.md](docs/scootbg/README.md#the-control-protocol).
scoot's `[wallpaper]` section, which runs `apply-config`, is in
[docs/configuration.md](docs/configuration.md#wallpaper).

## Documentation

- [docs/ipc.md](docs/ipc.md) — driving scoot from a script or an agent:
  requests, actions, what the socket refuses, the rules that bite.
- [docs/protocols.md](docs/protocols.md) — porting a bar, launcher, locker or
  shell: every protocol, and what to know before writing against it.
- [docs/configuration.md](docs/configuration.md) — every flag, table, field
  and keybinding.
- [docs/nix.md](docs/nix.md) — consuming the flake from your own
  configuration: the home-manager and NixOS modules, platform notes, and
  the live-defaults reference.
- [docs/tty.md](docs/tty.md) — real hardware: DRM device selection, Asahi
  Linux, hotplug, modes, renderers.
- [docs/benchmarks.md](docs/benchmarks.md) — measured resource usage (CPU,
  wakeups, memory, startup, screenshots), including an A/B with niri on the
  dev VM, with how it was measured and what it cannot show.
- [docs/scootbg/README.md](docs/scootbg/README.md) — scootbg, the
  wallpaper daemon (early: colors and images per output, restored at
  start-up, set from scoot's `[wallpaper]` section), with its design and
  its own backlog.
- [CHANGELOG.md](CHANGELOG.md) · [ROADMAP.md](ROADMAP.md)

## Developing

```sh
nix develop                     # every dependency, on Linux or macOS
cargo test --workspace          # the compositor only compiles on Linux
cargo nextest run --workspace   # one process per test -- the required runner
```

On Linux the shell also carries what `scripts/smoke-test.sh` drives (`foot`,
`jq`, ImageMagick, `wayland-info`), sets `$XDG_RUNTIME_DIR` when the box has
none (a container, a CI runner), sets `LANG=C.UTF-8` when no locale is set at
all (`LC_ALL`, `LC_CTYPE` and `LANG` all empty; an explicit one, `C`
included, is kept), and provides `soft-egl`, which runs one command against
Mesa's software EGL the way CI runs the GLES tests:

```sh
soft-egl cargo nextest run --workspace   # without it the GLES tests fail on a GPU-less box
scripts/smoke-test.sh                    # without soft-egl: don't hand it Mesa
```

`devenv shell` gives a shell with the same contents (both read
`nix/dev-shell.nix`), and
enters faster once warm because devenv caches its evaluation.
`scripts/devenv-bootstrap.sh` installs single-user Nix and devenv on a
disposable Linux box that has neither (a container, a Claude Code on the web
session); on a machine that already has Nix, install devenv the usual way.

`crates/scoot-core` is the platform-independent layout engine (no Wayland, no
I/O), `crates/scoot-ipc` the wire protocol and a client over it,
`crates/scootctl` the `scootctl` remote-control client, `crates/scoot`
the CLI and the Smithay-based compositor, `crates/scootbg` the wallpaper
daemon and `crates/scootbg-mem` the only `unsafe` code it has. `vm/README.md` sets up a Mac-native
NixOS VM to run the Linux-only half in; `CLAUDE.md` has the engineering
standards.

Every pull request runs `.github/workflows/ci.yml`, which does the above
plus `cargo fmt`, `cargo clippy -D warnings`, `scripts/smoke-test.sh` under
`--headless`, an `ldd` check that the default build links no GPU stack
(`libgbm`/`libdrm` are the live assertions; `libEGL`/`libGLESv2` are belt-and-braces,
since both are `dlopen`ed and never appear in `ldd` either way), a `nix fmt`
check over all tracked `.nix` files, `nix flake check -L` (Linux and macOS
jobs each cover their own systems' outputs, modules and checks), and a macOS
`cargo check --workspace --all-targets` (on a Mac that is the `scootctl`
client plus the compositor crate with its Linux halves cfg'd out). Jobs are
split by path: a change under `crates/scootbg/` or `crates/scootbg-mem/`
alone runs only scootbg's own job (fmt, clippy, tests, a no-`libc`-crate
check, the release size) plus scootbg's integration tests (on a headless
scoot, and on a headless sway from the pinned nixpkgs for outputs coming
and going, with colors and images checked by real pixels on both,
fractional scales included), and a compositor-only change skips scootbg's job; shared files
(`Cargo.*`, `flake.*`, `nix/`, `.github/`, and anything unlisted) run
everything. The
packaged artifacts themselves (`nix build .#scoot .#scootctl .#scootbg`) build on every
merge to main via `.github/workflows/nix-build.yml` instead of on every PR --
a full release Smithay build the cargo cache cannot reuse, so it would tax
every push; the every-PR `flake check` already evals every output and runs
the module suite. It runs the
build and test steps through `nix develop` (the smoke test's own tools come
via `nix shell` pinned to the same lockfile), so the flake stays the only
dependency list. **A green check
is not full coverage**: a GitHub runner has no seat, no VT, no `/dev/dri`
and no GPU, so `--tty`, `--nested`, every GPU path and all performance work
stay manual on the dev VM — the workflow's header says so in full.

## License

MIT. See `NOTICE` for third-party attribution (this compositor is built on
[Smithay](https://github.com/Smithay/smithay)).
