# scoot

A scrolling-tiling Wayland compositor, in the shape of
[niri](https://github.com/YaLTeR/niri): lightweight, fast, GPU-optional, and
built to be driven by a script or an agent as easily as by a keyboard.

![Three scoot columns: htop, vim, and a file tree stacked above a shell
querying the compositor over IPC](docs/assets/screenshot.png)

Two things distinguish it from a typical compositor:

- **It runs with no GPU.** Rendering goes through
  [pixman](http://pixman.org/) on the CPU, so it works headless and works in
  a GPU-less container (the target is running inside
  [webtop](https://github.com/linuxserver/docker-webtop)). A GLES renderer is
  available opt-in (`--renderer gles`, `--headless`/`--nested` only) but
  pixman stays the default — see
  [docs/tty.md](docs/tty.md#which-renderer-draws-the-frames) for what that
  does and does not buy today.
- **It's IPC-first.** Every action a keybind would trigger — focus, move,
  resize, spawn, close — and every input a user could give — key presses,
  pointer movement, clicks — is also a request on a Unix socket, alongside
  screenshots and window/output introspection. The compositor itself is
  driven the same way in its own end-to-end test
  (`scripts/smoke-test.sh`). The intent is that an agent doing simple
  computer-use tasks in a VM is a first-class client, not an afterthought
  bolted on later. Because that socket can inject any keystroke, it is
  treated as a privileged channel: it lives in `$XDG_RUNTIME_DIR` (override
  with `$SCOOT_SOCKET`), is created `0600`, and serves only connections from
  the same user as the compositor.

**Already running niri?** Those two bullets are the whole reason to switch.
niri is more complete; if you need neither a compositor that runs with no GPU
at all nor one you can drive — synthetic key, pointer and text input included
— from a script over a socket, stay where you are.

It is real enough to use: **confirmed working on Apple Silicon under Asahi
Linux, and daily-driven on `--tty`** (2026-09-18).

## Not yet

- **One output.** Plug in a second monitor and it stays dark.
- **No XWayland.** X11-only applications do not run.
- **No GPU scanout.** The GLES renderer above reads every frame back to main
  memory exactly as pixman does, so it buys correctness parity, not speed.
  Scanning a GPU buffer out under `--tty` is the milestone in flight.
- **No config reload.** Settings are read once at startup.
- **No macOS adapter.** `scoot-core` is kept platform-independent so one can
  exist, but nothing drives the Accessibility API yet. On macOS you get the
  `scoot msg` client only.

## Install

Clone, then, from the flake at the repo root:

```sh
nix build                            # ./result/bin/scoot
nix run . -- --headless -- foot      # build and run it in one step
nix run . -- msg windows             # the client, same binary
```

On Linux that builds the compositor. On macOS it is compiled out and you get
`scoot msg` alone — enough to drive a compositor running in a VM. The flake
covers Apple Silicon only; an Intel Mac builds the same client with `cargo
build`.

## Running

```sh
scoot --headless --width 1280 --height 800 -- foot   # start, spawn a terminal
scoot --nested --width 1280 --height 800 -- foot     # inside your compositor
scoot --tty -- foot                                  # on a real DRM/KMS seat

scoot msg windows                                    # in another shell
scoot msg action focus-column left
scoot msg screenshot --out /tmp/shot.png
scoot msg type "hello"
```

`--tty` needs a seat (`seatd` or logind) with a DRM device on it; everything
about device choice, hotplug and modes is in [docs/tty.md](docs/tty.md). Run
`scoot --help` for every flag, request and action.

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
| `Super+r` | Cycle column width |
| `Super+q` | Close focused window |
| `Super+Return` | Spawn `foot` |
| `Super+Shift+e` | Quit |
| `Ctrl+Alt+F1`..`F12` | Switch VT (`--tty` only) |

## Configuring

Drop a TOML file at `~/.config/scoot/config.toml`, or pass `--config PATH`.
Everything in it is optional:

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
```

Every table, field, default and failure mode is in
[docs/configuration.md](docs/configuration.md). Almost nothing in it can stop
scoot starting — a bad value logs a warning and falls back to the default.
The two exceptions are deliberate, because guessing would be worse than
refusing: `[tty] gpu` naming a device that will not open, and
`[renderer] backend = "gles"` on a box with no working EGL.

## What works

| If you want to… | scoot has | Detail |
| --- | --- | --- |
| Run a bar, dock, wallpaper, launcher or notification daemon | `wlr-layer-shell-v1`, with keyboard focus | [protocols.md](docs/protocols.md#layer-shell-bars-wallpapers-launchers) |
| List and switch workspaces from a bar | `ext-workspace-v1` | [protocols.md](docs/protocols.md#workspaces-ext-workspace-v1) |
| List, focus and close windows from a taskbar | `ext-foreign-toplevel-list-v1` **and** the wlr one | [protocols.md](docs/protocols.md#window-lists-two-protocols) |
| Lock the screen | `ext-session-lock-v1`, compositor-enforced | [protocols.md](docs/protocols.md#screen-locking-ext-session-lock-v1) |
| Auto-lock or dim on idle | `ext-idle-notify-v1` + `idle-inhibit-v1` (`swayidle`) | [protocols.md](docs/protocols.md#idle-detection) |
| Screenshot or screen-share | `ext-image-copy-capture-v1` (`grim`), plus `scoot msg screenshot` | [protocols.md](docs/protocols.md#screen-capture-ext-image-copy-capture-v1) |
| Clipboard manager, middle-click paste | `wlr-`/`ext-data-control`, `primary-selection-v1` | [protocols.md](docs/protocols.md#clipboard-and-primary-selection) |
| Night light | `wlr-gamma-control-v1` (`wlsunset`, `gammastep`) | [protocols.md](docs/protocols.md#night-light-wlr-gamma-control-v1) |
| A HiDPI display | `[output] scale`, integer and fractional | [protocols.md](docs/protocols.md#output-scaling) |
| An IME or on-screen keyboard | `text-input-v3` + `input-method-v2` | [protocols.md](docs/protocols.md#input-methods-text-input-v3-input-method-v2) |
| A drawing tablet | `tablet-v2` — tools yes, pads no | [protocols.md](docs/protocols.md#drawing-tablets-tablet-v2) |
| Read display modes (`wlr-randr`, Settings → Display) | `wlr-output-management-v1`, **read-only** | [protocols.md](docs/protocols.md#display-information-wlr-output-management-v1) |
| Drive the session from a script or an agent | the control socket: input injection, screenshots, introspection | [ipc.md](docs/ipc.md) |
| Run X11 applications | nothing — there is no XWayland | — |

The full protocol/version table, and the ones that are deliberately absent,
are at the top of [docs/protocols.md](docs/protocols.md).

## Documentation

- [docs/ipc.md](docs/ipc.md) — driving scoot from a script or an agent:
  requests, actions, what the socket refuses, the rules that bite.
- [docs/protocols.md](docs/protocols.md) — porting a bar, launcher, locker or
  shell: every protocol, and what to know before writing against it.
- [docs/configuration.md](docs/configuration.md) — every flag, table, field
  and keybinding.
- [docs/tty.md](docs/tty.md) — real hardware: DRM device selection, Asahi
  Linux, hotplug, modes, renderers.
- [CHANGELOG.md](CHANGELOG.md) · [ROADMAP.md](ROADMAP.md)

## Developing

```sh
nix develop                     # every dependency, on Linux or macOS
cargo test --workspace          # the compositor only compiles on Linux
cargo nextest run --workspace   # one process per test; runs no doctests, so
                                # it is an addition, not a replacement
```

`crates/scoot-core` is the platform-independent layout engine (no Wayland, no
I/O), `crates/scoot-ipc` the wire protocol and a client over it, `crates/scoot`
the CLI and the Smithay-based compositor. `vm/README.md` sets up a Mac-native
NixOS VM to run the Linux-only half in; `CLAUDE.md` has the engineering
standards.

## License

MIT. See `NOTICE` for third-party attribution (this compositor is built on
[Smithay](https://github.com/Smithay/smithay)).
