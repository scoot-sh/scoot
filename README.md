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
  session on a KMS display (one monitor for now), including VMs and machines
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
  coordinates `pointer click` takes. scoot's own end-to-end test drives it
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

- **Multi-output is partial.** `--headless --outputs N` now creates several virtual outputs for testing,
  each with its own geometry, its own scrolling strip and its own composited
  framebuffer (screenshots, screen captures, gamma and frame callbacks all
  work per output) — and workspace groups, output-management heads and the
  pointer clamp are per-output too — but new windows still open on the first
  output (moving one across, and focusing another output from the keyboard,
  is a config bind away — no defaults ship for either yet), and `--tty`
  still drives one connector (a second monitor there stays dark).
- **XWayland is opt-in, and partial.** X11 applications run with
  `--xwayland` (or `[xwayland] enabled`) in an `xwayland` build (`cargo
  build --release --features xwayland`, with `Xwayland` on `PATH`; no flake
  output ships it yet): their windows tile, dialogs float, fullscreen
  works, and they take focus by themselves only when nothing is focused,
  when they belong to the X app in use, or when scoot started them.
  Copy and paste works between X and Wayland apps both ways (clipboard
  and middle-click primary; `xclip`/`xsel` and `wl-copy`/`wl-paste` see
  each other), but an X app reads or sets it only while an X window has
  the keyboard. Dragging from an X app into a Wayland app works; dropping
  *onto* an X window does not land (from a Wayland app, from another X app,
  or within the same X app — nothing is lost, the drop just does nothing).
  Running one extends full trust to it — an X11 client can read and cover
  other windows by design ([protocols.md](docs/protocols.md#clipboard-drag-and-drop-and-input-methods)).
  Not there yet: drops onto X windows, and X input methods (XIM).
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
`--nested` and `--tty` warn and ignore the flag, having one host window and
one CRTC respectively. See
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
| `Super+comma` / `Super+period` | Focus output 1 / 2 |
| `Super+Shift+comma` / `Super+Shift+period` | Move window to output 1 / 2 directly |
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
| Run X11 applications | `--xwayland` / `[xwayland] enabled` (opt-in; X clients are fully trusted by design); clipboard and primary selection cross both ways while an X window is focused; drags from X into Wayland apps work, drops onto X windows do not; no XIM | [protocols.md](docs/protocols.md#xwayland-opt-in) |

The full protocol/version table, and the ones that are deliberately absent,
are at the top of [docs/protocols.md](docs/protocols.md).

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
- [CHANGELOG.md](CHANGELOG.md) · [ROADMAP.md](ROADMAP.md)

## Developing

```sh
nix develop                     # every dependency, on Linux or macOS
cargo test --workspace          # the compositor only compiles on Linux
cargo nextest run --workspace   # one process per test -- the required runner
```

`crates/scoot-core` is the platform-independent layout engine (no Wayland, no
I/O), `crates/scoot-ipc` the wire protocol and a client over it,
`crates/scootctl` the `scootctl` remote-control client, `crates/scoot`
the CLI and the Smithay-based compositor. `vm/README.md` sets up a Mac-native
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
client plus the compositor crate with its Linux halves cfg'd out). The
packaged artifacts themselves (`nix build .#scoot .#scootctl`) build on every
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
