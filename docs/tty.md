# Running on real hardware (`--tty`), and which renderer draws

`--tty` is a real DRM/KMS + libseat + libinput backend: scoot *is* the
session, with VT switching, hotplug and a drawn pointer cursor.

```sh
scoot --tty -- foot                          # whatever the seat has
scoot --tty --gpu /dev/dri/card0 -- foot     # ...naming the DRM device yourself
scoot --tty --mode 1920x1080 -- foot         # ...naming the display mode
```

It needs a seat (`seatd` or logind) with a DRM device on it. On a modern
kernel, on every non-root `--tty` run, Smithay logs `Unable to become drm
master, assuming unprivileged mode` at startup — expected, not a failure: the
session manager opens the device and already holds DRM master on scoot's
behalf, and scoot simply isn't permitted to call `SET_MASTER` itself on a
file another process opened. `vm/README.md`'s troubleshooting section has the
kernel-level reason and two commands that check whether master really is
held.

- [Which DRM device `--tty` drives](#which-drm-device---tty-drives)
- [Hotplug and host resizes](#hotplug-and-host-resizes)
- [Which renderer draws the frames](#which-renderer-draws-the-frames)

## Which DRM device `--tty` drives

Normally: whichever one works. scoot asks Smithay for the seat's primary GPU,
and if that device turns out not to be able to drive a display, it tries
every other DRM device on the seat in turn before giving up. On an ordinary
PC the first pick is right and nothing else is ever opened; the log line
worth grepping for either way is `drm: driving this device`, with the device
path on it (`RUST_LOG=info`, the default level, is enough — a rejected device
gets a `drm: device unusable` warning naming it and saying what it said).

The fallback exists because the usual heuristic — "the GPU whose PCI parent
has `boot_vga=1`, else the first one with a render node" — assumes the 3D GPU
and the display controller are the same DRM device. On Apple Silicon under
Asahi Linux they are not: `asahi`/AGX has the render node, `apple,dcp` owns
the CRTCs and connectors, and there is no PCI GPU or VGA BIOS for the first
rule to match. Picking the render-only device there fails with `Operation not
supported (os error 95)` loading its KMS resources.

**Confirmed working on Apple Silicon (2026-09-18).** On an Apple M2
(`apple,t8112`), the automatic search rejects the `asahi` render node and
drives the `apple-drm` display controller unattended, as a daily-driven
`--tty` session with no `--gpu` and no `[tty] gpu`:

```
drm: device unusable path=/dev/dri/card1 reason=has no usable KMS pipeline
     -- loading its DRM resources failed (Operation not supported (os error 95))
drm: driving this device path=/dev/dri/card2 connector=eDP-1 width=2560 height=1600
```

So **`--gpu` is not needed there** — don't reach for it first on Apple
Silicon. Naming a device is not a guarantee either: it skips the *search*,
not the checks, so it still has to open through the session and pass the same
KMS probe every automatic candidate does. [`../Asahi.md`](../Asahi.md)
records that run and remains the runbook for re-checking it on a different
Apple Silicon model.

If the automatic search picks wrong, name the device — the one that owns the
connectors, never a render-only node:

```sh
scoot --tty --gpu /dev/dri/card0 -- foot
```

`--gpu PATH` replaces the search entirely — exactly that device, no fallback
— so a wrong path is a clean startup error naming the device and what failed,
not a silent fall back to something else. It only means anything under
`--tty`; on `--headless` or `--nested` it is ignored with a warning. One
thing it can cost: hotplug is followed only for devices udev lists as GPUs on
this seat, and `--gpu` can name one that isn't in that list. scoot says so at
startup — `the chosen device is not in udev's list for this seat, so display
changes on it will not be noticed` — and the session otherwise runs exactly
as it always did, with the mode it started on.

On hardware where the automatic search picks wrong every time, the config
file saves retyping the flag:

```toml
[tty]
gpu = "/dev/dri/card0"
```

`--gpu` wins when both name one, the way `--config` beats the default path.
An explicitly-set-but-empty path in either (`--gpu ""`, `gpu = ""`) is a
startup error naming the surface that set it, on every backend. See
[`[tty]`](configuration.md#tty) for the rest.

### Naming the mode

The output's size is the connector's preferred mode. When that is the wrong
size — under Apple's Virtualization framework (vfkit, UTM) the "preferred"
mode is just the host window's size in backing pixels, so it doubles or
halves with whichever screen the window opened on — name the mode:

```sh
scoot --tty --mode 1920x1080 -- foot
```

`--mode WxH` picks the connector mode of exactly that size, and falls back to
the preferred one with a warning if the connector lists no such mode (`cat
/sys/class/drm/card*-*/modes` shows what it lists). Like `--gpu`, it is
ignored with a warning outside `--tty`.

### When nothing works

The startup error lists every device that was tried and why each one was
rejected, rather than naming only the first. If the *session* refused all of
them — which is what happens when another compositor already holds the seat,
since a seat takes one client at a time — the error says so instead of
suggesting `--gpu`: no choice of device gets around a busy seat. And if
enumerating the seat's other devices fails outright, the primary pick is
still tried on its own (with a `could not list the seat's other devices`
warning), rather than losing a working device to a failure in the fallback
machinery.

To see what the seat has, and which driver is behind each device:

```sh
ls /dev/dri/card*
for c in /sys/class/drm/card*/device/driver; do echo "$c -> $(readlink -f "$c")"; done
```

## Hotplug and host resizes

`--tty` watches udev for DRM changes and re-runs the choice above whenever
the display underneath it moves, so nothing here is a once-at-startup
decision:

- **Plug a monitor in or pull one out.** Unplugging the connector scoot is
  driving makes it pick another connected one and mode-set onto it — moving
  to a different CRTC when the display controller only routes that connector
  there (ordinary PC graphics route any connector to any CRTC; ARM SoCs often
  wire encoders to specific CRTCs). If no CRTC on the device can drive the
  new connector it logs `no other crtc on this device can drive the new
  connector` and stays put, retrying on the next hotplug. Plugging one back
  in after everything was unplugged mode-sets back onto it.
- **Resize, rescale or full-screen a VM window.** Apple's Virtualization
  framework reconfigures the guest display when you do, which reaches the
  guest as a hotplug with a new mode list and a new preferred mode; scoot
  follows it. `--mode WxH` is honoured on every re-probe, not just the first.

A change reaches everything that cares: the render target, `wl_output` (`mode`
+ `done`), `wlr-output-management`, layer-shell surfaces (bars re-anchor and
re-arrange), the window layout, and `scoot msg outputs`.

With nothing connected at all, scoot holds the last frame, keeps the session
running and logs `nothing is connected to this device any more` — plug a
display back in and it mode-sets onto it. Hotplug events that arrive while
scoot is on another VT are picked up on the switch back, since a session
without DRM master cannot mode-set when they happen.

Two limits:

- **Still one output.** Plugging a second monitor into a laptop already
  running on `eDP-1` keeps the session on `eDP-1` rather than jumping to the
  new screen — scoot drives one output, so one of the two has to be dark, and
  the one you are looking at is the one it keeps. Real multi-output is a
  separate, larger piece of work.
- **The output keeps the name it started with.** A session that started on
  `HDMI-A-1` and fell back to `eDP-1` when the cable came out still reports
  `HDMI-A-1`. Renaming a `wl_output` is not something the protocol allows;
  recreating it would make every client re-enter the output and re-map its
  surfaces, which is a bigger lie about what happened than a stale name.

Under `--tty` the output is named after its connector — `HDMI-A-1`, `eDP-1`,
`Virtual-1`, the same spelling as `/sys/class/drm/card*-*` — so bars and
shells label the screen as they would under any other compositor.
`--headless` and `--nested` have no connector and keep the name `headless`.

## VT switching

`--tty` binds `Ctrl+Alt+F1` through `Ctrl+Alt+F12` to switching to VT 1
through 12. These are added *after* the config file loads and always win over
a colliding config-file bind (logging a warning naming whatever they
displaced): on real hardware, with no other window manager and often no easy
remote access, `Ctrl+Alt+Fn` is the one recovery path if the display ever
gets wedged, so it can't be allowed to silently lose to a config-file typo.
They keep working while a full-screen layer surface, a popup grab or a
session lock is holding every other keystroke.

## Which renderer draws the frames

`--renderer pixman|gles` (config: [`[renderer] backend`](configuration.md#renderer))
picks what composites each frame. This is a different axis from
`--headless`/`--nested`/`--tty`, which choose how the compositor *presents*
what it drew.

**The default is `pixman`, the CPU renderer, and that is not changing** —
running with no GPU at all is a hard requirement here, not a fallback tier.
`gles` is opt-in, and worth being precise about:

- **What it buys you.** Correctness parity with pixman on a second renderer,
  and the groundwork for scanning a GPU buffer out directly under `--tty`.
  Every pixel-readback test in the suite — session lock, layer shell, alpha
  modifier, single-pixel buffers, the cursor, output scaling — passes
  byte-identically under either renderer.
- **What it does not buy you yet.** No speed. The frame is composited into an
  offscreen buffer and then read back to main memory exactly as pixman's is,
  so `gles` adds a GPU round trip without removing any CPU copy; on a machine
  whose "GPU" is a software rasteriser (llvmpipe, which is what a VM or a
  GPU-less container has) it is several times *slower* than pixman. Under
  `--tty` the scanout path below removes that round trip; everywhere else
  the read-back stands.
- **Under `--tty`, `gles` now scans out from the GPU** — in a build with
  `--features gpu-scanout` (see below). That is the path where it stops
  being a round trip: the frame is scanned out directly instead of being
  read back and memcpy'd into a dumb buffer. Without that feature `--tty`
  still warns and keeps pixman, because the read-back-and-copy shape it
  would otherwise take is strictly worse than compositing on the CPU.
  `--headless` and `--nested` are still read-back in every build.
- **Scanout is primary-plane only, plus the cursor where the hardware has a
  plane for it.** Overlay planes are still untouched (step 2 of
  `docs/backlog/rendering/gpu-scanout-planes.md`); the cursor rides a KMS
  cursor plane on CRTCs that expose one and stays composited into the primary
  plane everywhere else, with per-frame fallback to compositing where the
  plane cannot be claimed. Two paravirt caveats, both measured on the dev
  VM's virtio-gpu: the kernel hides the cursor plane until the session sets
  `CURSOR_PLANE_HOTSPOT` (which scoot does once per `--tty` session --
  without it the inventory is primary-only despite the plane existing), and
  even shown, virtio refuses the atomic TEST for the cursor state, so there
  the tier attempts the plane every frame and falls back every frame. One consequence to know: a capture (IPC
  screenshots, `ext-image-copy-capture-v1`) reads the primary plane only, so
  on a session whose cursor is plane-assigned the capture shows the screen
  *without* the cursor; where the cursor is composited, captures keep showing
  it. The startup log says which it is (`drm: scanout cursor planes
  cursor_planes=N ...`). It has
  now run on a real GPU: on an Apple M2 under Asahi Linux (`2560x1600@60`)
  it costs **4–5x less compositor CPU** than the default dumb-buffer tier
  under damage — 20.8% of a core down to 4.3% under large-damage pointer
  motion (the injected path jumps ~900×600 logical pixels per event, so this
  is not a small cursor-rect move), 52.0% down to 14.0% under a full
  relayout — while drawing the same pixels and
  drawing about 0.2 W *less* power. It costs 7–16 MB more RSS for the GBM
  swapchain, and both tiers use no measurable CPU at idle. Numbers and method
  in [`../Asahi.md`](../Asahi.md)'s Test 4.
- **A resize is expensive under `gles`, and `--nested` now resizes.** Every
  resize rebuilds the render target, and under `gles` that means a whole new
  EGL context and shader set: measured on the dev VM (llvmpipe, 800x800, 8
  windows) at **16.6 ms** per resize against **37 µs** for pixman (both
  measured with `CARGO_PROFILE_RELEASE_LTO=thin`, not the repo's fat-LTO
  release profile, which OOM-kills on the 3.8 GB dev VM) — a full
  60 Hz frame apiece. It is once per *distinct* size, not once per event, so
  settling at a new size costs one; dragging a `--nested --renderer gles`
  window to resize pays it per distinct size that drag passes through and
  will stutter until you let go. pixman is unaffected, and it is the
  default. If this ever matters, the fix is resizing the GLES target in
  place instead of rebuilding it.
- **Hardware first.** The EGL device is chosen by preferring a real device
  over a software one and taking the first that yields a working renderer, so
  a box with a GPU uses the GPU. Note that "software" here means only that
  the device advertises `EGL_MESA_device_software`: a real device *node*
  backed by a software driver answers no, so it is preferred and then served
  in software anyway. The chosen device is logged at startup (`the GLES
  renderer is up device=/dev/dri/renderD128 software=false`) — trust that
  line over the flag name.
- **A wrong `--renderer gles` is a startup error, not a silent downgrade.**
  If no EGL device can drive it -- including a box with no loadable libEGL
  at all, which the compositor probes before Smithay's first EGL touch so
  the missing library reports instead of panicking -- scoot says so and
  names each failure rather than quietly compositing with the other
  renderer.
- **dma-buf clients follow the renderer.** This used to be a known gap — the
  advertised buffer formats were the CPU renderer's whichever renderer was
  active, so a GPU buffer the GLES renderer could not import was refused, and
  through `create_immed` that disconnects the client. Since the stage-4
  change the `zwp_linux_dmabuf_v1` feedback names only what the *active*
  renderer can really import, so there is nothing left to stay on `pixman`
  for. Two consequences worth knowing: on a renderer that can import neither
  of the formats scoot serves, no dmabuf global is advertised at all (GL
  clients fall back to `wl_shm`, and a shell that waits for dmabuf feedback
  before capturing — quickshell does — stays waiting), and `main_device` in
  that feedback is the renderer's own DRM render node rather than a guessed
  path, which is what makes a client's allocation land on the device the
  import will happen on. See
  [protocols.md](protocols.md#gpu-rendering-clients-zwp_linux_dmabuf_v1).

### The `gpu-scanout` build feature

There is one optional Cargo feature, **`gpu-scanout`**, off by default:

```sh
cargo build -p scoot --features gpu-scanout
# ... or from the flake, which carries the same feature as a package:
nix build .#scoot-gpu
```

It is what the `--tty` GPU scanout tier is built behind (Smithay's
`DrmCompositor` over a GBM swapchain). With it, `--tty --renderer gles`
scans out from the GPU instead of reading each frame back; without it,
`--tty` warns and keeps pixman however `gles` was asked for.

One limit worth knowing before you turn it on: scanout drives the **primary
plane plus the cursor plane** — no overlay planes. The cursor plane is used
where the CRTC exposes one (the dev VM's virtio-gpu does: one `Cursor` plane
per `drm_info`) and silently not elsewhere; a plane-assigned cursor is absent
from captures, which read the primary plane only. Whether `apple,dcp`
exposes a usable cursor plane is still unknown (no plane inventory exists
from the Asahi runs). Virtio's own footnote: its cursor plane needs the
session's `CURSOR_PLANE_HOTSPOT` cap to be enumerated at all, and even then
its kernel refuses the atomic TEST, so on virtio the tier enumerates the
plane, attempts it per frame, and composites the cursor every frame. The case for scanout is no longer
reasoned but measured, on an Apple M2 under Asahi Linux: **4–5x less
compositor CPU** under damage, the same pixels, ~0.2 W less power, 7–16 MB
more RSS (see [`../Asahi.md`](../Asahi.md)'s Test 4). On a machine whose
"GPU" is a software rasteriser the tier is still a loss, which is why pixman
remains the default.

It is off by default because it is the one thing in the tree that adds a
**link-time** dependency on `libgbm`: the resulting binary carries
`libgbm.so.1` in its `DT_NEEDED` list and will not start on a machine
without it, while the default build has no such entry and runs anywhere.
Running with no GPU stack at all is a hard requirement here, so `cargo
build` keeps producing the binary that does.

`--renderer gles` is *not* in the same position and needs no feature:
libEGL and libGLESv2 are `dlopen`ed, so a GPU-less machine only fails when
that renderer is actually asked for, at startup, with a message.
