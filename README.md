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

- **Multi-output is partial.** `--headless --outputs N` now creates several virtual outputs for testing,
  each with its own geometry, its own scrolling strip and its own composited
  framebuffer (screenshots, screen captures, gamma and frame callbacks all
  work per output) — and workspace groups, output-management heads and the
  pointer clamp are per-output too — but new windows still open on the first
  output (moving one across, and focusing another output from the keyboard,
  is a config bind away — no defaults ship for either yet), and `--tty`
  still drives one connector (a second monitor there stays dark).
- **No XWayland.** X11-only applications do not run.
- **GPU scanout is new and narrow.** `--tty --renderer gles` scans out from
  the GPU, but only in a `gpu-scanout` build (`nix build .#scoot-gpu`).
  The cursor rides its own KMS plane where the CRTC exposes one (and an
  overlay plane where that is all there is), overlay planes are enumerated
  per CRTC, and frames may go direct on the primary (`ALLOW_SCANOUT`) — with
  the capture fix that makes the last one safe: a direct frame marks the
  capture recording, and a capture served off a marked recording forces one
  composite frame first, so screenshots and screen captures stay correct.
  The framebuffer exporter now admits client dma-bufs, but no window leaves
  the composited primary yet on any hardware: Smithay only hands the primary
  to a client buffer whose format and modifier match the swapchain's, and on
  this tree they never do (nothing is marked a scanout candidate for the
  overlays either). Direct scanout has been seen only on the dev VM
  (virtio-gpu) with that format check deliberately lifted in an uncommitted
  experiment — which is how the capture fix was watched working live:
  captures stayed correct through direct frames, and a capture taken while
  VT-switched away failed with a retry message instead of returning a stale
  screen. A plane-assigned cursor is absent from captures by design.
  `--headless`/`--nested` still read every frame back to main memory as
  pixman does. It is no longer unproven: on an Apple M2 under Asahi Linux it
  costs **4–5x less CPU** than the default tier under damage (and ~0.2 W
  less), draws the same pixels, and uses 7–16 MB more memory (numbers and
  method in [Asahi.md](Asahi.md), Test 4).
  pixman is still the default and still the right answer on a GPU-less box.
- **Config reload is live, except three restart fields.** `scootctl reload` (or `kill -HUP` on the
  compositor) re-applies the layout (gap, column widths, default column
  width), the output scale (except under `--nested`, where the host owns
  it), the
  appearance (including the cursor size, color and theme), the keybindings,
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

`--tty` needs a seat (`seatd` or logind) with a DRM device on it; everything
about device choice, hotplug and modes is in [docs/tty.md](docs/tty.md). Run
`scoot --help` for every flag, `scootctl --help` for every request and action.
`scoot --version` (or `scootctl --version`) identifies a build — its own
version plus the IPC protocol number — without starting anything.

`--headless --outputs N` (1–8) creates N virtual outputs side by side, so
per-output behaviour is testable with no second monitor: each gets its own
`wl_output`, its own place in the coordinate space, its own scrolling
strip, its own composited strip, its own layer-shell zones and input, its
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
- [docs/nix.md](docs/nix.md) — consuming the flake from your own
  configuration: the home-manager and NixOS modules, platform notes, and
  the live-defaults reference.
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
