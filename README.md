# flexwm

A scrolling-tiling Wayland compositor, in the shape of [niri](https://github.com/YaLTeR/niri):
lightweight, fast, GPU-optional, and built to be driven by a script or an agent
as easily as by a keyboard.

Two things distinguish it from a typical compositor:

- **It runs with no GPU.** Rendering goes through [pixman](http://pixman.org/)
  on the CPU, so it works headless and works in a GPU-less container (the
  target is running inside [webtop](https://github.com/linuxserver/docker-webtop)).
- **It's IPC-first.** Every action a keybind would trigger — focus, move,
  resize, spawn, close — and every input a user could give — key presses,
  pointer movement, clicks — is also a request on a Unix socket, alongside
  screenshots and window/output introspection. The compositor itself is
  driven the same way in its own end-to-end test (see `scripts/smoke-test.sh`).
  The intent is that an agent doing simple computer-use tasks in a VM is a
  first-class client, not an afterthought bolted on later. Because that socket
  can inject any keystroke, it's treated as a privileged channel: it lives in
  `$XDG_RUNTIME_DIR` (override with `$FLEXWM_SOCKET`), is created `0600`, and
  serves only connections from the same user as the compositor.

`flexwm-core`, the layout/state engine, is kept platform-independent on
purpose: it knows nothing about Wayland. The plan is for the same engine to
eventually back a macOS Accessibility-API adapter, doing OmniWM-style window
layout on a Mac, not just on Linux.

## Status

Early, but all three Wayland backends are real and working: `--headless`
(pixman rendering, xdg-shell, seat/input, full IPC control surface),
`--nested` (runs as a window inside an existing compositor, e.g. for webtop),
and `--tty` (a real DRM/KMS + libseat + libinput backend on actual hardware,
including VT switching and a rendered pointer cursor — a client's own cursor
image when it supplies one, a built-in shape otherwise). Also done: vim-style
keybindings, a TOML config file (`--config`, `[layout]`/`[binds]`), window
decorations (a niri-style focus ring, background color, server-side
`zxdg_decoration_manager_v1`), and a hardened control socket (owner-only
permissions, a same-user peer check, a 1 MiB cap on a single request, and
screenshots rate-limited to one per connection per frame) whose connections
are non-blocking end to end, so no client — however slow, chunked or
unresponsive — can stall the compositor for anyone else. Verified
end-to-end on every backend — a real
client maps, tiles, receives synthetic input, and a screenshot proves it. Not
yet started: a GPU rendering path and the macOS adapter.

## Layout

```
crates/
  flexwm-core/   Platform-independent window state and scrolling-tile layout.
                 No Wayland, no I/O — pure data and functions, fuzz-tested.
  flexwm-ipc/    The wire protocol (requests/responses) and a client over it.
                 Builds on any platform.
  flexwm/        The CLI and, on Linux only, the Smithay-based compositor.
                 `flexwm msg ...` builds everywhere, so it can drive a
                 compositor running in a VM from a Mac.
vm/              A NixOS VM + flake for developing the Linux-only compositor
                 from macOS. See vm/README.md.
scripts/         smoke-test.sh: an end-to-end test driven entirely over IPC.
```

## Building

```sh
nix develop        # every dependency, on Linux or macOS
cargo build         # flexwm-core, flexwm-ipc, and the CLI build anywhere;
                     # the compositor itself only compiles on Linux
cargo test --workspace
```

To actually run the compositor you need a real (or virtual) Linux machine with
a seat — see `vm/README.md` for a Mac-native NixOS VM that provides one.

## Running

```sh
flexwm --headless --width 1280 --height 800 -- foot   # start, spawn a terminal
flexwm --nested --width 1280 --height 800 -- foot     # inside your existing compositor
flexwm --tty -- foot                                  # on a real DRM/KMS seat
flexwm msg windows                                     # in another shell
flexwm msg action focus-column left
flexwm msg screenshot --out /tmp/shot.png
flexwm msg type "hello"
flexwm msg wait-idle --quiet-ms 200
```

Add `--config PATH` to any of the three to load a TOML config (`[layout]` and
`[binds]`); without it, flexwm looks for
`$XDG_CONFIG_HOME/flexwm/config.toml` and falls back to built-in defaults if
that's missing or malformed. Run `flexwm --help` for the full request/action
list.

## License

MIT. See `NOTICE` for third-party attribution (this compositor is built on
[Smithay](https://github.com/Smithay/smithay)).
