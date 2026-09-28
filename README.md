![scoot — a cat swiping three terminal windows sideways across the screen,
trailing motion lines](docs/assets/logo.png)

# scoot

[![CI](https://github.com/scoot-sh/scoot/actions/workflows/ci.yml/badge.svg)](https://github.com/scoot-sh/scoot/actions/workflows/ci.yml)

A scrolling-tiling Wayland compositor: windows sit in columns on a strip
that scrolls sideways, a layout it owes to
[niri](https://github.com/niri-wm/niri).

![Three scoot columns: htop, vim, and a file tree stacked above a shell
querying the compositor over IPC](docs/assets/screenshot.png)

scoot was built for two things:

- **Running without a GPU.** It renders on the CPU by default, so it
  runs on real hardware, in VMs with no 3D acceleration, headless, or
  nested inside another compositor or a GPU-less container. A GPU is
  optional, and an opt-in tier uses it when you have one:
  [docs/tty.md](docs/tty.md#which-renderer-draws-the-frames).
- **Being driven by an agent.** One socket gives a script or an AI
  everything computer use needs: keys, typed text, pointer clicks,
  screenshots, "wait until the screen settles," and window positions in
  the same coordinates clicks use. scoot's own end-to-end tests drive it
  this way. Because the socket can type anything, it's private to your
  user. Reference: [docs/ipc.md](docs/ipc.md).

If scoot doesn't fit what you need, you should absolutely check out
[niri](https://github.com/niri-wm/niri). It's awesome, and it's what
inspired this project.

## Not yet

- **Per-output configuration.** One scale and mode apply to every
  monitor, screens line up left to right in connector order, and
  `wlr-output-management` is read-only. Multi-monitor itself works:
  [docs/tty.md](docs/tty.md#more-than-one-monitor).
- **X input methods (XIM).** XWayland is opt-in (`--xwayland`) and
  otherwise works, clipboard and drag-and-drop included.
- **Direct scanout beyond fullscreen.** On the opt-in GPU tier, only a
  fullscreen window can skip compositing. Under `--headless`, `gles` is
  slower than pixman.
- **Explicit sync on NVIDIA.** It's supported but only verified with Mesa.
- **Live reload of three fields.** `[tty] gpu`, `[renderer] backend` and
  `[xwayland] enabled` need a restart.
- **A macOS adapter.** On macOS you get `scootctl` only.

## Install

With Nix, from a clone:

```sh
nix build                    # ./result/bin/scoot
nix build .#scoot-gpu        # ...with the optional GPU tier for --tty
nix build .#scootctl         # the client alone (the default on macOS)
nix build .#scootbg          # the wallpaper daemon
nix run . -- --headless -- foot
```

Or from your own flake:

```nix
inputs.scoot.url = "github:scoot-sh/scoot";
# then, in a module:
environment.systemPackages = [ inputs.scoot.packages.${pkgs.system}.scoot ];
```

The compositor is Linux-only. On a Mac you get `scootctl`, enough to drive
scoot in a VM. The home-manager and NixOS modules, and platform notes, are
in [docs/nix.md](docs/nix.md).

## Running

```sh
scoot --headless -- foot              # no screen at all: for agents and tests
scoot --headless --outputs 2 -- foot  # two virtual screens, side by side
scoot --nested -- foot                # as a window inside your current desktop
scoot --tty -- foot                   # on real hardware, from a text console

scootctl windows                      # then, from another shell
scootctl action focus-column left
scootctl screenshot --out /tmp/shot.png
scootctl type "hello"
```

Trying it on your own machine? Start with `--nested`: it runs inside your
current desktop and closes like any window. `--tty` takes over a virtual
terminal, and `Ctrl+Alt+F1`..`F12` switches back to whatever you normally
run. It needs a seat (`seatd` or logind); device choice, monitors and
renderers are in [docs/tty.md](docs/tty.md).

`scoot --help` and `scootctl --help` list every flag, request and action;
[docs/configuration.md](docs/configuration.md#command-line-flags) explains
them.

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

scoot reads `~/.config/scoot/config.toml` (or `--config PATH`). Every
setting is optional, and `scoot --print-default-config --write` places a
commented starting file there:

```toml
[layout]
gap = 8
column_widths = [0.25, 0.5, 0.75, 1.0]

[appearance]
focus_ring_active_color = "#ffaa00"

[binds]
"super+t" = "spawn foot"

[autostart]
commands = ["spawn waybar"]

[wallpaper]                      # drawn by scootbg, below
image = "~/Pictures/hills.jpg"

[[window_rule]]                  # float this app instead of giving it a column
match_app_id = "*pavucontrol"
float = true
```

`scootctl reload` applies changes live. A mistake never stops scoot
starting: it logs the problem and uses the default. Every option is in
[docs/configuration.md](docs/configuration.md).

## What works

| If you want to… | scoot has | Detail |
| --- | --- | --- |
| Run a bar, dock, launcher or notification daemon | `wlr-layer-shell-v1` | [protocols.md](docs/protocols.md#layer-shell-bars-wallpapers-launchers) |
| Keep dialogs and pop-ups out of the strip | automatic floating, plus `[[window_rule]]` | [configuration.md](docs/configuration.md#floating) |
| Watch a video or play a game fullscreen | fullscreen, edge to edge | [protocols.md](docs/protocols.md#fullscreen) |
| List and switch workspaces from a bar | `ext-workspace-v1` | [protocols.md](docs/protocols.md#workspaces-ext-workspace-v1) |
| List, focus and close windows from a taskbar | `ext-foreign-toplevel-list-v1` and the wlr one | [protocols.md](docs/protocols.md#window-lists-two-protocols) |
| Lock the screen | `ext-session-lock-v1` | [protocols.md](docs/protocols.md#screen-locking-ext-session-lock-v1) |
| Lock or dim when idle | `ext-idle-notify-v1`, `idle-inhibit-v1` | [protocols.md](docs/protocols.md#idle-detection) |
| Take screenshots or share the screen | `ext-image-copy-capture-v1` (`grim`) | [protocols.md](docs/protocols.md#screen-capture-ext-image-copy-capture-v1) |
| Run GPU-rendering apps, with or without a GPU | `zwp_linux_dmabuf_v1` | [protocols.md](docs/protocols.md#gpu-rendering-clients-zwp_linux_dmabuf_v1) |
| Run apps that use explicit sync (NVIDIA, Vulkan) | `linux-drm-syncobj-v1`, GPU tier only | [protocols.md](docs/protocols.md#explicit-sync-linux-drm-syncobj-v1) |
| Use a clipboard manager, middle-click paste | `wlr-`/`ext-data-control`, `primary-selection-v1` | [protocols.md](docs/protocols.md#clipboard-and-primary-selection) |
| Use a night light | `wlr-gamma-control-v1` | [protocols.md](docs/protocols.md#night-light-wlr-gamma-control-v1) |
| Use a HiDPI display | integer and fractional scaling | [protocols.md](docs/protocols.md#output-scaling) |
| Use an IME or on-screen keyboard | `text-input-v3`, `input-method-v2` | [protocols.md](docs/protocols.md#input-methods-text-input-v3-input-method-v2) |
| Use a drawing tablet | `tablet-v2` (pens, not pads) | [protocols.md](docs/protocols.md#drawing-tablets-tablet-v2) |
| See display modes in `wlr-randr` | `wlr-output-management-v1`, read-only | [protocols.md](docs/protocols.md#display-information-wlr-output-management-v1) |
| Run X11 apps | XWayland, opt-in with `--xwayland` | [protocols.md](docs/protocols.md#xwayland-opt-in) |
| Drive the session from a script or an agent | the control socket | [ipc.md](docs/ipc.md) |

The full protocol list, with versions, is at the top of
[docs/protocols.md](docs/protocols.md).

## scootbg (early)

`scootbg` is scoot's wallpaper daemon: a solid color or an image (PNG,
JPEG, WebP) on each output, sharp at fractional scales, restored at the
next login. It aims to be the lightest wallpaper daemon there is: once the
wallpaper is up it uses no CPU at all. It is a separate program and works
on any compositor with `wlr-layer-shell` (sway, niri, Hyprland, river).

In scoot, the `[wallpaper]` section above is all it takes. Anywhere else,
or to change it live:

```sh
scootbg daemon                          # start it
scootbg set ~/Pictures/hills.jpg        # an image on every output
scootbg set '#1e1e2e' --output DP-2     # a color on one output
scootbg query                           # what each output shows, as JSON
```

Every command and option is in
[docs/scootbg/cli.md](docs/scootbg/cli.md); the design and measurements are
in [docs/scootbg/README.md](docs/scootbg/README.md).

## Documentation

| Doc | For |
| --- | --- |
| [configuration.md](docs/configuration.md) | every flag, config option and keybinding |
| [ipc.md](docs/ipc.md) | driving scoot from a script or an agent |
| [protocols.md](docs/protocols.md) | writing or porting a bar, launcher, locker or shell |
| [tty.md](docs/tty.md) | real hardware: devices, monitors, hotplug, renderers |
| [nix.md](docs/nix.md) | the flake, and the home-manager and NixOS modules |
| [benchmarks.md](docs/benchmarks.md) | measured CPU, memory and startup, with a niri comparison |
| [scootbg/](docs/scootbg/README.md) | the wallpaper daemon |
| [development.md](docs/development.md) | building, testing and CI |
| [CHANGELOG.md](CHANGELOG.md) · [ROADMAP.md](ROADMAP.md) | what changed, what is next |

## Developing

```sh
nix develop                     # or `devenv shell`: every dependency
cargo nextest run --workspace   # the test runner
scripts/smoke-test.sh           # end to end, driven over IPC
```

The dev shell, the crates, and what CI does and cannot check are in
[docs/development.md](docs/development.md).

## License

MIT. See `NOTICE` for third-party attribution (this compositor is built on
[Smithay](https://github.com/Smithay/smithay)).
