![scoot — a cat swiping three terminal windows sideways across the screen,
trailing motion lines](docs/assets/logo.png)

# scoot

[![CI](https://github.com/scoot-sh/scoot/actions/workflows/ci.yml/badge.svg)](https://github.com/scoot-sh/scoot/actions/workflows/ci.yml)

scoot is a Wayland compositor where windows sit side by side in columns on
a strip that scrolls sideways. Opening a window never squeezes the others;
the strip just gets longer. The layout comes from
[niri](https://github.com/niri-wm/niri).

![Three scoot columns: htop, vim, and a file tree stacked above a shell
querying the compositor over IPC](docs/assets/screenshot.png)

## Why scoot

- **It doesn't need a GPU.** scoot draws on the CPU by default, so it runs
  anywhere: real hardware, a VM with no 3D acceleration, a container, or
  inside another desktop. Got a GPU? There's an optional tier that uses it.
- **Scripts and AI agents can drive it.** One socket gives you everything
  computer use needs: typing, clicking, screenshots, window positions, and
  "wait until the screen settles." scoot's own tests drive it this way.
- **It stays out of the way.** An idle scoot doesn't wake up at all, and
  it used less memory than niri in [our benchmarks](docs/benchmarks.md).

If you want the most mature scrolling compositor, use
[niri](https://github.com/niri-wm/niri). It's great, and it inspired this
project.

## Get started

### Install

scoot runs on Linux. With [Nix](https://nixos.org):

```sh
nix profile add github:scoot-sh/scoot#scoot github:scoot-sh/scoot#scootctl
```

(Older Nix calls it `nix profile install`.)

From source on Debian or Ubuntu (Rust 1.87 or newer):

```sh
sudo apt install pkg-config libwayland-dev libxkbcommon-dev libinput-dev \
  libdrm-dev libdisplay-info-dev libseat-dev libudev-dev libpixman-1-dev \
  libgbm-dev libegl-dev libdbus-1-dev
git clone https://github.com/scoot-sh/scoot && cd scoot
cargo build --release -p scoot -p scootctl    # binaries land in target/release/
```

The examples below open [foot](https://codeberg.org/dnkl/foot), scoot's
default terminal, so install that too (or name another one). NixOS and
home-manager modules are in [docs/nix.md](docs/nix.md).

### Try it

The safest first run is in a window inside the Wayland desktop you already
use (GNOME, KDE Plasma, sway, …):

```sh
scoot --nested -- foot
```

That opens scoot with a terminal in it. Close the window to quit. If your
desktop keeps some `Super` shortcuts for itself, you can
[rebind scoot's](docs/configuration.md#binds).

When you want it as your whole session, switch to a text console
(`Ctrl+Alt+F3`), log in, and run:

```sh
scoot --tty -- foot
```

`Super+Shift+e` quits, and `Ctrl+Alt+F1`..`F12` switches back to your
usual desktop at any time. Hardware setup, multiple monitors and the GPU
tier are in [docs/tty.md](docs/tty.md).

### Drive it from a script

While scoot is running, from any terminal:

```sh
scootctl windows                          # list windows, as JSON
scootctl action focus-column left
scootctl type "hello"
scootctl screenshot --out shot.png
```

For agents and tests, `scoot --headless -- foot` runs with no screen at
all. Everything the socket can do is in [docs/ipc.md](docs/ipc.md).

### Keys

`Super` is the Windows/Command key. Directions are vim's `h` `j` `k` `l`.

| Keys | Does |
|---|---|
| `Super+Return` | Open a terminal (`foot`) |
| `Super+h` / `Super+l` | Focus the column left / right |
| `Super+j` / `Super+k` | Focus the window below / above |
| `Super+Shift` + direction | Move the column or window |
| `Super+1`..`9` | Go to workspace 1–9 |
| `Super+Shift+1`..`9` | Send the window to workspace 1–9 |
| `Super+r` | Cycle the column's width |
| `Super+f` | Fullscreen |
| `Super+Shift+Space` | Float the window, or put it back |
| `Super+q` | Close the window |
| `Super+Shift+e` | Quit scoot |

All the default keys, and how to change them, are in
[docs/configuration.md](docs/configuration.md#default-keybindings).

### Configure

scoot reads `~/.config/scoot/config.toml`. Everything is optional. To start
from the defaults, with comments:

```sh
scoot --print-default-config --write
```

A small example:

```toml
[layout]
gap = 8

[appearance]
focus_ring_active_color = "#ffaa00"

[binds]
"super+t" = "spawn foot"

[autostart]
commands = ["spawn waybar"]

[wallpaper]
image = "~/Pictures/hills.jpg"
```

`scootctl reload` applies changes without restarting. A mistake never
stops scoot from starting: it logs the problem and uses the default. Every
option is in [docs/configuration.md](docs/configuration.md).

## What works

scoot speaks the standard Wayland protocols, so the usual tools work.

| You want to… | |
| --- | --- |
| Run a bar, dock, launcher or notification daemon | [yes](docs/protocols.md#layer-shell-bars-wallpapers-launchers) |
| Switch workspaces or windows from a bar or taskbar | [yes](docs/protocols.md#workspaces-ext-workspace-v1) |
| Have dialogs and pop-ups float | [yes, automatically](docs/configuration.md#floating) |
| Watch video or play games fullscreen | [yes](docs/protocols.md#fullscreen) |
| Lock the screen, or lock and dim when idle | [yes](docs/protocols.md#screen-locking-ext-session-lock-v1) |
| Take screenshots or share the screen | [yes](docs/protocols.md#screen-capture-ext-image-copy-capture-v1) |
| Run GPU apps, even with no GPU on the compositor | [yes](docs/protocols.md#gpu-rendering-clients-zwp_linux_dmabuf_v1) |
| Use a clipboard manager or middle-click paste | [yes](docs/protocols.md#clipboard-and-primary-selection) |
| Use a night light | [yes](docs/protocols.md#night-light-wlr-gamma-control-v1) |
| Use a HiDPI screen, fractional scaling included | [yes](docs/protocols.md#output-scaling) |
| Use an input method or on-screen keyboard | [yes](docs/protocols.md#input-methods-text-input-v3-input-method-v2) |
| Use a drawing tablet | [pens, not pads](docs/protocols.md#drawing-tablets-tablet-v2) |
| Change display modes from `wlr-randr` or Settings | [not yet: read-only](docs/protocols.md#display-information-wlr-output-management-v1) |
| Run X11 apps | [yes, with `--xwayland`](docs/protocols.md#xwayland-opt-in) in an XWayland build ([`nix build .#scoot-xwayland`](docs/nix.md#xwayland-from-the-flake)) |
| Drive it from a script or an agent | [yes](docs/ipc.md) |

Every protocol and version is listed in [docs/protocols.md](docs/protocols.md).

## Not yet

- **Per-monitor settings.** Multiple monitors work, but they share one
  scale and resolution and line up left to right.
- **Some X11 extras.** X input methods (XIM) and X window icons.
- **A macOS version.** On a Mac you can build `scootctl`, to drive scoot
  in a Linux VM.

## scootbg

scoot comes with `scootbg`, a small wallpaper daemon. It shows a color or
an image on each monitor, and uses no CPU once it's up. The `[wallpaper]`
section above runs it for you. It also works on sway, niri, Hyprland and
other compositors with `wlr-layer-shell`:

```sh
scootbg daemon &
scootbg set ~/Pictures/hills.jpg
```

It's early. More in [docs/scootbg/README.md](docs/scootbg/README.md).

## Documentation

| Doc | For |
| --- | --- |
| [configuration.md](docs/configuration.md) | every flag, config option and key |
| [ipc.md](docs/ipc.md) | driving scoot from a script or an agent |
| [protocols.md](docs/protocols.md) | writing a bar, launcher or other tool for scoot |
| [tty.md](docs/tty.md) | real hardware: devices, monitors, the GPU tier |
| [nix.md](docs/nix.md) | the flake and the NixOS and home-manager modules |
| [benchmarks.md](docs/benchmarks.md) | measured CPU and memory, next to niri |
| [scootbg/](docs/scootbg/README.md) | the wallpaper daemon |
| [scootbar/](docs/scootbar/README.md) | the status bar (early: a solid bar so far) |
| [development.md](docs/development.md) | building, testing and contributing |

What changed is in [CHANGELOG.md](CHANGELOG.md), and what's next in
[ROADMAP.md](ROADMAP.md).

## License

MIT. See `NOTICE` for third-party attribution (this compositor is built on
[Smithay](https://github.com/Smithay/smithay)).
