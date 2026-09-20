![scoot — a cat swiping three terminal windows sideways across the screen,
trailing motion lines](docs/assets/logo.png)

# scoot

[![CI](https://github.com/scoot-sh/scoot/actions/workflows/ci.yml/badge.svg)](https://github.com/scoot-sh/scoot/actions/workflows/ci.yml)

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
  available opt-in (`--renderer gles`, and under `--tty` it scans out from
  the GPU with a `gpu-scanout` build) but
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

- **One output is composited.** Plug in a second monitor and it stays dark.
  `--headless --outputs N` now creates several virtual outputs for testing,
  each with its own geometry and its own scrolling strip, but only the first
  is drawn and `--tty` still drives one connector.
- **No XWayland.** X11-only applications do not run.
- **GPU scanout is new and narrow.** `--tty --renderer gles` scans out from
  the GPU, but only in a `gpu-scanout` build, only on the primary plane (no
  overlay or cursor planes), and it has never run on a real GPU — every
  measurement so far is a software rasteriser's. `--headless`/`--nested`
  still read every frame back to main memory as pixman does.
- **No config reload.** Settings are read once at startup.
- **No macOS adapter.** `scoot-core` is kept platform-independent so one can
  exist, but nothing drives the Accessibility API yet. On macOS you get
  `scootctl`, the remote-control client, only.

## Install

Clone, then, from the flake at the repo root:

```sh
nix build                            # ./result/bin/scoot
nix build .#scootctl                 # ./result/bin/scootctl, the client alone
nix run . -- --headless -- foot      # build and run it in one step
nix run .#scootctl -- windows        # the client, from anywhere
```

On Linux that builds the compositor. On macOS the default is `scootctl`
alone — enough to drive a compositor running in a VM. The flake
covers Apple Silicon only; an Intel Mac builds the same client with `cargo
build -p scootctl`.

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

`--tty` needs a seat (`seatd` or logind) with a DRM device on it; everything
about device choice, hotplug and modes is in [docs/tty.md](docs/tty.md). Run
`scoot --help` for every flag, `scootctl --help` for every request and action.

`--headless --outputs N` (1–8) creates N virtual outputs side by side, so
per-output behaviour is testable with no second monitor: each gets its own
`wl_output`, its own place in the coordinate space and its own scrolling
strip. One of them is composited — the first — so `scootctl screenshot
--output 2` is refused rather than answered with the first output's pixels.
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

[autostart]
commands = ["spawn waybar"]
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
| List and switch workspaces from a bar | `ext-workspace-v1` | [protocols.md](docs/protocols.md#workspaces-ext-workspace-v1) |
| List, focus and close windows from a taskbar | `ext-foreign-toplevel-list-v1` **and** the wlr one | [protocols.md](docs/protocols.md#window-lists-two-protocols) |
| Lock the screen | `ext-session-lock-v1`, compositor-enforced | [protocols.md](docs/protocols.md#screen-locking-ext-session-lock-v1) |
| Auto-lock or dim on idle | `ext-idle-notify-v1` + `idle-inhibit-v1` (`swayidle`) | [protocols.md](docs/protocols.md#idle-detection) |
| Screenshot or screen-share | `ext-image-copy-capture-v1` (`grim`), plus `scootctl screenshot` | [protocols.md](docs/protocols.md#screen-capture-ext-image-copy-capture-v1) |
| Run a GPU-rendering client with no GPU on the compositor | `zwp_linux_dmabuf_v1`, formats taken from whichever renderer is active | [protocols.md](docs/protocols.md#gpu-rendering-clients-zwp_linux_dmabuf_v1) |
| Clipboard manager, middle-click paste | `wlr-`/`ext-data-control`, `primary-selection-v1` | [protocols.md](docs/protocols.md#clipboard-and-primary-selection) |
| Night light | `wlr-gamma-control-v1` (`wlsunset`, `gammastep`) | [protocols.md](docs/protocols.md#night-light-wlr-gamma-control-v1) |
| A HiDPI display | `[output] scale`, integer and fractional | [protocols.md](docs/protocols.md#output-scaling) |
| An IME or on-screen keyboard | `text-input-v3` + `input-method-v2` | [protocols.md](docs/protocols.md#input-methods-text-input-v3-input-method-v2) |
| A drawing tablet | `tablet-v2` — tools yes, pads no | [protocols.md](docs/protocols.md#drawing-tablets-tablet-v2) |
| Read display modes (`wlr-randr`, Settings → Display) | `wlr-output-management-v1`, **read-only** | [protocols.md](docs/protocols.md#display-information-wlr-output-management-v1) |
| Drive the session from a script or an agent | the control socket: input injection, screenshots, introspection | [ipc.md](docs/ipc.md) |
| Run scoot inside another compositor (webtop, a nested test session) | `--nested`, following the host window's size as it changes | [configuration.md](docs/configuration.md#command-line-flags) |
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
(`libgbm` is the live assertion; `libEGL`/`libGLESv2` are belt-and-braces,
since both are `dlopen`ed and never appear in `ldd` either way), and a macOS
`cargo check --workspace --all-targets` (on a Mac that is the `scootctl`
client plus the compositor crate with its Linux halves cfg'd out). It runs the
build and test steps through `nix develop` (the smoke test's own tools come
via `nix shell` pinned to the same lockfile), so the flake stays the only
dependency list. **A green check
is not full coverage**: a GitHub runner has no seat, no VT, no `/dev/dri`
and no GPU, so `--tty`, `--nested`, every GPU path and all performance work
stay manual on the dev VM — the workflow's header says so in full.

## License

MIT. See `NOTICE` for third-party attribution (this compositor is built on
[Smithay](https://github.com/Smithay/smithay)).
