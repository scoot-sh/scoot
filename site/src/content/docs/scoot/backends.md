---
title: Backends and rendering
description: "Headless, nested and TTY backends; the pixman and GLES tiers; which DRM device runs; VT switching."
---

The backend flag chooses how scoot *presents* what it drew; the
renderer flag chooses what *draws* it. They are independent axes —
any renderer runs on any backend — but only some combinations buy
anything. (Deciding which *package* to install is the [install
page's](../start/install.md#which-build-do-i-need) one-command check;
this page is what happens underneath.)

## The three backends

| Backend | Presents via | Outputs | Scale |
|---|---|---|---|
| `--headless` | nowhere; `scoot msg screenshot` reads the framebuffer | N virtual outputs (`--outputs`, 1–8) | from the config |
| `--nested` | a window in the host compositor | one window; follows the host's size live | the host owns it (a set scale is ignored with a warning) |
| `--tty` | real DRM/KMS hardware on a console | every connected monitor, by connector name | from the config, per output |

Under `--nested` the session follows the host window's size for life:
resize it and the desktop inside resizes with it; if a size cannot be
allocated scoot logs it, stays where it was, and the host letterboxes
the difference. Under `--tty` monitors plug and unplug live (below).

Under `--nested` scoot is a client of the host as well as a server to
its own windows. If the host connection breaks — the host exits or
restarts — the session cannot present anything and no input can reach
it, so scoot stops cleanly instead of lingering displayless: the log
names the loss (`lost the connection to the host compositor`) and the
process exits 1 with the same message. A supervisor (s6, systemd)
restarts it into a fresh session once the host is back.

Starting up waits at most 10 seconds for the host to answer each
handshake step: the registry, and, in a `gpu-scanout` build with a GPU
renderer, the dma-buf feedback too, so up to 20 seconds there. A host that accepts the connection but never answers — a
restart that came back listening without dispatching — used to wedge
the session before `scoot is up`, with no IPC socket and nothing for a
supervisor to act on. Now the log names it (`could not reach the host
compositor during startup`) and the process exits 1, so the supervisor
retries. There is no knob for the wait: ten seconds is ~170x a healthy
startup, and one bound every host meets beats a tunable nobody asked for.

> **Symptom:** `scoot msg` says `Connection refused` right after the
> host went away. The nested session is gone — check its log for the
> `lost the connection` line — not deaf. Restart the session (or let
> the supervisor do it) once the host is back; a stale `scoot.sock`
> file from the dead session is replaced on the next start.

> **Symptom:** nested scoot exits 1 seconds after starting, with no
> `scoot is up` and no `scoot.sock` at all. Check its log for the
> `could not reach the host compositor during startup` line — the host
> accepted the connection but never answered. Restart the host (or wait
> for the supervisor's next retry); the next start replaces the stale
> state.

## Which renderer draws the frames

`--renderer pixman|gles` (config: `[renderer] backend`) picks what
composites each frame. **The default is `pixman`, the CPU renderer,
and that is not changing** — running with no GPU at all is a hard
requirement, not a fallback tier. `gles` is opt-in:

- **What it buys.** Correctness parity with pixman on a second
  renderer, and the groundwork for scanning a GPU buffer out directly
  under `--tty`. Every pixel-readback test passes byte-identically
  under either renderer.
- **What it does not buy (yet).** No speed, except on the scanout
  path below: the frame is composited into an offscreen buffer and
  read back to main memory exactly as pixman's is, so `gles` adds a
  GPU round trip without removing any CPU copy — on a machine whose
  "GPU" is a software rasteriser it is several times *slower* than
  pixman.
- **Under `--tty`, `gles` scans out from the GPU** — in a
  `gpu-scanout` build. The frame is scanned out directly instead of
  being read back and memcpy'd into a dumb buffer. Without that
  feature `--tty` warns and keeps pixman, because the read-back shape
  would be strictly worse than the CPU. With the feature, a device
  that cannot drive the scanout tier warns and falls back the same
  way — CPU renderer and dumb buffers — instead of refusing to start:
  on `--tty` scoot *is* the session, so a refusal would be a lockout.
  `--headless` is read-back in every build.
- **Under `--nested`, a `gpu-scanout` build hands each frame to the
  host as a dma-buf** — no CPU copy on scoot's side, nothing for the
  host to upload — but only when it is safe (the host offers dma-buf
  feedback naming the same DRM device, a common format, and a startup
  test buffer allocates, renders and copies). Anything else keeps the
  read-back, and the startup log says which and why, once.
- **Hardware first.** The EGL device is chosen by preferring a real
  device over a software one and taking the first that yields a
  working renderer. The chosen device is logged at startup (`the GLES
  renderer is up device=/dev/dri/renderD128 software=false`) — trust
  that line over the flag name.
- **A wrong `--renderer gles` is a startup error, not a silent
  downgrade — when EGL itself is missing or broken.** If no EGL device
  can drive it scoot says so and names each failure rather than quietly
  compositing with the other renderer. There is no automatic fallback
  on that path: drop the flag (stay on pixman) or fix the cause. (The
  deliberate exception is `--tty` GPU scanout above: there scoot warns
  and keeps the CPU renderer instead of refusing to start.)

Measured where it pays (Apple M2 under Asahi Linux): 4–5x less
compositor CPU under damage than the default tier, ~0.2 W less power,
7–16 MB more RSS, no idle difference. A fullscreen window covering its
output goes primary-direct (zero-copy), decided per frame; GPU clients
get the driver's whole import set (tiled and compressed layouts,
multi-plane YUV included), where the CPU tier offers linear RGB only.

**The pointer rides a plane when the display has one for it.** With a
hardware cursor plane, scoot's pointer goes there. A display with no
cursor plane but a free overlay plane (Apple Silicon's DCP has two)
gets scoot's own pointer shapes as small `LINEAR` buffers on the topmost
overlay instead of compositing them. So a visible pointer no longer stops
the fullscreen window under it from going direct, and moving the pointer
only moves the plane, with nothing redrawn. On the M2, fullscreen mpv
with the pointer visible and still went from 22–33 to 13–15 jiffies per
10 s, and pointer motion over a desktop from 17–19 to 11 (`Asahi.md`,
Test 17). It falls back to compositing, frame by frame, wherever the
plane cannot take it: a pointer so near a screen edge (in practice the
right or bottom one) that under 32 pixels of its plane would stay on
screen (DCP refuses planes that small), a client's own cursor
image in shared memory, or a display with no overlay to spare. Above
scale 1 the shape's right and bottom edges are filtered against
transparency rather than repeated, which can soften that one-pixel edge
slightly. Screenshots and screen capture still show the pointer exactly
where the panel does.

| Field | Type | Default | Reload | Meaning |
|---|---|---|---|---|
| `[renderer] backend` | `"pixman"` / `"gles"` | `"pixman"` | restart only | Which renderer composites each frame. A name this build *knows* but cannot build (`gles` with no working EGL) is a startup error; `--renderer` wins over the file either way. |

## Which DRM device `--tty` drives

Normally: whichever one works. scoot asks for the seat's primary GPU,
and if that device cannot drive a display it tries every other DRM
device on the seat in turn. The log line worth grepping is `drm:
driving this device`, with the device path on it (a rejected device
gets a `drm: device unusable` warning naming it and what it said).

If the automatic search picks wrong, name the device — the one that
owns the connectors, never a render-only node:

```sh
scoot --tty --gpu /dev/dri/card0 -- foot
```

```toml
[tty]
gpu = "/dev/dri/card0"
```

`--gpu` replaces the search entirely — exactly that device, no
fallback — so a wrong path is a clean startup error, not a silent
fallback. It means nothing outside `--tty` (ignored with a warning).
Prefer a stable `/dev/dri/by-path/...` alias over a `cardN` number.

| Field | Type | Default | Reload | Meaning |
|---|---|---|---|---|
| `[tty] gpu` | string (device path) | unset (automatic search) | restart only | Which DRM device `--tty` drives, when the search is wrong. Set means exactly that device: a wrong or empty path is a startup error naming the key. |

> **Symptom:** every device refused, "seat takes one client at a time".
> Another compositor already holds the seat — no choice of device gets
> around a busy seat. To see what the seat has: `ls /dev/dri/card*`.

## Hotplug, VT switching, captures

**Hotplug.** `--tty` watches udev and re-runs the device choice
whenever the display moves: a plugged monitor gets an output of its
own, placed right of the others, without moving the session off
already-lit screens; pulling one adopts its workspaces elsewhere (and
a matching monitor's return moves them back). With nothing connected
at all scoot holds the last frame and keeps running. One tier for the
whole session: the first monitor decides it — if the first falls back
to dumb buffers, every monitor uses dumb buffers, and a later monitor
that cannot join the GPU tier stays dark rather than mixing tiers.

**VT switching.** `--tty` binds `Ctrl+Alt+F1`…`F12`, layered on *after*
the config loads and always winning over a colliding file bind (with a
warning naming what they displaced) — on real hardware that is the one
recovery path, so it cannot silently lose to a typo. They keep working
under fullscreen grabs and the session lock, and a reload cannot strip
them.

### Captures and the pointer

Screenshots and screen capture read the
same composited frame the outputs show — which is why `scoot msg
screenshot` works on every backend, including `--headless` with no
display at all. See [Screenshots](../msg/screenshots.md). A VNC
server like wayvnc reads the same frame over the capture protocol and
drives input through the virtual-input globals — see [Remote
desktop](./remote-desktop.md).
