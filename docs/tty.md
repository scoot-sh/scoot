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
- **Scanout drives every plane it can claim, with captures kept correct.**
  The cursor plane is attempted on CRTCs that expose one, with per-frame
  fallback to compositing where the plane cannot be claimed; overlay planes
  ride along whole from the CRTC's inventory the same way, and likewise fall
  back per frame. Two paravirt caveats, both measured on the dev
  VM's virtio-gpu: the kernel hides the cursor plane until the session sets
  `CURSOR_PLANE_HOTSPOT` (which scoot does once per `--tty` session --
  without it the inventory is primary-only despite the plane existing), and
  the cap must land before `DrmDevice::new` (with `ATOMIC` first) or the
  plane is visible to queries but unknown to commits. Overlay planes need no
  such cap (`UNIVERSAL_PLANES` exposes them), and virtio-gpu has none to
  expose anyway -- its inventory is one primary plus one cursor plane. On
  virtio the cursor then scans out live (pointer-tracked; captures draw it
  back in when they ask for it). No window surface is ever an overlay candidate (every one is
  built `Kind::Unspecified`), so an overlay plane carries at most the
  cursor -- never a window; candidate-marking is
  [`backlog/core/gpu-overlay-window-candidates.md`](backlog/core/gpu-overlay-window-candidates.md).
  A capture (IPC screenshots, `ext-image-copy-capture-v1`) reads the
  primary plane's swapchain slot, which lacks a plane-assigned cursor --
  so every capture reconciles the cursor with what it asked for rather
  than with what the slot happens to hold: the cursor's region is
  re-rendered from the frame's own element list, with the pointer or
  without it, and written over the copy (see
  [Captures and the pointer](#captures-and-the-pointer)). The pointer in a
  capture is therefore the same on this tier as on the dumb tier. The
  startup log still says which planes exist (`drm: scanout cursor planes
  cursor_planes=N overlay_planes=M ...`). It has
  now run on a real GPU: on an Apple M2 under Asahi Linux (`2560x1600@60`)
  it costs **4–5x less compositor CPU** than the default dumb-buffer tier
  under damage — 20.8% of a core down to 4.3% under large-damage pointer
  motion (the injected path jumps ~900×600 logical pixels per event, so this
  is not a small cursor-rect move), 52.0% down to 14.0% under a full
  relayout — while drawing the same pixels and
  drawing about 0.2 W *less* power. It costs 7–16 MB more RSS for the GBM
  swapchain, and both tiers use no measurable CPU at idle. Numbers and method
  in [`../Asahi.md`](../Asahi.md)'s Test 4.
- **A fullscreen window scans out directly (zero-copy).** When a fullscreen
  window covers the output and its app hands scoot a dma-buf the display
  can take, the primary plane shows that buffer itself: no compositing, no
  copy. Which frames may try is decided per frame
  (`render/primary_direct.rs`): the session is unlocked, a fullscreen
  window covers the output, no capture client is streaming it, nothing
  in the frame is translucent (`wp_alpha_modifier_v1`) or a rounded window,
  and the element Smithay would try for the primary is the window's own
  (rule 6, mirroring Smithay's walk). That holds when the window is opaque
  over the whole output (an opaque-format buffer, or one whose surface
  declares an opaque region covering it), or when the background is black
  and nothing visible lies under the window (a solid-colour wallpaper made
  of a single-pixel buffer counts as the background: Smithay clears to its
  colour instead of drawing it, so a black one qualifies). A fullscreen app with an
  alpha-format buffer and no opaque region is not eligible over the default
  background, nor over any wallpaper (Smithay would try the wallpaper, or
  nothing), and so is not steered either (below).
  Those frames pass `ALLOW_PRIMARY_PLANE_SCANOUT_ANY`; every other frame
  passes no primary bit at all, so a tiled window never goes direct, even
  one covering the whole output over a black background. `ANY` is what lets
  the client's framebuffer (the opaque `XR24` variant) replace the `AR24`
  swapchain without changing the format every composited frame is drawn
  in: the framebuffer still carries its own format, the plane must still
  list that exact format, and the atomic test still has to pass --
  what `ANY` skips is only the "same format as the swapchain" check, and
  the one difference that check guarded (alpha) cannot show on the bottom
  plane under an opaque window (the reasoning, traced at the pinned
  Smithay rev, is on `DIRECT_FLAGS` in `tty/scanout.rs`). Anything drawn
  above the window -- a notification on the `overlay` layer, a popup
  menu, a cursor with no plane of its own -- makes that frame composite,
  as does a `wl_shm` buffer, a buffer that does not cover the output, or
  one the display refuses; the next frame without it goes direct again.
  The lock screen always composites. A direct frame is not in the buffer a
  capture reads, so a capture of it forces one composite frame first (IPC
  `screenshot` and `ext-image-copy-capture-v1` alike; a median 3-7 ms
  more per screenshot on the dev VM, polled at 1-10 Hz, while the
  compositor itself uses a fraction of the CPU compositing would), and a
  capture that cannot be forced (the
  session is VT-switched away) is refused with "held for direct scanout;
  retry once a composite frame lands" rather than served stale. A
  capture *stream* is different: forcing every frame of one measured worse
  than compositing throughout, so while a session is capturing (a frame
  parked, or one asked for within the last second) the output composites
  as it always did. Seen live on the dev VM's virtio-gpu, at default
  config, with a test client allocating card0 dumb buffers (card0
  allocations, dumb or GBM `LINEAR`, are the client buffers virtio can
  import at all; a GL client's buffers there cannot): the primary on the
  client's framebuffer, captures byte-correct, the VT-away refusal and
  recovery, a lock over it composited; compositor CPU for that fullscreen client
  dropped from ~76% of a core (llvmpipe compositing) to ~1.5%. Not yet
  seen on real GPU hardware or with a real video player --
  [`../Asahi.md`](../Asahi.md)'s Test 5 asks for that. The log line
  `scanout: primary-direct eligibility changed` (at `debug`) says when a
  session starts or stops being allowed to go direct, and why. A frame
  that did go direct tells that window's `wp_presentation` feedback
  `zero_copy` (no other frame or surface is told it).
- **A fullscreen window is told which layouts the display can take.** A
  GL client picks its buffer layout from the dma-buf feedback, by what it
  renders into best -- on a real GPU often a tiled or compressed layout the
  primary plane cannot scan out, so it would composite for no reason it
  could know. So the window covering an output, while that output may go
  direct (the rules above), gets per-surface feedback whose first tranche
  is flagged `scanout`, names the display device, and lists the layouts
  the primary plane accepts out of those already advertised
  (`dmabuf/scanout.rs`); a client that acts on it reallocates into one and
  goes direct. It is sent only when that changes: at once when another
  window (or none) covers the output, on the first frame drawn two seconds
  or more after the same window, still covering, stopped being able to go
  direct (locked, streamed, translucent),
  and not at all for a notification or popup drawn over it. Every pair in
  it is one the default feedback already offers, so a client that allocates
  from it and ends up composited is imported like any other. On the dev
  VM's virtio-gpu (no `IN_FORMATS`) the tranche is `XR24` and `AR24` at
  `LINEAR`, logged once as `dmabuf feedback: scanout tranche for
  fullscreen windows`; a tiled modifier the display's GBM is seen to lose on
  import -- the scanout exporter refuses such a framebuffer, since KMS would
  show scrambled tiles -- is dropped from it and the window re-sent. Whether a real GL client reallocates into the tranche on
  real hardware and goes direct is [`../Asahi.md`](../Asahi.md)'s Test 6 --
  not yet seen; no GL client on the dev VM can allocate a dma-buf at all.
- **A resize under `gles` keeps the renderer.** `--nested` follows its host
  window's size, and under `--renderer gles` each new size reallocates only
  the offscreen target: the EGL context, its shaders and every client
  texture already uploaded stay as they are. On the dev VM (llvmpipe,
  800x800, 8 windows, release build with LTO off) a resize costs **15.7 µs**
  against **11.3 µs** for pixman; rebuilding the renderer, which it used to
  do, cost **3.95 ms** there. (It was first recorded as 16.6 ms against
  37 µs for pixman; that did not reproduce, and pixman shows the same ~3x
  gap, so the earlier conditions differed.) What a size change still costs is the frame drawn at the
  new size: a `--nested --renderer gles` drag measured **14.7 ms** of
  compositor CPU per size (was 17.0 ms), against 4.5 ms under pixman, and
  that is llvmpipe drawing and reading back a whole frame, not the resize.
  pixman is unaffected, and it is the default. If the GPU refuses a size (larger than it can render), scoot
  tries a whole new renderer on the same GPU and, if that fails too, stays
  at the size it was, as before.
- **Hardware first.** The EGL device is chosen by preferring a real device
  over a software one and taking the first that yields a working renderer, so
  a box with a GPU uses the GPU. Note that "software" here means only that
  the device advertises `EGL_MESA_device_software`: a real device *node*
  backed by a software driver answers no, so it is preferred and then served
  in software anyway. The chosen device is logged at startup (`the GLES
  renderer is up device=/dev/dri/renderD128 software=false`) — trust that
  line over the flag name. Only the session's first build chooses: a resize
  keeps the renderer it has, and every later build (another `--headless
  --outputs` output, or a resize the GPU could not reallocate in place) stays
  on that same device; if it cannot build there the resize is refused (the
  log says `could not resize the render target`) rather than moving to
  another GPU, whose driver might not take the buffer layouts clients were
  already offered.
- **A wrong `--renderer gles` is a startup error, not a silent downgrade.**
  If no EGL device can drive it -- including a box with no loadable libEGL
  at all, which the compositor probes before Smithay's first EGL touch so
  the missing library reports instead of panicking -- scoot says so and
  names each failure rather than quietly compositing with the other
  renderer.
- **dma-buf clients follow the renderer, and on `gles` get the GPU's own
  formats.** This used to be a known gap — the advertised buffer formats
  were the CPU renderer's whichever renderer was active, so a GPU buffer the
  GLES renderer could not import was refused, and through `create_immed`
  that disconnects the client. Since the stage-4 change the
  `zwp_linux_dmabuf_v1` feedback names only what the *active* renderer can
  really import, so there is nothing left to stay on `pixman` for. Under
  `gles` (both the offscreen tier and the `--tty` GPU scanout tier, whose
  renderer is the one on the GBM device) that is the driver's whole import
  set: every format at every explicit modifier it names, tiled and
  compressed layouts and multi-plane YUV (`NV12`, `P010`, …) included —
  where it used to be `Xrgb8888`/`Argb8888` at `LINEAR` only, which forced
  every GL client into linear buffers and every video player into a
  conversion. Implicit-modifier entries are never offered next to explicit
  ones (a YUV buffer imported that way draws the wrong colours). On the dev
  VM's llvmpipe that is 57 formats, all `LINEAR`, identical on both GLES
  tiers; on real hardware it is recorded, not promised
  ([`../Asahi.md`](../Asahi.md)'s Test 6). A multi-plane YUV buffer can go
  direct on the primary plane only where that plane lists the format;
  otherwise it composites. The same goes for a tiled layout: with those on
  offer, a fullscreen GL client on real hardware may pick one the display
  cannot scan out and composite rather than go direct — which is what the
  per-surface scanout tranche above steers it away from.
  Two consequences worth knowing: on a renderer
  that can import nothing scoot can vouch for, no dmabuf global is
  advertised at all (GL clients fall back to `wl_shm`, and a shell that
  waits for dmabuf feedback before capturing — quickshell does — stays
  waiting), and `main_device` in that feedback is the renderer's own DRM
  render node rather than a guessed path, which is what makes a client's
  allocation land on the device the import will happen on. See
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

What it drives: the **primary plane, attempting the cursor and overlay
planes**, and -- for a fullscreen window covering the output -- handing the
primary to that window's own buffer (see the bullet above for exactly when).
The cursor plane is attempted
where the CRTC exposes one (the dev VM's virtio-gpu does: one `Cursor` plane
per `drm_info`) and silently not elsewhere; overlay planes ride along whole
from the same inventory (virtio exposes none: `overlay_planes=0`) and fall
back per frame the same way. A plane-assigned cursor is not in the
swapchain slot captures read, so captures draw it back in when they ask for
the pointer (see [Captures and the pointer](#captures-and-the-pointer)) --
and no window can ride an overlay yet (nothing is marked a scanout
candidate). A capture of a frame
that went direct forces one composite frame first, and a capture stream
keeps the output composited, so captures stay correct throughout --
watched working on the dev VM. Whether `apple,dcp`
exposes usable cursor or overlay planes is still unknown (no plane inventory exists
from the Asahi runs). Virtio's own footnote: its cursor plane needs the
session's `CURSOR_PLANE_HOTSPOT` cap to be enumerated at all (set before
`DrmDevice::new`, `ATOMIC` first, or commits fail unknown-plane), after
which it scans out live on virtio. The case for scanout is no longer
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

## Captures and the pointer

Whether a capture shows the pointer is decided by the capture, never by
the tier: an IPC `screenshot` draws it unless asked not to (`cursor:
false`, `scootctl screenshot --no-cursor`), and an
`ext-image-copy-capture-v1` session draws it exactly when it asked for
`paint_cursors` (`grim -c`). What each tier's own frame holds differs, and
the capture path reconciles the two:

| Tier | The frame a capture reads | Pointer asked for | Pointer not asked for |
| --- | --- | --- | --- |
| dumb (pixman) | always holds the cursor | nothing to do | cursor region re-rendered without it |
| GPU scanout, cursor on the cursor plane | lacks it | cursor region re-rendered with it | nothing to do |
| GPU scanout, cursor on an overlay plane | lacks it, and may hold a transparent hole there (an underlay) | re-rendered with it | re-rendered without it (fills the hole) |
| GPU scanout, plane refused this frame | holds it | nothing to do | re-rendered without it |
| `--headless`, `--nested` | never holds it (no cursor on screen) | re-rendered with it | nothing to do |

"Re-rendered" means the region under the cursor -- where it is now (when
asked for), where the frame drew it, and where a cursor on an overlay plane
may have left a hole -- is drawn again from the frame's own element list by
the session's own renderer, into a cursor-sized target, and written over
the captured copy. So the pointer in a capture is the same image, hotspot
and scale the screen shows, a capture can never end up with two of them
(the region is replaced, not blended over), and where a cursor rides an
overlay plane *under* the primary (an underlay: Smithay punches a
transparent hole in the primary above it) the hole is filled whether or not
the pointer was asked for, including at a position the pointer has since
left. Which elements the frame put on which plane is read from the
`DrmCompositor`'s own answer for that frame, not guessed from the plane
inventory. The underlay case has not been seen on hardware (virtio has no
overlay plane); it is pinned with a synthetic frame record.

A capture session that asked for the pointer is served a new frame when
only the pointer moves or changes image; one that did not is not, even on
the dumb tier, where moving the pointer redraws the frame (the redraw
counts as a cursor change, not a scene change). A stream whose region is
re-rendered every frame reuses one offscreen target and one pixel buffer
per output rather than allocating them per frame.

What it costs, measured on the dev VM (virtio-gpu, llvmpipe):

- **Where the frame already matches the request** -- the dumb tier by
  default, the GPU tier with `--no-cursor` -- nothing: no region, no
  gather.
- **pixman** draws a region in ~22 µs (harness); IPC screenshot latency on
  the dumb tier and `--headless` is within run-to-run spread of the
  previous build in both modes.
- **The GPU tier with its cursor on the plane and the pointer asked for**
  pays one region render and read-back per capture, 0.4-0.9 ms on
  llvmpipe. Compositor CPU per 40 IPC screenshots went from 26 to 42-46
  jiffies in the first measurement and 29 to 37-39 after the region's
  buffers were pooled. The screenshot's *wall latency* is not a stable
  measure of it: fresh-process medians moved +3.5 ms at first and +1.5 ms
  after pooling, while with and without the cursor interleaved in one
  process the pointer-requesting captures were the *faster* ones (p50
  10.9-11.3 ms against 12.4-13.0 ms). The latency tracks page faults from
  the capture path's whole-frame, multi-megabyte per-capture allocations
  (which both modes pay, and which are
  [a follow-up](backlog/core/capture-whole-frame-allocations.md)) and
  process history, not the region.
- **A capture stream asking for the pointer** on that tier delivers ~4%
  fewer frames (436-439 against 453-457 in 10 s; mean interval 22.8 ms
  against 22.0 ms) at the same compositor CPU; pooling the region's
  buffers did not change it, so it is the GLES region render itself.
- **A stream that did not ask for the pointer** is no longer re-served when
  only the pointer moves: on the dumb tier, over a still window with the
  pointer moving every 20 ms, 1 frame in 5 s at 12 jiffies, where it was
  209 frames at 60-61.

None of this has been measured on a real GPU.
