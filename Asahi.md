# Running scoot on Apple Silicon (Asahi Linux)

Three things in the backlog were blocked on an Asahi machine and cannot be
answered anywhere else. This is the runbook for answering them. Written
against `main` at `1b706b8`; the flags and bindings below are quoted from
`README.md` at that commit. The project was renamed `flexwm` → `scoot` on
2026-09-18 and every command here is the one to run *today*. Wherever a past
result is quoted against the commit it was captured at — the "Results so
far" section below, pinned to `f688ac9`, and the verification set at the end
— the old name stands, because that is what was typed and logged then.
Renaming those would falsify the record. **Two of the three were answered on 2026-09-18**
— see the results table and section immediately below; the runbook itself is
kept intact for re-runs on other Apple Silicon models. **Test 4 (added
later, and the largest of them) was answered on 2026-09-21**: its results sit
at the end of its own section, and `scripts/asahi-test4.sh` now runs the
whole of it as one command that writes its own report. **Tests 5–10 were
run on 2026-09-25** against `main` at `e1dce6f`. Their results sit at the
end of each test's own section, and the builds, machine and launch recipe
are under [Keys for the 2026-09-25 runs](#keys-for-the-2026-09-25-runs).

| What it unblocks | Priority | Needs a VT? | Status |
| --- | --- | --- | --- |
| [Ghostty fails at `scale = 1.5`](docs/backlog/resolved/ghostty-fails-at-1-5-done.md) | high → none | no | **RESOLVED, not reproducible** (2026-09-18) |
| [Does `--gpu` actually fix this machine](docs/backlog/resolved/tty-gpu-config-key-done.md) — the residual on a resolved entry | — | yes | **closed**: not needed, the search works (2026-09-18) |
| Issue #48's unconfirmed connector fallback, and multi-output phase E | — | yes | **answered** (2026-09-25 + 2026-09-28): both monitors driven at once on both tiers, lock and VT verified live, and a physical DP-1 unplug/replug removed and re-added its output; #48 was closed on unit-test evidence on 2026-09-27, and its fallback onto a *different* connector has since run live on the dev VM (Virtual-1 force-off / Virtual-2 force-on: `switch_crtc` to a new CRTC, modeset committed — [record](docs/backlog/resolved/tty-hotplug-confirmation-done.md)); the GPU-tier runtime add was answered by Test 12 |
| [Test 4: CPU vs GPU on a real GPU](docs/backlog/resolved/gpu-vs-cpu-measured-done.md) | high → none | yes | **ANSWERED** (2026-09-21): scanout comes up on the split topology and costs 4–5x less CPU |
| [Test 5: a fullscreen video scanned out directly](docs/backlog/resolved/gpu-primary-direct-format-gate-done.md) | medium | yes | **ANSWERED** (2026-09-25): mpv goes direct (~60% less compositor CPU) once it hides its pointer; `apple,dcp` has no cursor plane, so a visible pointer rules out any primary attempt ([ticket](docs/backlog/resolved/gpu-direct-blocked-by-composited-cursor-done.md), resolved by Test 17); planes: 1 primary, 1 overlay, 0 cursor |
| [Test 6: what the GLES tier advertises, and what GPU clients do with it](docs/backlog/resolved/gles-dmabuf-full-formats-done.md) (Part C: [the scanout tranche](docs/backlog/resolved/gpu-scanout-candidates-done.md)) | high | partly | **ANSWERED** (2026-09-25): 162 pairs (54 formats x tiled-compressed/tiled/`LINEAR`); clients pick compressed and Mesa follows the scanout tranche to `LINEAR`; mpv `dmabuf-wayland` cannot run (no hardware decoder: AVD firmware missing) |
| [Test 7: explicit sync on a real GPU](docs/backlog/resolved/linux-drm-syncobj-done.md) | medium | yes | **ANSWERED** (2026-09-25): offered on the display device; `vkcube` uses it |
| [Test 8: nested dma-buf presentation](docs/backlog/resolved/nested-dmabuf-present-done.md) | medium | Part B only | **ANSWERED** (2026-09-25): nested CPU 1.8–2x lower, host 2.2–3x lower, pixels identical, niri imports scoot's compressed buffers intact |
| [Test 9: scoot vs niri on a real GPU](docs/benchmarks.md) | medium | Part B only | **ANSWERED** (2026-09-25) except input latency: on the real panel scoot-gpu used the least total CPU for relayout and pointer motion (no frame counts there); pixman the most |
| Test 10: does the GPU driver keep an fd per imported plane | — | yes | **ANSWERED** (2026-09-25): yes, one (`copies_per_plane_per_output=1`). While a client runs scoot counts it exactly; after a client quits, one fd can linger until the next redraw, and one extra fd in one session is unexplained |
| Test 11: output remove/restore, reconnected modeset and multi-head mode change on the dumb tier (the #249 runbook, for real) | — | yes | **ANSWERED** (2026-09-26): virtual-pull force-off/on of DP-1 removed and re-added its output; both windows adopted then restored in order under a fresh id, positional binds reach the returned monitor; `--mode 1280x720` drives DP-1 at that mode with eDP-1 warned onto preferred; runtime mode switch stays refused by design (read-only `wlr-output-management`) |
| Test 12: GPU-tier runtime add, real-HPD remove/restore and multi-head mode change | — | yes | **ANSWERED** (2026-09-26): powering on a sleeping monitor adds a GPU-scanout head in ~50 ms; a real unplug adopts its window and the replug restores it under a fresh id with binds following; `--mode 1280x720` drives DP-1 at that mode on the GPU tier and a replug keeps it; adopted windows feel like they disappear ([UX ticket](docs/backlog/core/unplug-adopted-windows-invisible.md)) |
| [Test 17: the drawn cursor on an overlay plane](docs/backlog/resolved/gpu-direct-blocked-by-composited-cursor-done.md) | medium | yes | **ANSWERED** (2026-10-06): DCP takes scoot's drawn cursor on overlay plane 45 while fullscreen mpv stays direct on the primary with the pointer visible: 13-15 jiffies per 10 s against 22-33, and pointer motion over a desktop 11 against 17-19 |

## Results so far (run 2026-09-18, `main` at `f688ac9`)

The machine is an Apple M2 (`apple,t8112`, j413), NixOS aarch64. The DRM
split is exactly as this document predicted: `card1` → `asahi` (render, no
KMS), `card2` → `apple-drm` (display, owns `eDP-1`), `renderD128` → render
node.

- **Test 2 — answered, no VT needed after all.** scoot was *already the
  live desktop session* (greetd → `flexwm --tty -- noctalia`), running with
  no `--gpu` flag and no `[tty] gpu` key, with `/dev/dri/card2` as its only
  open DRM fd. The automatic search works on Apple Silicon; `--gpu` is a
  convenience here, not a requirement. Read-only `/proc/<pid>/fd` and
  `flexwm msg outputs` gave this without restarting anything — and the
  canonical log lines this section asks for were captured on the next reboot,
  once the session started logging to a file:

  ```
  WARN …tty::gpu: drm: device unusable path=/dev/dri/card1
       reason=has no usable KMS pipeline -- loading its DRM resources failed
       (Operation not supported (os error 95))
  INFO …tty: drm: driving this device path=/dev/dri/card2 connector=eDP-1
       width=2560 height=1600
  ```

  Rejected candidate with its reason, then the winner — exactly the shape
  "What to look for in the log" below describes.
- **Test 1 — RESOLVED, does not reproduce anywhere.** Not under `--headless`
  at either scale on the real AGX GPU (dmabuf, `renderD128`), with zero
  EGL/GL and zero protocol errors, on `f688ac9` and on the older build the
  session happened to be running at the time — which refutes both hypotheses
  this runbook named, Ghostty version and the GPU/GL path. And **not under
  `--tty` at 1.5 on the real `eDP-1` either**: the machine was rebuilt with
  `scale = 1.5` and rebooted into it, and Ghostty mapped and was used
  interactively at logical 1707×1067. That was the original reported
  configuration and the last one capable of testing this, so the entry is
  archived as not reproducible with the cause never captured.
- **Test 3 — partial (2026-09-25).** An external monitor now works (DP-1
  over the `fairydust` kernel). Since multi-output phase E, `--tty` drives
  it *and* the panel at once; both tiers, the lock and VT switching all
  verified live, and a physical unplug and replug of DP-1 removed and then
  re-added its output (results below). #48's fallback onto a *different*
  connector is still unreachable here, because the panel cannot be
  unplugged.

Everything below is the runbook as written, plus traps found while running
it. Re-running any of it is still worthwhile on a different Asahi model or
a newer Ghostty.

## Why this machine

Two of the three are Apple-Silicon-specific by construction, not by accident:

- **The DRM split.** The usual "the GPU whose PCI parent has `boot_vga=1`"
  heuristic assumes the 3D GPU and the display controller are one DRM device.
  Under Asahi they are not: `asahi`/AGX owns the render node, `apple,dcp`
  owns the CRTCs and connectors, and there is no PCI GPU or VGA BIOS for the
  rule to match. scoot has a fallback that tries every DRM device on the
  seat, and `--gpu PATH` to skip the search entirely. Both were built and
  unit-tested long before either had run on Apple Silicon; **the fallback
  has now run there and is correct** (2026-09-18), and `--gpu` remains
  unexercised on this topology because nothing required it.
- **The GPU/GL path.** The dev VM falls back to software rendering (Mesa
  `swrast`/`zink` failures throughout its logs). The leading hypothesis for
  the Ghostty failure is a client-side EGL/GL problem at fractional scale,
  which software rendering structurally cannot reproduce.

## Safety

Nothing here can strand you out of your own desktop, and it is ordered so the
risky part comes last:

- **Test 1 needs no VT at all.** It runs `--headless` inside your normal
  session. Ghostty still uses the real Asahi GL stack, which is the part
  under suspicion, so the backend costs nothing here. *(That reasoning held
  and did its job — the GL stack was exercised over dmabuf and the GL
  hypothesis was refuted. Refuting it did promote `--headless`-vs-`--tty` to
  the largest remaining variable, so that was then tested too, as a real
  session; see Test 1's result note. Nothing is left outstanding here.)*
- **Test 2 may need no VT either** — if scoot is already your desktop
  session, the read-only shortcut in that section answers it from inside the
  session you are in. Check before booking a VT.
- **Tests 2 and 3 take a VT**, absent that shortcut. Under `--tty`, scoot binds `Ctrl+Alt+F1`
  through `Ctrl+Alt+F12` to VT switching. These are added *after* the config
  file loads and always win over a colliding config bind (logging a warning
  naming what they displaced), precisely so this recovery path cannot be lost
  to a typo. `Ctrl+Alt+F<your desktop's VT>` gets you back.
- `Super+Shift+e` quits scoot. (Deliberately not `Super+Shift+q` — one
  slipped Shift from `Super+q`, close window.)

## Build

```sh
git clone https://github.com/scoot-sh/scoot && cd scoot
nix build                      # -> ./result/bin/scoot
nix build .#scoot-gpu          # -> ./result-scoot-gpu/bin/scoot, with the gpu-scanout tier (Test 4)
mkdir -p /tmp/fx
```

Without Nix you need a Rust toolchain plus the compositor's system
dependencies (libseat, libinput, libxkbcommon, pixman, udev and friends);
`nix develop` provides them on a machine that has Nix, and the flake is the
only dependency set this repo maintains.

---

## Test 1 — Ghostty at `[output] scale = 1.5`

**The question.** On this machine, Ghostty loads at `scale = 2.0` and does
not load at `1.5`; `foot` works at both. The integer
`wl_surface.preferred_buffer_scale` companion event was missing and has since
been fixed (PR #31) — but GTK4 ignores that event whenever a
`wp_fractional_scale_v1` object exists, which scoot always advertises, so
**the fix probably does not explain the symptom and the root cause is
unknown.** A dev-VM reproduction attempt failed to reproduce at all: Ghostty
1.3.1 mapped and rendered real content at both scales.

So the job here is not to confirm a fix. It is to get the one piece of
evidence that tells us whether this is a Ghostty-version thing or a GPU/GL
thing — a client-side EGL/GL error would mean scoot's scaling is not the
defect.

```sh
printf '[output]\nscale = 1.5\n' > /tmp/fx/s15.toml
printf '[output]\nscale = 2.0\n' > /tmp/fx/s20.toml

ghostty --version                   # record this
```

Run at 1.5:

```sh
SCOOT_SOCKET=/tmp/fx/t15.sock WAYLAND_DEBUG=1 \
  ./result/bin/scoot --headless --width 1280 --height 800 \
  --config /tmp/fx/s15.toml \
  -- ghostty --gtk-single-instance=false -e sh -c "sleep 60" \
  > /tmp/fx/ghostty-1.5.log 2>&1 &

sleep 12
SCOOT_SOCKET=/tmp/fx/t15.sock ./result/bin/scoot msg windows
SCOOT_SOCKET=/tmp/fx/t15.sock ./result/bin/scoot msg screenshot --out /tmp/fx/g15.png
```

Then the same with `--config /tmp/fx/s20.toml` and a *different* socket
(`/tmp/fx/t20.sock`), writing `/tmp/fx/ghostty-2.0.log` and `/tmp/fx/g20.png`.

**Three traps. The first two produced false failures during the original
investigation; the third was found running this document on 2026-09-18. All
are recorded so they are not repeated:**

- **Set `SCOOT_SOCKET` per instance if scoot might already be running.**
  `socket_path()` defaults to `$XDG_RUNTIME_DIR/scoot.sock`, so on a machine
  where scoot *is the desktop session* — which is how this one is set up —
  a bare `scoot msg windows` talks to **the live session, not the test
  instance**. That reads the real desktop's window list as if it were
  Ghostty's and screenshots the real screen, which can look like either a
  pass or a failure depending on what happens to be open. This is the same
  class of harness artifact as the two below, and the most dangerous,
  because the output looks entirely plausible.

- **Don't shorten the sleep.** GTK4/Ghostty need roughly 7 s to map under
  software GL; `foot` needs ~0.5 s. A 4–5 s wait reads as "no windows
  mapped" when the client was merely slow.
- **Don't use a command that exits.** `ghostty -e true` opens and closes the
  window in a fraction of a second, so polling after it is gone reads as
  "doesn't load" at *every* scale. Hence `-e sh -c "sleep 60"`.

**What the result means.** `msg windows` listing a Ghostty window, and a
screenshot with drawn content rather than a blank backdrop, means it loaded.
If 1.5 fails here, the log is the payload: the lines after the first
`wl_surface.commit`, and any EGL/GL error. A Ghostty-side EGL/GL error means
the defect is not scoot's scaling and the fix (if any) is Ghostty-side or a
workaround (`scale = 2.0`, or Ghostty's own `window-scale`/font-size
setting). A Wayland protocol error means the opposite, and is a scoot bug.

**Don't reach for `--nested` as a middle ground.** It looks like a way to
test fractional scale against a real compositing backend without taking the
VT, and it is not — scoot refuses output scaling there by design, because
the host compositor owns the window's scale:

```
WARN flexwm::compositor: output scaling is not supported under --nested
     (the host compositor owns the window's scale); using 1.0 configured=1.5
```

The client then sees `preferred_scale(120)` regardless of what the config
says, so the run cannot say anything about 1.5. Tried and confirmed
2026-09-18. `--headless` and `--tty` are the only two backends that can test
this at all.

**Result on this machine (2026-09-18): did not reproduce.** Ghostty 1.3.1
mapped and drew real content at *both* scales — 409×510 logical at 1.5 and
360×376 at 2.0, matching the dev VM's numbers exactly — with correct
`preferred_scale` (180 / 240), zero protocol errors and zero EGL/GL errors,
over dmabuf on the real AGX GPU. Both hypotheses this runbook was built
around are therefore refuted.

**And then the last suspect, `--tty` at 1.5, was tested too — it also works.**
The machine was rebuilt with `scale = 1.5` and rebooted into it: Ghostty
mapped at logical 1707x1067 on the real `eDP-1` and was used interactively.
That was the original reported configuration, so the entry is closed as NOT
reproducible. See the backlog entry for the full table.

---

## Test 2 — which DRM device `--tty` drives

**The question.** Does the automatic search find the display controller on
Apple Silicon, and if not, does `--gpu` fix it?

> **Answered 2026-09-18: yes, the search works; `--gpu` is not needed.**
> And it needed no VT, because scoot was already the live session. **Check
> that first** — if scoot is already running as your desktop, the cheapest
> and most authoritative version of this test is read-only, on the session
> you are already in:
>
> ```sh
> pgrep -ax scoot                       # was --gpu passed?
> # Pick the --tty one by name: a test instance or a stray `scoot msg`
> # makes `$(pgrep -x scoot)` expand to several pids and the path garbage.
> for p in $(pgrep -f 'scoot .*--tty'); do
>   echo "pid $p:"; ls -l /proc/$p/fd | grep dri
> done
> scoot msg outputs                     # which connector?
> ```
>
> On this machine that gave `--tty` with no `--gpu`, one DRM fd
> (`/dev/dri/card2`, the `apple-drm` display controller), and `eDP-1` —
> i.e. the fallback rejected the `asahi` render node unattended, in a real
> daily-driven session. That is better evidence than a test run, and it
> costs nothing. The steps below are for the case where scoot is *not*
> already up, or where you want the log lines themselves.
>
> Note the log lines are the one thing this shortcut cannot give you: a
> greetd-launched session's stderr goes to `/dev/tty1` uncaptured, and
> getting it back means restarting your desktop. Add a redirect to the
> session command if you need it.

This was originally the gate on the `[tty] gpu` config key. That key shipped
on 2026-09-18 without waiting, on the reasoning that the gate conflated two
separable things: the key itself (parse, validate, plumb — fully testable
without split-GPU hardware) and this confirmation (which needs your machine).
So what remains is the question above on its own, recorded as the residual on
`docs/backlog/resolved/tty-gpu-config-key-done.md`. If a device path turns
out to be the wrong *shape* of persistent setting for this hardware, the key
is small enough to reshape — this test is what would show that.

First, from your normal desktop session, see what the split looks like:

```sh
ls -l /dev/dri/
for c in /sys/class/drm/card*; do
  printf '%s -> %s\n' "$c" "$(readlink -f "$c/device/driver")"
done
```

Then switch to a free VT (**Ctrl+Alt+F3**), log in, and run the **automatic**
search first — whether the fallback works here is the actual open question,
and `--gpu` would mask the answer:

```sh
cd ~/scoot
RUST_LOG=info ./result/bin/scoot --tty -- foot 2>&1 | tee /tmp/fx/tty-auto.log
```

If that fails, name the display controller explicitly (adjust the path from
the listing above):

```sh
RUST_LOG=info ./result/bin/scoot --tty --gpu /dev/dri/card1 -- foot 2>&1 \
  | tee /tmp/fx/tty-gpu.log
```

**What to look for in the log.**

- `drm: driving this device` — the winner, with its path. This is the line
  that answers the question.
- `drm: device unusable` — each rejected candidate and what it said. On a
  working fallback you should see the render node rejected and the display
  controller accepted.
- `Operation not supported (os error 95)` — the known failure signature of
  loading KMS resources from the render-only device.
- `Unable to become drm master, assuming unprivileged mode` — expected and
  harmless on every non-root `--tty` run on a modern kernel; not a failure.
- With `--gpu`, possibly `the chosen device is not in udev's list for this
  seat, so display changes on it will not be noticed`. That is a real
  tradeoff, not an error: hotplug is followed only for devices udev lists as
  GPUs on this seat, so naming one outside that list costs Test 3.

`--gpu PATH` replaces the search entirely — exactly that device, no fallback
— so a wrong path is a clean startup error naming the device and what
failed, never a silent fall back to something else. Naming a device skips the
*search*, not the checks: it still has to open through the session and pass
the same KMS probe every automatic candidate does.

**Once you know the right device, stop retyping it.** `~/.config/scoot/config.toml`:

```toml
[tty]
gpu = "/dev/dri/card1"
```

`--gpu` wins when both name one. Like the flag, this key is fail-closed
where every other config field degrades gracefully: a wrong path is a
startup error naming the key, because falling back would mean silently
driving a device the config explicitly ruled out. An empty `gpu = ""` is a
startup error on every backend; a set non-empty value is ignored with a
warning outside `--tty`.

---

## Test 3 — connector fallback on a real unplug (issue #48)

**Only if Test 2 gets a session up**, and only if `--gpu` was not needed (or
was needed but did not print the udev-list warning — hotplug is not followed
otherwise). Test 2 cleared both conditions on 2026-09-18.

> **Not yet run — blocked on hardware, not on software.** This machine
> currently exposes exactly one connector:
>
> ```
> /sys/class/drm/card2-eDP-1: status=connected enabled=enabled
> ```
>
> There is no second connector to fall back *to*, so the unplug this test
> describes has no defined outcome to observe. It needs an external display
> on USB-C (DP alt mode). Check for a second `card2-*` entry after plugging
> one in — if none appears, that is an Asahi DCP limitation to establish
> before blaming scoot.
>
> Second constraint: it needs the `--tty` seat, which only one process can
> hold. If scoot is already your desktop session, that session *is* the
> one to test — don't start a second `--tty` instance, it will be refused.

Issue #48 was the last open issue in the repo (closed 2026-09-27 on unit-test
evidence; the two unrun paths below were tracked in the hotplug-confirmation
backlog entry and both have since run live
on the dev VM — see
`docs/backlog/resolved/tty-hotplug-confirmation-done.md`). PR #51 implemented DRM hotplug
and deliberately said `Refs #48`, not `Closes`: two of its four code paths
have never run on real hardware, because nothing in a QEMU/virtio-gpu VM can
change a connector's mode list at runtime (EDID override, off/detect cycles
and `vkms` were all tried and documented as dead ends). A laptop with an
external display is the only thing that can produce one of them.

With scoot running under `--tty` and an external display plugged in, unplug
the one scoot is currently driving. Expected: it falls back to the other
connector and keeps running, rather than black-screening until restart.
Capture the log the same way (`tee`), and note which connector it started on
(`scoot msg outputs` names it — `eDP-1`, `HDMI-A-1` and so on).

This exercises `Plan::NewConnector`, which is the only path that runs
`set_pending` against real DRM. Plugging a *second* display in without
unplugging the first should keep the session where it is: this is a
one-output backend, and staying on the panel the user is looking at is
deliberate.

### Results, 2026-09-25 (partial; scoot `e1dce6f` build, kernel `fairydust` 7.1.13)

An external 1920x1080 monitor on the M2 Air's front-left USB-C port is now
driven by Linux, via the `fairydust` Asahi kernel (USB-C DisplayPort alt
mode; the machine's config pins AsahiLinux/linux `fairydust`
`ce9f2eba`). It appears as `card2-DP-1` and connects after being plugged in
*after* boot (plugged in across the reboot, it read `disconnected` until
replugged); the DCP then set `1920x1080@60`.

- **Started with both connected**, `scoot --tty` (pixman) chose `eDP-1`
  (`drm: driving this device ... connector=eDP-1`), as designed.
- **Unplugging and replugging the external monitor** while scoot drove the
  panel: scoot received both udev change events (13 s apart) and stayed on
  `eDP-1`, running, with no re-modeset on the panel — the
  second-display-plugged-in case this test describes ("keep the session
  where it is") holds on real hardware.
- **Not reachable here:** the connector-*loss* fallback (`Plan::NewConnector`)
  needs scoot driving the connector that disappears. scoot always opens on
  the panel, which cannot be unplugged, and with this kernel `DP-1` kept
  reading `connected` in sysfs across the unplug (an alt-mode HPD quirk of
  the experimental kernel), so no connector ever went away from scoot's
  point of view. The new-mode-list-on-the-same-connector path is likewise
  not exercised. Both stay open on #48.

### Results, 2026-09-25 (later): both monitors at once, multi-output phase E

scoot built from the phase E branch: E1 `b782b06`, and E2 `2bd7d47` for the
final product code, via `nix build .#scoot-gpu`. It ran on VT 2 through
`~/fx/e-live.sh` (seatd plus openvt as in the local recipe), with
`[output] scale = 1.5`. The DP-1 monitor had been plugged in after boot.

- **Both connectors driven.** One `drm: driving this device` line each:
  `eDP-1` on CRTC 50 at 2560x1600 and `DP-1` on CRTC 68 at 1920x1080. That
  matches `drm_info`, where each encoder reaches exactly one CRTC.
  `scoot msg outputs` names `eDP-1` (logical 1707x1067 at 0,0) and `DP-1`
  (1280x720 at 1707,0).
- **Windows and pointer.** Two `foot` windows, the second carried to DP-1
  with `Super+Shift+period`. The IPC screenshots (`screenshot --output 1`,
  `--output 2`) each show their own window only, and the pointer moved onto
  DP-1 is drawn there.
- **VT switch.** `chvt 1` then `chvt 2`: `session paused`, `session
  activated`, then a full modeset on each CRTC, and both screens come back.
  The re-probe of both connectors on activation took about 240 ms.
- **Lock.** swaylock (`-c 7a1fa0`) covers both screens (both screenshots
  are that color). `session lock confirmed: every output's blanked frame
  reached scanout` came about 60 ms after `locking the session`.
- **Both tiers.** Pixman/dumb buffers by default. The GPU scanout tier with
  `--renderer gles` shows `scanout="gpu"` on both heads. For the GPU tier:
  the booted generation has no `/run/opengl-driver`, so Mesa 26.2.2's paths
  were passed by environment (`GBM_BACKENDS_PATH`,
  `__EGL_VENDOR_LIBRARY_DIRS`, `LIBGL_DRIVERS_PATH`). No system change was
  made.
- **CPU** (utime+stime jiffies, 30 s windows, a `foot` printing every
  20 ms):

  | Tier | Idle | Both screens damaged | eDP-1 only |
  |---|---|---|---|
  | pixman | 0.00% | 37.27% | 36.20% |
  | GPU | 0.00% | 13.73% | 9.27% |

  Single-output `main` (`0785420`, eDP-1 only) with the same workload
  measured 35.20% and 35.67% on pixman (one full-width window), and 9.43%
  and 8.63% on the GPU tier (the same two-column layout).
- **Pacing.** 705 page flips in 10 s across the two heads with both damaged,
  about 35 per second each, never more than one per CRTC vblank.
- **Correction.** An earlier version of this section said the unplug was
  not observable here. It was wrong for this run, as the next section
  shows: the morning's quicker unplug never read `disconnected`, but this
  one did.

### Results, 2026-09-25 17:59Z: a physical DP-1 unplug and replug

- **Setup.** scoot `2bd7d47` (`nix build .#scoot-gpu`, pixman tier),
  `~/fx/replug-start.sh`: a foot window on each monitor, a udev monitor, and
  a 0.5 s poll of `card2-DP-1/status`. The user pulled DP-1 and plugged it
  back into the front-left port. Collected with `~/fx/replug-collect.sh`.
  Raw files are in `~/fx/replug/`: `scoot.log`, `udev.log` (6 events),
  `dp-status.log`, `outputs-{before,after}.json`,
  `windows-{before,after}.txt` and `after-out{1,3}.png`.
- **Kernel.** `dp-status.log` reads 22 × `connected`, then 33 ×
  `disconnected` (about 16 s), then 35 × `connected`. HPD loss *did* reach
  userspace this time.
- **scoot** (`scoot.log`):
  - 17:59:17.39 `drm: this connector went away connector=DP-1`, then
    `a display went away; removing its output output=2`.
  - 17:59:34.08 `drm: driving a newly connected display connector=DP-1
    crtc=crtc::Handle(68) width=1920 height=1080 scanout="dumb"`, then
    `added an output for it ... output=3`, then `drm: modeset (full
    commit)`.
  - eDP-1's head was left alone throughout (`hotplug changed nothing this
    head is driving connector=eDP-1`).
- **After.**
  - `outputs` lists `eDP-1` (id 1) and `DP-1` (id **3**, at x=1707).
  - `windows` puts both on output 1, and the returned monitor shows an
    empty workspace. This was the reconnect gap, since resolved by PR #249
    (`docs/backlog/resolved/output-reconnect-restore-done.md`).
- **Found by review of this run.**
  - foot logged `unmapped from unknown output`: the window's
    `wl_surface.leave` went out after the output's `global_remove`. Fixed
    in PR #247's review round (`remove_output` refreshes the `Space` before
    withdrawing the global).
- **Still unexecuted on hardware.**
  - A GPU-tier runtime add (this run was dumb buffers). *Since run: Test 12.*
  - #48's `MoveTo` fallback.
  - A mode change with several heads. *Since run: Test 11 (dumb tier), Test 12 (GPU tier).*

---

## Test 4 — CPU vs GPU rendering, on a GPU that is actually a GPU

**This is the only machine that can answer it.** Every GPU number the
project has is llvmpipe's — a software rasteriser — so nothing measured so
far says anything about real GPU performance. What the VM *did* establish is
the shape: GLES rendering offscreen and reading each frame back cost
**18–31x** pixman, and GPU scanout on the same rasteriser costs **~1.5x**.
(This paragraph said "17–32x" until 2026-09-21. That pair divides out of
round 1 of the stage-2 bench — 1.978ms/62.2µs and 2.384ms/138.3µs — and
`docs/roadmap/06-gpu-pipeline.md:872` had already retired those two figures
for "flattering one scene and penalising the other for no reason beyond
round order". The medians, 64.4µs and 131.0µs, give 30.7x and 18.2x.)
Removing the read-back closed nearly the whole gap, which means the
read-back was the dominant cost rather than the rasterising. On real
hardware, where rasterising stops being a CPU's problem, scanout should win
— but that sentence is an extrapolation, and this test is what turns it into
a number.

Needs a `gpu-scanout` build (`nix build .#scoot-gpu`, or
`cargo build -p scoot --features gpu-scanout` — see `docs/tty.md`), and the `--tty`
seat.

**Expect correctness questions before performance ones.** Scanout has never
run on a real GPU, drives the **primary plane only**, and this machine is
the split render/display case — AGX (`card1`) has the render node, `apple,dcp`
(`card2`) owns the connectors — which the design allows for but nothing has
exercised. If it does not come up, that result is worth more than any
timing.

### The metrics that matter, in order

**FPS is the wrong headline number for a compositor.** scoot renders on
damage, not on a clock, so "frames per second" mostly measures how much the
clients asked for. The numbers that decide whether GPU rendering is worth
using on a laptop are:

1. **Idle CPU.** A compositor that is fast but busy is worse on battery than
   one slightly slower that sleeps. Sample jiffies over a fixed window with
   nothing moving:
   ```sh
   pid=$(pgrep -x scoot); read -r a b < <(awk '{print $14, $15}' /proc/$pid/stat)
   sleep 10
   read -r c d < <(awk '{print $14, $15}' /proc/$pid/stat)
   echo "jiffies over 10s: $(( (c-a) + (d-b) ))"
   ```
2. **Frame cost under damage.** One window scrolling, then several. This is
   where scanout should show its advantage, because it is the read-back it
   deletes.
3. **Memory.** `grep VmRSS /proc/$pid/status` — a GBM swapchain is not free,
   and three buffers at panel resolution is real memory.
4. **Power**, if you care about the laptop: GPU scanout may cut CPU wakeups
   and raise GPU draw. Net effect is genuinely unknown and is the most
   interesting result here.

### Method

Alternate the two tiers rather than running each once. The project has
already been burned by this: a single A/B pair once read as a **70%
regression** that was pure noise, and it took eight alternating rounds to
show the medians were 2.4µs apart against a 19µs spread. Same scene, same
clients, same duration, at least four rounds each, and report medians with
the spread — not a best-of.

```sh
# tier A: CPU, the default
scoot --tty -- foot
# tier B: GPU scanout
scoot --tty --renderer gles -- foot        # gpu-scanout build
```

Confirm which tier you are actually on before trusting a number. The startup
line names the tier either way — `scanout="gpu"` or `scanout="dumb"`, from
`presenter.tier()` at `tty/mod.rs:501` — and `--renderer gles` without the
feature silently keeps pixman with a warning.

**A missing `scanout=` field means your grep failed, not that you are on the
dumb tier.** This instruction used to say the dumb tier was recognisable by
the *absence* of the field, which is wrong and wrong in the dangerous
direction: it is exactly what a log full of ANSI escapes looks like, and that
misreading is what cost the 2026-09-21 run its first analysis (see the traps
at the end of this section). Strip escapes before grepping an older log:
`sed 's/\x1b\[[0-9;]*m//g'`.

### ANSWERED, 2026-09-21 — scanout comes up, and it wins by 4–5x

Run it with `scripts/asahi-test4.sh`, which is the whole of this test as one
command: it drives `scripts/tty-tier-bench.sh` for the alternating rounds and
prints the analysis into `$OUT/test4-report.txt`. Two runs: four rounds (raw
in `/tmp/scoot-tier-bench`) then two (raw in `/tmp/scoot-asahi-test4`); the
second exists because the first left two gaps, named below.

**What the numbers are keyed to.** Both runs measured *byte-identical
binaries* — `/nix/store/mf5nm9…-scoot-0.1.0` and
`/nix/store/b1kkl6s…-scoot-gpu-0.1.0`, both `nix build`s of `main` at
`499083b`, which is also why the two runs are comparable with each other.
The harness trees differ (`650a187`, then `52672b8`) because the harness was
fixed between runs; the compositor under test was not rebuilt and did not
change. `environment.txt` in each output directory records both, and the
store paths are the ones that matter.

*Where run 1's raw file went.* `/tmp/scoot-tier-bench`'s `summary.tsv` and
`environment.txt` were destroyed after the fact by
`OUT=/tmp/scoot-tier-bench scripts/asahi-test4.sh` — intended as "re-read the
old numbers", which at the time re-ran the benchmark into the evidence
directory, failed every round on the busy seat, and left four `came_up=no`
rows where four rounds of measurements had been. Every screenshot, every
power sample and the `.outputs`/`.windows` dumps survive, as do the **r3–r4**
logs (the clobbering run defaulted to `ROUNDS=2`, so rounds 1 and 2 hold
seat-failure output from 23:10 instead). **The rows themselves are
recovered** and remain checkable: review held a verbatim capture, transcribed
in
[`docs/backlog/resolved/gpu-vs-cpu-measured-done.md`](docs/backlog/resolved/gpu-vs-cpu-measured-done.md)
under "Run 1's raw rows, recovered", and because `idle_uW_mean` was computed
from the surviving `.power` files that whole column re-derives from disk
today — 8 of 8 exact. What is gone is the original *file*, not the numbers,
and run 1's store paths are quoted from that capture while run 2's are still
on disk and match. Run 2 is complete and intact and on its own establishes
the correctness result, the ratios and every power figure. Both scripts now
refuse to measure into a directory that already holds a run; `ANALYSE_ONLY=1`
re-reads one.

**Correctness first, as this section asked for.** It comes up:

```
drm: driving this device path=/dev/dri/card2 connector=eDP-1
     width=2560 height=1600 scanout="gpu"
```

Mode set atomically on `crtc::Handle(45)`/`plane::Handle(35)`,
`2560x1600@60` (`vrefresh: 60` in the mode it chose), logical 1707×1067 at
`scale = 1.5`. **The split render/display topology needs no special
handling**: one `GbmDevice` serving allocator, exporter and EGL is enough
even though AGX owns `renderD128` and `apple,dcp` owns `card2`'s CRTCs. The
separable-construction fallback `docs/roadmap/06-gpu-pipeline.md` reserved
for this case is not required. Both tiers logged an identical set of four
warnings (`card1` rejected for no KMS pipeline, unprivileged DRM master,
gamma size 0, stale mode blob) — nothing silently fell back to the dumb
tier.

**And the frames are right.** Comparing the pinned-scene captures — two
freshly mapped `foot` windows, pointer parked at 1280,800 — the two tiers
are identical across all 4.096M pixels *except* an 18×34 box at physical
1911,1184, with a maximum channel delta of 3/255. That box is the cursor:
1280,800 × 1.5 = physical 1920,1200, the crop holds 118 distinct colors
(an antialiased glyph, not a flat block), and the same crop taken anywhere
else differs by exactly zero. The control matters as much as the result —
the same tier captured in different rounds gives `AE = 0`, byte-identical,
so the scene really is pinned and the 18×34 box is the whole difference.
For scale, a whole-frame one-least-significant-bit difference at this
resolution measures `AE ≈ 4016` if one channel of four differs everywhere,
or `AE ≈ 12044` if all three color channels do (measured, not derived — add
`1/255` to R, G and B and compare). Observed: **1.003**, four orders of
magnitude below either. The `AE = 0` control is the stronger half of this:
the captures are byte-identical `md5`-wise within a tier.

**Performance, per damage event, medians with spread:**

| scene | dumb + pixman | gpu scanout | |
| --- | --- | --- | --- |
| large-damage motion | 0.4553 j/ev (0.4430–0.4613) | **0.0908 j/ev** (0.0872–0.0972) | **4.8–5.1x cheaper** |
| full relayout | 3.358 j/ev (3.32–3.43) | **0.796 j/ev** (0.790–0.814) | **4.2–4.3x cheaper** |

All six rounds pooled. The two ranges are the runs' **ratios of medians** —
5.09x and 4.79x motion, 4.20x and 4.30x relayout. As a share of one core over
the fixed windows: motion cost **20.8%** on the dumb tier against **4.3%** on
scanout; relayout **52.0%** against **14.0%**.

**Round-to-round spread, since the ratio is large enough not to need
flattering.** Against each tier's own median: motion −2.7%/+1.3% (dumb) and
−3.9%/+7.1% (gpu); relayout −1.2%/+2.2% (dumb) and −0.8%/+2.2% (gpu). The gpu
motion column spans 11% min-to-max; the effect being measured is 400%, so it
survives the honest number comfortably.

**The other three metrics this section asked for, in its own order:**

1. **Idle CPU: zero jiffies over 10s on both tiers**, every round of both
   runs. Neither wakes when nothing moves, so the "fast but busy" trade this
   section worried about does not exist here. Idle power is
   indistinguishable: medians 5.510 W (dumb) and 5.505 W (gpu), whole-system,
   with per-round means spanning 5.411–5.527 and 5.481–5.533. Note that
   0.12 W within-tier idle band when reading the damage-power figure below —
   it is over half the difference that figure reports.
2. **Frame cost under damage:** above.
3. **Memory: +7 to +16 MB RSS** for the GBM swapchain (medians: 76.2→92.4 MB
   in run 1, 87.3→94.8 MB in run 2 — the baseline itself moves, so this is a
   range, not a constant). The one column the dumb tier wins.
4. **Power — the one this section called "genuinely unknown".** On this
   hardware it does **not** trade CPU wakeups for GPU draw: whole-system
   draw under damage is **lower** on scanout, by **0.22 W** under motion
   (6.26→6.04) and **0.25 W** under relayout (6.36→6.11), roughly 3.5% of
   total system draw on battery. **Read that as a direction, not a precise
   quantity**: it rests on run 2 alone (run 1 sampled power at idle only),
   so n = 2 rounds per tier at 12 samples per motion scene and 8–9 per relayout
   scene, and the within-tier idle band above is half its size. Both signs
   agree across both scenes and both rounds, which is the part worth
   trusting.

**What this does not say.** The motion scene is *large-bbox* damage: the
injected pointer path jumps across ~900×600 logical pixels, so each frame's
damage bounding box is big. The 4.8–5.1x is for substantial damage; a small
cursor-rect move is a different measurement and was not made. Both figures
are one panel at one resolution on one machine, and scanout remains
primary-plane only — cursor and overlay planes are phase 2 of
`docs/backlog/resolved/gpu-scanout-planes-done.md`, which this unblocks.
(That was the state at the time of this run; cursor and overlay steps have
since landed — see the resolved record. On this machine the cursor step
cannot engage: `apple,dcp` exposes no cursor plane. Its one overlay does
not help, because at this Smithay rev no overlay on any hardware can take
scoot's drawn cursor, which is a memory buffer. See Test 5's results.)

**Corrections to earlier drafts of this section**, collected here rather than
left inline, because a result is hard to read with its own revision history
threaded through it. Each was found by review re-deriving the number instead
of checking the prose:

- *"Every round agrees within ±1% per tier"* overstated the tightness by
  about 5x and contradicted the min–max column printed directly above it.
  The real spreads are in the spread paragraph.
- *Motion ratios of 5.07x and 4.18x.* 5.07 is the median of the four
  per-round *ratios* — a third statistic, inconsistent with the medians
  quoted beside it, which divide to 5.09 — and 4.18 reproduces under no
  convention at all (it is 4.20). Hence the table now names its convention.
- *Run 1's dumb RSS as 75.5 MB*, which is that round set's **mean** in a
  section that says medians throughout. The median is 76.2, making the delta
  +16.2 and the published range `+7 to +16 MB`.
- *A relayout row of `3.34 (3.32–3.43)` and `0.797 (0.790–0.802)`.* The
  medians were each one run's alone, and `0.802` was run 2's *maximum*: the
  pooled maximum is `0.8136`, so a 1.5%-wide band was published where the
  real one is 3.0%. A hidden spread in a table headed "medians with spread".
- *A gpu motion median of `0.090`*, which truncates `0.09078` rather than
  rounding it, in the flattering direction.
- *"17–32x" for the llvmpipe read-back penalty* (above, in this test's
  preamble). That pair divides out of round 1 of the stage-2 bench, which
  `docs/roadmap/06-gpu-pipeline.md:872` had already retired for flattering
  one scene and penalising the other; the medians give 18–31x.

Two harness traps found while running it, in the tradition of the ones above.
First, the compositor colored its log output unconditionally, so
`scanout="gpu"` in a redirected log is really
`scanout\x1b[0m\x1b[2m=\x1b[0m"gpu"` — the harness read the tier as absent on
a run where it had come up, i.e. it mis-reported the single most important
field in this test. Every `tee` capture this document recommends was
affected; fixed in `225719e`, gated on `IsTerminal`. Second, the dev VM's
"300 unpaced pointer moves" cannot be copied here: an IPC round trip costs
~0.8 ms on this machine against the VM's ~11 ms, so an unpaced burst arrives
~20x faster than the panel refreshes and is correctly coalesced away (3000
moves produced 16 jiffies). Damage is now driven at a fixed rate below the
refresh rate for a fixed wall-clock window.

## Test 5 — does a fullscreen video go direct on a real GPU

Why: a fullscreen window covering its output is now scanned out straight
from the client's buffer on the GPU tier (`docs/tty.md`, "A fullscreen
window scans out directly"). That has only been seen on the dev VM, with a
test client allocating virtio dumb buffers. This asks the two questions
only this machine can answer: does a real GL client's buffer (allocated by
Mesa on the AGX render node, handed over `LINEAR`) import onto `apple,dcp`
and go direct, and what does it save. It replaces the earlier one-line
format question: the format no longer gates anything (see
`docs/backlog/resolved/gpu-primary-direct-format-gate-done.md`).

Same safety as Test 4 (a VT; `Ctrl+Alt+F<n>` gets you back). Build the
`gpu-scanout` package at the commit you were given (`nix build .#scoot-gpu`),
then from a VT:

```sh
mkdir -p /tmp/fx
RUST_LOG=info,scoot=debug,smithay::backend::drm::compositor=trace \
  ./result-scoot-gpu/bin/scoot --tty --renderer gles \
  > /tmp/fx/t5.log 2>&1 &
# a GL client, fullscreen, drawing continuously -- any of:
WAYLAND_DISPLAY=wayland-1 mpv --fs --vo=gpu --gpu-context=wayland --loop some-video.mkv &
WAYLAND_DISPLAY=wayland-1 glmark2-wayland --fullscreen --run-forever &
sleep 10
sudo cat /sys/kernel/debug/dri/*/state > /tmp/fx/t5-kms-fullscreen.txt
scootctl screenshot --out /tmp/fx/t5-fs.png
scootctl action toggle-fullscreen; sleep 5          # the same client, tiled
sudo cat /sys/kernel/debug/dri/*/state > /tmp/fx/t5-kms-tiled.txt
scootctl action toggle-fullscreen; sleep 5
# quit scoot (Super+Shift+e), then:
sed 's/\x1b\[[0-9;]*m//g' /tmp/fx/t5.log > /tmp/fx/t5.clean.log
grep -o 'testing direct scan-out .* on plane::Handle([0-9]*)' /tmp/fx/t5.clean.log | grep -o 'plane::Handle([0-9]*)' | sort | uniq -c
grep -o 'successfully assigned element .* to plane::Handle([0-9]*)' /tmp/fx/t5.clean.log | grep -o 'plane::Handle([0-9]*)' | sort | uniq -c
grep 'eligibility changed\|Testing Formats' /tmp/fx/t5.clean.log | head
grep -o 'failed to assign element[^,]*\|skipping direct scan-out[^,]*\|could not import[^,]*' /tmp/fx/t5.clean.log | sort | uniq -c | head
```

The counts above are per plane handle. The primary is the plane that, in
`t5-kms-tiled.txt`, shows a scoot-allocated fb whose `crtc-pos` is the
whole mode (`2560x1600+0+0`); `plane[N]` there is `plane::Handle(N)` here.

What each answer means:

- `eligibility changed ... to=Eligible`, `testing direct scan-out` and
  `successfully assigned` lines on the primary's handle, and the primary in
  `t5-kms-fullscreen.txt` on an fb that is not one of the tiled run's:
  **direct scanout works here.** The screenshot should show the video
  frame, not a stale one.
- `Eligible`, `testing direct scan-out` on the primary, but no
  `successfully assigned`, with `test failed` / `format ... not supported` /
  import failures: the client's buffer cannot be scanned out by
  `apple,dcp` (the import across the AGX→DCP split, or the plane refusing
  `LINEAR` or that size). Send the lines -- that is the next ticket. (A
  `no cached fb, exporting new fb` line is *not* a failure: Smithay prints
  it for every buffer's first, successful export too.)
- `Eligible` but **no** `testing direct scan-out` on the primary at all: the
  frame was allowed but Smithay never tried. Over the default (non-black)
  background it only tries a window that is opaque edge to edge; a client
  whose buffer has an alpha channel and sets no opaque region is not, and
  composites every frame. Worth knowing which client that was -- mpv and
  games usually are opaque, but not all.
- No `Eligible` at all: the window was not covering the output (check the
  client really went fullscreen), or something stayed above it.

`wayland-1` is whatever socket the log's `scoot is up` line names. If it
does go direct, a CPU comparison is worth one more minute: the same client
fullscreen vs tiled (toggle as above), twice each, alternating, sampling the
compositor for 10 s each time --

```sh
p=$(pgrep -x scoot); a=$(awk '{print $14+$15}' /proc/$p/stat); sleep 10
b=$(awk '{print $14+$15}' /proc/$p/stat); echo "jiffies/10s: $((b-a))"
```

### Results, 2026-09-25 — mpv goes direct once its pointer is hidden; a visible pointer rules out every attempt

Run on the same Apple M2 (j413), NixOS 26.11, kernel 7.1.5, Mesa 26.2.2.
Built on the machine from `main` at `e1dce6f`
([Keys for the 2026-09-25 runs](#keys-for-the-2026-09-25-runs) has the
store paths and sha256s). Config: `[output] scale = 1.5` only, as in Test 4.

**Plane inventory first, because it decides this test.** `sudo drm_info`
on `card2` (`apple,dcp`), CRTC 45, connector `eDP-1`:

| plane | type | zpos | formats (`IN_FORMATS`, all at `LINEAR` only) |
| --- | --- | --- | --- |
| 35 | Primary | 0 | XR30 AR30 XR24 AR24 XB24 AB24 NV12 NV16 NV24 P010 P210 |
| 40 | Overlay | 1 (fixed) | AR30 AR24 AB24 NV12 NV16 NV24 P010 P210 (no opaque `X` formats) |

**No cursor plane** (scoot logs `drm: scanout cursor planes cursor_planes=0
overlay_planes=1`), one overlay, and no tiled or compressed modifier on
either plane. **No VRR:** the connector has no `vrr_capable` property at
all (only the CRTC's `VRR_ENABLED`, at 0), and niri reports `Variable
refresh rate: not supported` for the same panel. The swapchain line: `Testing Formats: [AR24 Invalid, AR24
Linear]` (Smithay's plane set, filtered to its color formats, is
`{XR24, AR24} x {Invalid, Linear}`).

**The trap: the software cursor blocks direct scanout.** With no cursor
plane, the pointer is a `Kind::Cursor` memory buffer. Smithay can put that
kind on an overlay, but a memory buffer has no framebuffer to export
(`ExportBuffer::from_underlying_storage` maps `UnderlyingStorage::Memory`
to `None`, `drm/exporter/mod.rs` ~38), so the attempt fails silently and
the cursor is composited. That holds on any hardware at this Smithay rev,
not just here. Smithay only
tries the primary plane for the last element when nothing above it went to
the render list (`remaining_elements == 1 && primary_plane_elements.is_empty()`,
`drm/compositor/mod.rs` ~1995 at the pinned fork `43f50eb`). So while the
pointer is drawn over a fullscreen window, every frame renders two
elements and **no `testing direct scan-out` line appears at all**. That is
what the first run showed with `es2gears_wayland` and with mpv before it
hid its cursor: `Eligible` and steering `Sent` while fullscreen, and across the whole
session (2083 frames) zero primary attempts. `docs/tty.md` already says "a cursor with no plane of its own
makes that frame composite"; on this panel that is **every** frame the
pointer is visible, because there is only one output to park it on.

**With the pointer hidden, it goes direct.** `mpv --fs --cursor-autohide=always
--vo=gpu --gpu-context=wayland` (a 2560x1600 H.264 clip decoded in
software), after one `scootctl pointer move` so mpv gets pointer focus and
sends `set_cursor(serial, nil)`:

```
eligibility changed from=NothingOpaqueCovers to=Eligible
scanout steering changed steer=Sent eligible=true
testing direct scan-out for element … wl_surface@5 … on plane::Handle(35) … GbmFramebuffer { fb: 51, format: XR30 … }
successfully assigned element … to plane::Handle(35) with zpos Some(0) for direct scan-out
```

- 743 `testing direct scan-out` lines on plane 35, 743 `successfully
  assigned`, 1 `test failed` (the very first frame at startup, unrelated).
  About 52 direct frames per second, which is what software decode of that
  clip managed.
- The first direct frame is 11 ms after mpv's `set_cursor(13, nil)`. Before
  that the frames composited.
- `/sys/kernel/debug/dri/2/state` while fullscreen: plane 35 on
  `fb=51 format=XR30 modifier=0x0 size=2561x1601`. While tiled:
  `fb=50 format=AR24 size=2560x1600`, scoot's swapchain. The second
  fullscreen phase went back to `fb=51`.
- mpv's presentation feedback: `presented` flags 9 (`vsync | zero_copy`)
  on exactly the 743 direct frames, 1 on all 451 others.
- **The client's buffer is 2561x1601, and DCP takes it.** At scale 1.5
  the logical output is 1707x1067 (2560/1.5 = 1706.67, rounded up), so a
  fullscreen client renders `ceil(1707 x 1.5)` = 2561 by 1601. The plane is
  committed with `crtc-pos=2561x1601+0+0`, one pixel past the 2560x1600
  mode on each axis, and the atomic test passes. A driver that rejects
  out-of-bounds planes would composite here instead, so this is a
  scale-1.5 edge to keep in mind on other hardware. (niri rounds the same
  panel down, to 1706x1066.)
- The fb's modifier in debugfs is `0x0`, and the client's
  `add(…, 0, 0)` is `LINEAR`, so the panel reads the layout the client
  wrote. Smithay's own trace describes the same fb as `GbmFramebuffer {
  format: XR30, modifier: Invalid }`. Its GBM import does not carry the
  modifier through for this `LINEAR` buffer, which is harmless for
  `LINEAR`. Do not read "matches" as "GBM preserved the modifier": no
  tiled buffer went direct here, so the `DIRECT_FLAGS` guard for tiled
  modifiers was never exercised on this machine. No `lost its tiled
  layout` line appeared.

**CPU, same mpv, same 1080p30 clip, alternating, info-level logging**
(`scoot` jiffies over 10 s; trace logging was off for these):

| round | fullscreen, direct | tiled (composited) | fullscreen, composited (pointer shown) |
| --- | --- | --- | --- |
| 1 | 11 | 23 | 28 |
| 2 | 10 | 23 | 27 |
| 3 | 11 | 23 | 28 |

Idle afterwards: 0. A separate one-round rerun snapshotted debugfs during
each sample, to show which path each column really took: `fb=54 XR30
2561x1601` (direct) and `fb=50 AR24 2560x1600` (composited), and read 10
(direct), 23 (tiled) and 27 (composited) jiffies, in line with the three
rounds. **Direct scanout cuts compositor CPU for a fullscreen video by
about 60%** (10–11 against 27–28 jiffies per 10 s), and costs less than
compositing the same video in a smaller tiled window.

**What this does not show.** A `scootctl screenshot` of a direct frame
forces a composite and re-imports the client's buffer through scoot's own
GLES importer. It never reads what the panel shows, and DCP has no
writeback connector. So the screenshots (`t5b-fs.png`: the test pattern,
correct) prove the import path, not the scanout. The proxy for the panel is
the debugfs fb, whose format and modifier match what the client sent. The
panel was not photographed.

**Consequences.**
- **A visible pointer denies every fullscreen client a primary attempt on
  this machine. Whether a client then goes direct still depends on its
  buffer**: a format and modifier the primary takes (`LINEAR` only here),
  and a size matching the mode. One client was seen to pass both: mpv,
  whose fullscreen buffer was `LINEAR` at 2561x1601, once it hid its
  pointer. The other two would not have gone direct even with the pointer
  hidden:
  - `vkcube` stayed `APPLE_GPU_TILED_COMPRESSED` throughout (all 20
    `add(…, 201326592, 2)` in its trace), which the primary does not take.
  - Both `vkcube` and `es2gears_wayland` submit 1707x1067 buffers at
    `set_buffer_scale(1)` when fullscreen, the logical size. The primary
    would have to upscale those 1.5x, which was not tested on DCP.
    es2gears did switch to `LINEAR` after the steer, so size is its only
    remaining obstacle.

  That is one player, seen once. It does not show how other players or
  games behave.
- Hiding scoot's own cursor after inactivity would remove the pointer
  obstacle for clients whose buffers qualify. Putting the cursor on the
  overlay plane would too: the overlay takes `AR24`, so format is not the
  problem. What blocks it is that Smithay at this rev cannot export a
  memory buffer at all (above), and its GBM exporter also rejects `wl_shm`
  buffers (`drm/exporter/gbm.rs` ~105–118). So a dma-buf-backed cursor
  element needs a change in scoot's Smithay fork. Ticket:
  [gpu-direct-blocked-by-composited-cursor](docs/backlog/resolved/gpu-direct-blocked-by-composited-cursor-done.md)
  (resolved by Test 17: the drawn cursor rides an overlay plane).

## Test 6 — the GLES tier's dma-buf formats, and what GPU clients do with them

Why: under `--renderer gles` the `zwp_linux_dmabuf_v1` feedback is now the
GPU driver's whole import set — every format at every explicit modifier it
names (`docs/protocols.md`, "GPU-rendering clients"), where it used to be
`Xrgb8888`/`Argb8888` at `LINEAR` only. On the dev VM's llvmpipe that is 57
formats, every one at `LINEAR`, because Mesa lists no other layout there;
only this machine can say what AGX offers (tiled and compressed modifiers
are expected), whether a GL client then allocates one of them instead of
`LINEAR`, and whether a video player hands over `NV12` directly.

Nothing here is claimed in advance. It is a record to capture.

**Part A — the advertised table, both GLES tiers.** The offscreen tier needs
no VT; run it from your normal session (or a VT). The `--tty` tier needs a
VT, as in Test 4 (`Ctrl+Alt+F<n>` gets you back).

```sh
mkdir -p /tmp/fx
# offscreen GLES (headless: no window appears; scoot keeps running after
# the command exits, hence the kill)
RUST_LOG=scoot=debug ./result-scoot-gpu/bin/scoot --headless --renderer gles \
  -- sh -c 'wayland-info -i zwp_linux_dmabuf_v1 > /tmp/fx/t6-table-headless.txt' \
  > /tmp/fx/t6-headless.log 2>&1 &
sleep 5; kill %1
# GPU scanout tier, from a VT
RUST_LOG=scoot=debug ./result-scoot-gpu/bin/scoot --tty --renderer gles \
  -- sh -c 'wayland-info -i zwp_linux_dmabuf_v1 > /tmp/fx/t6-table-tty.txt' \
  > /tmp/fx/t6-tty.log 2>&1 &
sleep 8; kill %%
sed -i 's/\x1b\[[0-9;]*m//g' /tmp/fx/t6-*.log
grep 'dmabuf feedback' /tmp/fx/t6-*.log
grep -c '= ' /tmp/fx/t6-table-*.txt              # pairs per tier
grep -v LINEAR /tmp/fx/t6-table-headless.txt | head -20   # the non-linear layouts
grep -E "NV12|P010" /tmp/fx/t6-table-*.txt
```

(`wayland-info` is in `nixpkgs#wayland-utils`.) Read it as: a `pairs=` count
well above 57 and `grep -v LINEAR` listing Apple-vendor modifiers is the
expected shape; `pairs=2` would mean the driver answered no modifier query
at all (the old table, kept on purpose — see `dmabuf::driver_tranche`);
the two tiers' tables differing is worth a line on its own, since both
renderers are Mesa's AGX driver.

**Part B — a GL client and a video player, on the `--tty` tier.** Same VT
session as Test 5, with the log flags from there:

```sh
RUST_LOG=info,scoot=debug ./result-scoot-gpu/bin/scoot --tty --renderer gles \
  > /tmp/fx/t6b.log 2>&1 &
# which modifier does Mesa pick from the new table?
WAYLAND_DISPLAY=wayland-1 WAYLAND_DEBUG=1 es2gears_wayland 2> /tmp/fx/t6-gears.trace &
sleep 5; scootctl screenshot --out /tmp/fx/t6-gears.png; kill %2
grep -m3 'zwp_linux_buffer_params_v1.*add(' /tmp/fx/t6-gears.trace   # last two args: modifier hi, lo
grep -c 'create_pool(' /tmp/fx/t6-gears.trace                        # >0 means it fell back to wl_shm
# a video player handing over dma-bufs (hardware decode if this machine has one)
WAYLAND_DISPLAY=wayland-1 mpv --fs --vo=dmabuf-wayland --hwdec=auto --msg-level=all=v \
  --loop some-video.mkv > /tmp/fx/t6-mpv.log 2>&1 &
sleep 10; scootctl screenshot --out /tmp/fx/t6-mpv.png
sudo cat /sys/kernel/debug/dri/*/state > /tmp/fx/t6-kms-mpv.txt; kill %2
grep -E 'Using DRM device|hwdec|upload|VO:|failed|error' /tmp/fx/t6-mpv.log | head -20
```

(`es2gears_wayland` is in `nixpkgs#mesa-demos`.) What each answer means:

- **The GL client's `add(` modifier is not `0, 0`** (not `LINEAR`) and the
  screenshot shows gears: GPU clients now render into their native layout
  here, the point of this change. `0, 0` with the table offering other
  layouts is Mesa's own choice and worth recording; a non-zero
  `create_pool(` count means the client never used dma-bufs at all.
- **The client disappears, or the log has `import refused`**: the promise
  broke on this hardware — the most important thing this test can find.
  Send the trace and the log.
- **mpv plays, `--vo=dmabuf-wayland` is the active VO and the screenshot
  shows the video**: zero-copy video into the compositor works. Its
  `add(` lines (in a `WAYLAND_DEBUG=1` rerun) say which fourcc it sent —
  `NV12` is the expected one. mpv refusing the VO or failing to upload says
  this machine has no decoder path mpv can hand over, which is a finding
  about the machine, not about scoot. On the dev VM it could not run at
  all (no VA-API driver, and software frames cannot be uploaded to
  `drm_prime` there); the same NV12 path is covered there by a test that
  allocates the buffers itself.
- **Direct scanout, re-checked.** With the new table a fullscreen GL client
  may allocate a tiled layout the display controller cannot scan out, in
  which case Test 5's fullscreen frames composite instead of going direct
  (a missed optimisation, not a failure — steering fullscreen clients to a
  scannable layout is the per-surface scanout tranche, Part C below). If
  Test 5 is run on a build with this change, note the modifier the client
  sent next to the answer. And if a fullscreen buffer *does* go direct, check the fb's
  modifier in `t5-kms-fullscreen.txt` against the client's `add(` modifier:
  they must match. A GBM import that drops or changes a tiled modifier is
  the one exposure `DIRECT_FLAGS` in `tty/scanout.rs` names; scoot now
  refuses such a framebuffer and composites instead, logging `not scanning
  out a client buffer whose framebuffer lost its tiled layout` at `debug`.
  Seeing that line here means this machine's GBM does it (worth reporting);
  seeing scrambled tiles on screen means the guard missed a case.

**Part C — the scanout tranche: does a fullscreen GL client move into a
layout the display takes, and then go direct?** On the GPU tier the window
covering the output is sent per-surface dma-buf feedback whose first
tranche is flagged `scanout`, names the display device (`apple,dcp`'s card,
not AGX's render node), and lists the layouts the primary plane accepts
(`docs/protocols.md`, "Per-surface feedback: the scanout tranche"). On the
dev VM that tranche is `XR24`/`AR24` at `LINEAR` and the test client went
direct with it; no GL client there can allocate a dma-buf, so whether Mesa
acts on it is only answerable here. Same VT session and flags as Test 5:

```sh
RUST_LOG=info,scoot=debug,smithay::backend::drm::compositor=trace \
  ./result-scoot-gpu/bin/scoot --tty --renderer gles > /tmp/fx/t6c.log 2>&1 &
# tiled first, then fullscreen: the feedback changes on the transition
WAYLAND_DISPLAY=wayland-1 WAYLAND_DEBUG=1 es2gears_wayland 2> /tmp/fx/t6c-gears.trace &
sleep 5; scootctl action toggle-fullscreen; sleep 8
sudo cat /sys/kernel/debug/dri/*/state > /tmp/fx/t6c-kms-fullscreen.txt
scootctl screenshot --out /tmp/fx/t6c-fs.png
scootctl action toggle-fullscreen; sleep 5; kill %2
# the same with a client that asks for presentation feedback
WAYLAND_DISPLAY=wayland-1 WAYLAND_DEBUG=1 mpv --fs --vo=gpu --gpu-context=wayland \
  --loop some-video.mkv 2> /tmp/fx/t6c-mpv.trace &
sleep 10; sudo cat /sys/kernel/debug/dri/*/state > /tmp/fx/t6c-kms-mpv.txt; kill %2
# quit scoot (Super+Shift+e), then:
sed -i 's/\x1b\[[0-9;]*m//g' /tmp/fx/t6c.log
grep 'scanout tranche\|scanout steering changed\|eligibility changed' /tmp/fx/t6c.log
grep -c 'get_surface_feedback' /tmp/fx/t6c-*.trace          # did the client ask at all
grep 'tranche_flags\|tranche_target_device' /tmp/fx/t6c-gears.trace | head
grep 'zwp_linux_buffer_params_v1.*add(' /tmp/fx/t6c-gears.trace | awk '{print $NF, $(NF-1)}' | uniq -c
grep 'wp_presentation_feedback[@#][0-9]*\.presented' /tmp/fx/t6c-mpv.trace \
  | sed 's/.*, \([0-9]*\))$/\1/' | sort | uniq -c      # count, then flags in decimal
```

(The `dmabuf scanout tranche` line at `debug` in `t6c.log` is the whole
tranche; the `scanout tranche for fullscreen windows pairs=… device=…` line
above it is its size and the device it names.) What each answer means:

- **`scanout tranche … pairs=N`, then after the toggle `scanout steering
  changed steer=Sent`, a `tranche_flags(1)` in the trace, and the `add(`
  modifier changing after the toggle** to one in the tranche: Mesa moved
  the fullscreen window into a scannable layout. If Test 5's
  `successfully assigned … to plane::Handle(<primary>)` lines follow, it
  then went direct — the whole point of this part. The modifier going back
  after the second toggle is the revert.
- **`eligibility changed … to=NothingOpaqueCovers` or `to=NotTheWindow`
  while the client is fullscreen, and no `scanout steering changed`**:
  scoot judged that Smithay would never offer the display this window's
  buffer, so it neither tries direct scanout nor steers. `NothingOpaqueCovers`
  means nothing opaque spans the output over a non-black background --
  most likely the client renders with an alpha channel (an `ARGB` EGL
  config) and declares no opaque region; `NotTheWindow` means Smithay would
  try something else, typically a wallpaper under a window that is not
  opaque. Neither is a failure. Worth recording which client it was and
  whether it sets an opaque region (`grep -c set_opaque_region` on its
  trace), since opaque-format clients are the common case; rerun it over a
  black background with no wallpaper (`background_color = "#000000"` in
  `[appearance]`) to see it steered.
- **`the primary plane takes none of the advertised formats`**: nothing
  the renderer imports is on the plane's list, so nothing is steered. Send
  the `dmabuf scanout tranche`-less log and `t6-table-tty.txt` from Part A
  plus `sudo drm_info` (or the plane's `IN_FORMATS` from debugfs).
- **The trace shows the scanout tranche but the `add(` modifier never
  changes**: Mesa did not act on it. One known reason to look for first:
  the tranche's `target_device` is the display card, not the AGX render
  node Mesa renders on — record which device each `tranche_target_device`
  names (`ls -l /dev/dri`, major/minor). That is a finding about how Mesa
  treats a split render/display machine, not a failure.
- **`not scanning out a client buffer whose framebuffer lost its tiled
  layout`**, followed by a second `scanout tranche` line with `lost=1`: this
  machine's GBM loses that modifier; scoot dropped it from the tranche and
  re-sent it. Worth reporting with the modifier.
- **mpv's `presented` flags** are the event's last argument, which
  `WAYLAND_DEBUG` prints in decimal and the pipeline above extracts: `9` is
  `vsync | zero_copy` (direct), `1` is composited. Seeing `9` confirms direct
  scanout from the client's side; seeing only `1` while Test 5's lines say
  the primary took the buffer would be a scoot bug. (If mpv did not ask for
  presentation feedback, the count is empty; nothing is wrong.)

### Results, 2026-09-25 — tiled and compressed on offer, Mesa uses them, and follows the tranche to `LINEAR`

Same machine, build and config as Test 5's results.

**Part A: the table.** Both GLES tiers advertise the same table. The two
`wayland-info` dumps are identical except for the global's name (39
headless, 40 on `--tty`), and all 162 format lines match:
`dmabuf feedback: advertising the renderer's importable formats pairs=162
fourccs=54`, main device `renderD128` (0xe280). Every one of the 54 fourccs
comes at exactly three modifiers: `APPLE_GPU_TILED_COMPRESSED`
(`0x0c00000000000002`), `APPLE_GPU_TILED` (`0x0c00000000000001`) and
`LINEAR`. No `INVALID`. That includes `NV12`, `P010` and the other YUV
formats. So this is the expected shape, well above the dev VM's 57 pairs.

**Part B: a GL client.** `es2gears_wayland` (Mesa `asahi`, "Apple M2 (G14G
B0)"), tiled: `add(…, 201326592, 2)`, i.e. `APPLE_GPU_TILED_COMPRESSED`, in
format `XR30` (`808669784`), zero `create_pool` (dma-bufs throughout), and
the gears draw correctly (`t5-gears-tiled.png`, `t5-gears-fs.png`). Mesa
also picked `XR30` for mpv, not `XR24`. `vkcube` (honeykrisp, Vulkan 1.4)
allocates `XR24` (`875713112`), also `APPLE_GPU_TILED_COMPRESSED`. No client
disappeared, and no `import refused` or protocol error was logged.

**Part B: mpv with `--vo=dmabuf-wayland` — cannot run on this machine.**
There is no hardware decoder that userspace can reach:
- The AVD video decoder's driver fails to probe:
  `avd 269080000.avd: Direct firmware load for apple/avd-fw-v3-t1.bin
  failed with error -2`, then `probe with driver avd failed with error
  -2`. So there is no `/dev/videoN` for it. The only V4L2 device is
  `apple-isp`, the camera (`v4l2-ctl --list-devices`). The system's
  firmware search path holds no `*avd*` file. All of this was recorded in
  `~/fx/t6-decoder-evidence.txt` on the machine, from `journalctl -k -b`,
  since the kernel ring buffer had wrapped by then.
- VA-API has no driver (`asahi_drv_video.so` not found).
- mpv's Vulkan video hwdec found nothing usable.
- With `--hwdec=no`, mpv 0.41's dmabuf VO cannot upload software frames
  (`[hwupload] no support for this hw format`), and the file exits with
  `Errors when loading file`.

This is a finding about the machine, not scoot. Separately, Mesa's Vulkan
loader could not open `card1`/`card2` (`Permission denied`). That was the
test setup: VT 2 was active with no logind session on it, so the user held no
seat ACLs. It does not affect `renderD128`, which is mode 0666.

**Part C: the scanout tranche, and Mesa follows it.** `scanout tranche for
fullscreen windows pairs=10 lost=0 device=57858`: 57858 is `0xe202`,
`card2`, the display controller, not AGX's render node. The pairs are
`XR24 AR24 AR30 XR30 AB24 XB24 NV12 P010 NV16 NV24` at `LINEAR`: the
primary plane's list minus `P210`, which the renderer does not import.
- On every toggle into fullscreen the log shows `scanout steering changed
  steer=Sent`, and the client trace shows `tranche_flags(1)`.
- **Mesa reallocates in response.** es2gears went from
  `APPLE_GPU_TILED_COMPRESSED` to `LINEAR` (`add(…, 0, 0)`) after the
  toggle, and back after `steer=Reverted`. mpv did the same: compressed
  836x1043 tiled, then `LINEAR` 2561x1601 fullscreen, then compressed
  1254x1565 once tiled again. So Mesa acts on a tranche whose
  `target_device` is the display card, even though it renders on the AGX
  node.
- mpv then went direct once its pointer was hidden. Its `presented` flags
  were `9` (`vsync | zero_copy`) 743 times and `1` 451 times, which lines
  up frame for frame with Test 5's plane assignments.
- `vkcube` did not reallocate after its `tranche_flags(1)`, but the
  ordering matters. On the toggle to fullscreen, vkcube rebuilt its
  swapchain at 1707x1067 between 12:39:41.4404 and .4476 (four new
  `APPLE_GPU_TILED_COMPRESSED` buffers, the old four destroyed). The
  scanout tranche reached it about 7 ms later, at .4549, and it never
  rebuilt again. It then composited (the pointer was visible anyway). So
  this may be a timing artefact: scoot sends the tranche after the
  fullscreen configure, and it is not known whether vkcube would have used
  it had it arrived before the rebuild. It was not tested whether the
  cause is Mesa's WSI, vkcube, or that ordering.
- `get_surface_feedback` was requested once by each client.
- Neither `the primary plane takes none of the advertised formats` nor
  `lost its tiled layout` appeared.

`WAYLAND_DEBUG` prints `tranche_target_device` as `array[8]`, so the
device number comes from scoot's log line rather than the client trace.

## Test 7 — explicit sync (`linux-drm-syncobj-v1`) on a real GPU

Why: on the GPU tier scoot now offers explicit sync where a DRM device can
import timeline syncobjs and wait on them with an eventfd
(`docs/protocols.md`, "Explicit sync"). It tries the display device first
and the render nodes (`/dev/dri/renderD*`) second, because on this machine the display controller
(`apple,dcp`) and the GPU (AGX) are separate DRM devices and only the GPU's
driver is expected to support syncobjs. On the dev VM (virtio-gpu, one
device) the display device passes, and a test client that signals its own
timelines was seen held until its acquire point signalled and released only
after scoot was done with each buffer. Whether the global comes up here,
on which device, and whether a real Vulkan client uses it, is only
answerable here. Nothing is claimed in advance.

Same VT session and build as Test 5 (a `gpu-scanout` build):

```sh
RUST_LOG=info,scoot=debug ./result-scoot-gpu/bin/scoot --tty --renderer gles \
  > /tmp/fx/t7.log 2>&1 &
sleep 4
WAYLAND_DISPLAY=wayland-1 wayland-info -i wp_linux_drm_syncobj_manager_v1 > /tmp/fx/t7-global.txt
# a Vulkan client (nixpkgs#vulkan-tools); Mesa's WSI uses explicit sync when
# the global is present and the driver supports it
WAYLAND_DISPLAY=wayland-1 WAYLAND_DEBUG=1 vkcube --wsi wayland 2> /tmp/fx/t7-vkcube.trace &
sleep 6; scootctl screenshot --out /tmp/fx/t7-vkcube.png
scootctl action toggle-fullscreen; sleep 6
scootctl screenshot --out /tmp/fx/t7-vkcube-fs.png
sudo cat /sys/kernel/debug/dri/*/state > /tmp/fx/t7-kms-fs.txt
kill %2
# quit scoot (Super+Shift+e), then:
sed -i 's/\x1b\[[0-9;]*m//g' /tmp/fx/t7.log
grep 'explicit sync\|explicit-sync candidate' /tmp/fx/t7.log
grep -c 'import_timeline' /tmp/fx/t7-vkcube.trace
grep -c 'set_acquire_point' /tmp/fx/t7-vkcube.trace
grep -m3 'protocol error\|killed' /tmp/fx/t7.log
```

What each answer means:

- **`explicit sync (wp_linux_drm_syncobj_manager_v1) offered
  device=/dev/dri/renderD128`**: the display device could not import
  timelines and the fallback took over, the shape this machine was
  expected to have. `device=the display device` means `apple,dcp`'s
  driver supports syncobjs after all (worth a line). `not offered` with
  both `explicit-sync candidate` lines at debug means neither could, and
  no client here is offered explicit sync; send the log.
- **`import_timeline` and `set_acquire_point` counts above zero, and
  `vkcube` spinning in both screenshots**: a real Vulkan client runs with
  explicit sync on scoot. Zero with the global present means Mesa chose
  not to use it (its driver may lack what WSI needs); the cube still
  spinning is then implicit sync, as before.
- **The cube freezes, stutters every few seconds, or the client
  disappears**: a release or acquire point went wrong. Send the trace and
  the log, and the `protocol error` grep. A `no_memory` kill names the
  outstanding-wait bound, which a real client should never reach.
- **On an NVIDIA machine** (not this one), the same steps with any Vulkan
  or GL app answer the question that matters most for NVIDIA users; this
  project has no NVIDIA hardware to run them on.

### Results, 2026-09-25 — offered on the display device, and `vkcube` uses it

Same machine, build and config as Test 5's results.

- **`drm: explicit sync (wp_linux_drm_syncobj_manager_v1) offered
  device="the display device"`.** This was not the expected shape: the
  `apple,dcp` driver supports syncobj timelines itself. `drm_info` agrees
  for both cards: `DRM_CAP_SYNCOBJ = 1` and `DRM_CAP_SYNCOBJ_TIMELINE = 1`
  on `card2` as well as on AGX's `card1`. So the render-node fallback
  never ran here. `wayland-info` lists `wp_linux_drm_syncobj_manager_v1`
  version 1.
- **A real Vulkan client uses it.** `vkcube --wsi wayland` (Mesa
  honeykrisp, "Apple M2 (G14G B0)", Vulkan 1.4.354):
  - `import_timeline` 40 times, two timelines per swapchain image across
    its startup swapchain rebuilds and one fullscreen rebuild;
  - `set_acquire_point` 827 times and `set_release_point` 827 times over
    about 14 s.
  - The cube is drawn and spinning in both captures, tiled (`t7-vkcube.png`)
    and fullscreen (`t7-vkcube-fs.png`).
  - No protocol error, no `no_memory` kill, and no stall.
- Fullscreen it composited (`fb=50 AR24`, the pointer was visible, see
  Test 5), so this run covers explicit sync on the composited path. On the
  direct path, the release points follow the flip. That was covered on the
  dev VM but not here.

## Test 8 — `--nested --renderer gles` handing its frames to the host as dma-bufs

Why: in a `gpu-scanout` build, a `--nested --renderer gles` scoot whose
host composites on the same GPU no longer reads each frame back into
`wl_shm`: it copies the frame on the GPU into a buffer shared with the host
(`docs/tty.md`, "Which renderer draws the frames"). On the dev VM that works
end to end -- the host's screenshot is byte-identical to the nested one, and
the protocol trace shows the buffers created and attached -- but its GPU is
llvmpipe, where drawing the frame in software is nearly all of the cost, so
per frame it measured the same as read-back (and a little dearer per resize,
~0.9 ms of the ~14 ms a resize costs there). Whether dropping the copy
saves CPU on a GPU that is a GPU is only answerable on one. Nothing is
claimed in advance.

No VT switch: the bench starts its own host (an outer headless scoot on
the same GPU, sized so the nested window fills it) from a terminal in your
normal session. It needs `foot` on `PATH` (`nix shell nixpkgs#foot` if it
is not installed). Same commit for both builds (the Build section's two):

```sh
for build in result result-scoot-gpu; do
  SCOOT=./$build/bin/scoot HOST_SCOOT=./result/bin/scoot PREFIX=/tmp/fx/t8-$build \
    scripts/nested-dmabuf-bench.sh | tee /tmp/fx/t8-$build.txt
done
grep -h 'presentation:\|frames:\|resizes:\|pixels:' /tmp/fx/t8-*.txt
```

What each answer means:

- **`presentation:`** must read `by read-back ... no gpu-scanout feature`
  for `result` and `by dma-buf (no read-back)` for `result-scoot-gpu`. A
  gpu build that says read-back names its reason (a host on another DRM
  device than the renderer, no common format, GBM refusing): send
  `/tmp/fx/t8-result-scoot-gpu-inner.log`.
- **`frames:` ms/tick, nested and host, gpu build against default**: the
  number this test is for. Lower nested CPU per tick under the same load is
  the copy that is gone; the host figure says whether importing the buffer
  instead of uploading it helped the host too.
- **`resizes:` ms/size** and the RSS/fd lines around them: a new size
  allocates host buffers under dma-buf, so a small increase is expected;
  fd counts must be the same before and after on both processes.
- **`pixels: ... identical`** on both builds: the host shows exactly what
  the nested scoot rendered, the right way up. `DIFFERENT` is a bug: send
  both PNGs.

**Part A results, 2026-09-25** (same machine and build as Test 5's
results). Run as written above, twice, alternating the builds. The host
is the default build's `--headless --renderer gles` on the AGX render node.

| | default build (read-back) | `gpu-scanout` build (dma-buf) |
| --- | --- | --- |
| `presentation:` | `by read-back into wl_shm reason="this build has no gpu-scanout feature"` | `by dma-buf (no read-back)` |
| nested, ms per client tick | 1.167, 1.167 | 0.617, 0.633 |
| host, ms per client tick | 0.967, 0.967 | 0.317, 0.317 |
| nested, ms per resize | 5.868, 5.785 | 4.711, 4.545 |
| RSS / fds, before → after 121 resizes | nested 82192→82240 kB, 26→26; host 79488→79648 kB, 23→23 | nested 75920–75936→75968 kB, 32→32; host 72928–72992→73056–73120 kB, 26→26 |
| `pixels:` | identical | identical |

**On a real GPU, dropping the read-back pays off**: nested CPU per frame
falls by 1.8–1.9x, and the host's by 3.0x, since it imports a buffer
instead of uploading one. A resize is 20% cheaper too, where the dev VM
measured it 6–8% dearer. Every size was applied without a redraw under
dma-buf (`owed frames handed over without a redraw: 243`), fds stayed flat
on both processes, and the host's screenshot was byte-identical to the
nested one in all four runs.

### Part B -- nested inside your own desktop

Part A is scoot hosting scoot: one importer, Smithay's. The daily-drive
case is scoot nested inside the desktop you normally run -- GNOME, KDE,
sway, niri -- whose importer is not Smithay's and whose feedback on this
GPU likely offers tiled or compressed layouts, not only `LINEAR`. If your
session *is* scoot, do this from a VT running another compositor instead
(`sway` or `niri` from `nix shell`), and say which.

From a terminal in that desktop, same two builds:

```sh
for build in result result-scoot-gpu; do
  WAYLAND_DEBUG=client RUST_LOG=info,scoot::compositor::nested=debug \
    ./$build/bin/scoot --nested --renderer gles --socket /tmp/fx/t8b.sock \
    > /tmp/fx/t8b-$build.log 2>&1 &
  pid=$!; sleep 3
  SCOOT_SOCKET=/tmp/fx/t8b.sock ./result/bin/scoot msg action spawn foot sh -c \
    'i=0; while :; do i=$((i+1)); printf "\r%08d" $i; sleep 0.033; done'
  sleep 5
  j0=$(awk '{print $14 + $15}' /proc/$pid/stat); sleep 20
  j1=$(awk '{print $14 + $15}' /proc/$pid/stat)
  echo "$build: $((j1 - j0)) jiffies over 20 s at ~30 Hz" | tee -a /tmp/fx/t8b.txt
  SCOOT_SOCKET=/tmp/fx/t8b.sock ./result/bin/scoot msg screenshot --no-cursor \
    --out /tmp/fx/t8b-$build.png
  kill $pid; wait $pid
done
sed -i 's/\x1b\[[0-9;]*m//g' /tmp/fx/t8b-*.log
grep -h 'nested: presenting' /tmp/fx/t8b-*.log
grep -c 'zwp_linux_buffer_params_v1@[0-9]*\.created' /tmp/fx/t8b-result-scoot-gpu.log
grep -c 'zwp_linux_buffer_params_v1@[0-9]*\.failed' /tmp/fx/t8b-result-scoot-gpu.log
grep -m3 'zwp_linux_buffer_params_v1@[0-9]*\.add(' /tmp/fx/t8b-result-scoot-gpu.log
grep -h ' WARN \| ERROR ' /tmp/fx/t8b-*.log | head
```

Take a screenshot of the scoot window with your desktop's own tool too.

What each answer means:

- **`presenting to the host by dma-buf ... modifiers=[...]`** for the gpu
  build: your desktop takes scoot's buffers directly. The modifiers show
  whether a tiled layout was chosen (anything but `Linear`). `by read-back
  ... reason=` names why not; send the log.
- **`created` above zero and `failed` zero**: the host imported every
  buffer. Any `failed` means it refused one, and the log must then show
  exactly one `presenting to the host by read-back into wl_shm from now on`
  warning with scoot still drawing -- anything else is a bug.
- **The two `jiffies` lines**: nested CPU for the same load, read-back
  against dma-buf, inside a real desktop. This is the number the change is
  for. Both runs carry the protocol trace's cost; if they come out close,
  rerun the loop without `WAYLAND_DEBUG=client` for a cleaner pair.
- **Your desktop's screenshot of the window against `t8b-*.png`**: the same
  picture, the right way up, with no garbled tiles. Scrambled blocks mean a
  layout was mismatched between scoot and the host: send both, and the
  `add(` lines.

**Part B results, 2026-09-25.** The host is niri 26.04 (nixpkgs), which
owned `--tty` on VT 2 with the benchmark config plus `output "eDP-1" {
scale 1.5; }` (`/tmp/fx/niri-t9.kdl` below, animations off). niri rendered
on the AGX: `GL Renderer: "Apple M2 (G14G B0)"`. Nested scoot used
`--config` with `scale = 1.5` only, which `--nested` ignores as it should
(`using 1.0`). The loop above ran once per build with
`WAYLAND_DEBUG=client`, then twice per build without it, alternating.

- **`nested: presenting to the host by dma-buf (no read-back)
  format=DrmFourcc(AR24) modifiers=[APPLE_GPU_TILED_COMPRESSED,
  APPLE_GPU_TILED, Linear] node=renderD128`** for the gpu build, in all
  three runs. The trace shows scoot allocated
  `APPLE_GPU_TILED_COMPRESSED` (`add(…, 201326592, 2)`), `created` 3,
  `failed` 0. So niri's importer (Smithay's too, but a different
  compositor's) takes scoot's tiled, compressed buffers. The default build
  said `by read-back … no gpu-scanout feature`, as it should.
- **CPU over 20 s at ~30 Hz** (jiffies, no protocol trace):

  | run | default: scoot | default: niri | gpu: scoot | gpu: niri |
  | --- | --- | --- | --- | --- |
  | 1 | 77 | 83 | 38 | 34 |
  | 2 | 76 | 82 | 42 | 37 |
  | with `WAYLAND_DEBUG=client` | 110 | 84 | 75 | 35 |

  Inside a real desktop, dma-buf halves nested scoot's CPU (76–77 → 38–42)
  and more than halves the host's (82–83 → 34–37).
- **No garbled tiles.** niri's own `screenshot-screen` goes through niri's
  importer of scoot's compressed buffer. It shows the same frame as scoot's
  own capture (counter `00000709` in both), clean text, and the window
  border intact (`t8b-result-scoot-gpu-nodebug-1-niri.png` against
  `t8b-result-scoot-gpu-nodebug-1.png`). The nested window is scaled 1.5x
  by niri, because nested scoot draws at scale 1, so the two are compared
  by eye, not by bytes.
- WARNs: only niri's own (no DRM lease without seat ACLs, no EDID blob, no
  notification daemon for its screenshot toast), plus scoot's expected
  nested-scale one.

## Test 9 — scoot vs niri on a real GPU

Why: the scoot/niri A/B in [`docs/benchmarks.md`](docs/benchmarks.md) ran on
the dev VM, where it could only be half an answer. niri renders only through
GLES and refuses a software renderer on `--tty`. The VM's GPU has no 3D, so
there niri could run only nested, and both GLES paths (niri's and scoot's)
rasterised on llvmpipe. On the VM, the pixman row is the honest GPU-less
comparison. What the GLES rows cost on a real GPU, and what either
compositor costs on a real `--tty` session, can only be measured on a
machine like this one. Nothing is claimed in advance.

Needs: `niri` (`nix build nixpkgs#niri -o /tmp/fx/niri`), `cage`,
`wlr-randr`, `foot` and `grim` on `PATH`, both scoot builds from
[Build](#build), plus `scootctl` (`nix build .#scootctl -o result-scootctl`).
It also needs the benchmark's pointer helper:
`cargo build --release --manifest-path scripts/niri-ab/vptr/Cargo.toml --target-dir /tmp/fx/vptr`.

### Part A — nested, no VT

This is the same script and the same scenes as the VM run. The one change is
that the host (cage, headless) renders with GLES, which gives it
`linux-dmabuf`. That is what lets niri's nested backend use the AGX instead of
falling back to software, and what lets a `gpu-scanout` scoot present by
dma-buf (Test 8). It runs inside your normal session and takes no seat.

```sh
common="HOST_RENDERER=gles2 SCOOTCTL=$PWD/result-scootctl/bin/scootctl NIRI=/tmp/fx/niri/bin/niri VPTR=/tmp/fx/vptr/release/nab-vptr"
env $common SCOOT=$PWD/result/bin/scoot OUT=/tmp/fx/t9a scripts/niri-ab-bench.sh
env $common SCOOT=$PWD/result/bin/scoot OUT=/tmp/fx/t9a-diag DIAG=1 scripts/niri-ab-bench.sh
# the gpu-scanout tier: the same session, presenting by dma-buf
env $common SCOOT=$PWD/result-scoot-gpu/bin/scoot VARIANTS=scoot-gles OUT=/tmp/fx/t9a-gpu scripts/niri-ab-bench.sh
scripts/niri-ab/summarize.sh /tmp/fx/t9a /tmp/fx/t9a-diag > /tmp/fx/t9a.md
```

Check each of these before trusting any row:

- `grep -h "GL Renderer" /tmp/fx/t9a/r1-*/inner.log` names the AGX, not
  `llvmpipe`, for niri and for scoot-gles. Each should have one line. niri's
  default log filter hides this line, so the script runs niri with
  `RUST_LOG=niri=debug,smithay::backend::renderer::gles=info`. A niri log
  without the line means the filter was lost, not that nothing rendered.
- `grep -h "presenting to the host" /tmp/fx/t9a-gpu/r1-*/inner.log` says
  `by dma-buf`. `by read-back` means Test 8's path did not come up, so read
  its reason.
- `/tmp/fx/t9a/notes.log` is empty or absent. A line there means a window
  never mapped, a compositor never went idle, or a session was killed for
  running past `SESSION_TIMEOUT`. `HOST_RENDERER=gles2` itself could not be
  checked on the dev VM: cage's GLES renderer cannot allocate its output
  there (`gbm_bo_create failed: Permission denied` on virtio-gpu), and a
  nested niri whose host has no output never answers IPC. If that happens
  here as well, `host.log` in the session's directory says so.

### Part B — `--tty`, needs a VT

Run each compositor as the session on a spare VT, and measure from a `foot`
inside it. Read the [Safety](#safety) section first. Its way back,
`Ctrl+Alt+F<your desktop's VT>`, works in niri too, even with this config's
empty key bindings: niri handles the VT-switch keysyms itself, before it
consults any binds (`find_bind` in its `src/input/mod.rs`). **Stay on that
VT while a window is being measured**: a compositor on an inactive VT is
paused, and its numbers look spectacular for exactly the wrong reason
(Test 4 explains this). Each session covers one tier. Run the tiers alternately, at least
twice each: scoot (dumb + pixman), niri, scoot-gpu `--renderer gles`, niri,
and so on.

```sh
# the benchmark's niri config binds no keys, so start its foot from the config;
# and keep niri from starting xwayland-satellite (see below)
{ cat scripts/niri-ab/niri-anim-off.kdl; echo 'spawn-at-startup "foot"'
  printf 'xwayland-satellite {\n    off\n}\n'; } > /tmp/fx/niri-t9.kdl
# on a spare VT, one of:
./result/bin/scoot --tty -- foot
./result-scoot-gpu/bin/scoot --tty --renderer gles -- foot
RUST_LOG=niri=debug,smithay::backend::renderer::gles=info \
    /tmp/fx/niri/bin/niri -c /tmp/fx/niri-t9.kdl 2>/tmp/fx/t9b-niri.log
```

If `xwayland-satellite` is on your `PATH`, niri starts it as a separate
process, and `sample.sh` measures niri's process only, so that CPU would go
uncounted. The `xwayland-satellite { off }` block above stops niri from
starting it. Check with `pgrep -a xwayland-satellite` before measuring. scoot
is measured without `--xwayland` for the same reason.

Open two more `foot`s so that three columns exist: `Super+Return` in scoot,
`niri msg action spawn -- foot` in niri. Put `$PWD/result-scootctl/bin` on
`PATH` for the scoot lines, then run the following from one of the
`foot`s. `session` makes `sample.sh` measure the compositor it is running
inside. On a machine whose desktop is scoot, `pgrep -x scoot` would find
your live session too.

```sh
S=scripts/niri-ab/sample.sh; O=/tmp/fx/t9b.tsv
# columns: label comm wall_s proc_cpu_ms cpu_ns wakeups threads rss_kb pss_kb.
# Compare proc_cpu_ms (the process total). cpu_ns misses threads that exit
# inside the window, and niri encodes every screenshot on one: on the dev VM
# cpu_ns missed 19-23% of niri's screenshot CPU (docs/benchmarks.md).
$S session 20 idle >> $O
# relayout, through the compositor's own IPC (use the scoot or the niri line)
$S session 10 relayout -- sh -c 'while :; do scootctl action focus-column left; sleep 0.05; scootctl action focus-column right; sleep 0.05; done' >> $O
$S session 10 relayout -- sh -c 'while :; do niri msg action focus-column-left; sleep 0.05; niri msg action focus-column-right; sleep 0.05; done' >> $O
# pointer, kernel-level and so the same for both: needs ydotoold running with
# /dev/uinput access (nixpkgs#ydotool). One fork per event, so the rate is
# whatever the machine manages: compare CPU per second, not per event.
$S session 10 pointer -- sh -c 'while :; do ydotool mousemove -x 800 -y 0; ydotool mousemove -x -800 -y 0; done' >> $O
# screenshots through the compositor's own path (scoot or niri line), then grim
$S session 30 shot-ipc -- sh -c 'for i in $(seq 10); do scootctl screenshot --no-cursor --out /tmp/fx/t9b.png; sleep 0.2; done' >> $O
$S session 30 shot-ipc -- sh -c 'for i in $(seq 10); do niri msg action screenshot-screen --show-pointer false --path /tmp/fx/t9b.png; sleep 0.2; done' >> $O
$S session 30 shot-grim -- sh -c 'for i in $(seq 10); do grim /tmp/fx/t9b-grim.png; sleep 0.2; done' >> $O
```

On a real `--tty`, both compositors draw the pointer, which the nested run
could not compare: nested, niri composites its pointer into every frame and
scoot leaves it to the host. So the pointer row is the new information here.
Under `--renderer gles`, scoot's memory should stay flat across the
screenshot lines. Before PR #238, each capture of a still screen kept its
read-back buffer until the next frame drew
([fixed](docs/backlog/resolved/gles-capture-leaks-a-frame-per-shot-done.md)).
On the dev VM's llvmpipe that buffer was process heap, so the RSS column
(second from last) grew by one frame per capture. On a real GPU it is a
driver allocation. It only counts in RSS while it is mapped into the
process, so a leak may show there or may not. Two checks cover it:

- The RSS column should not climb by about one frame per capture (about
  8 MB at 1080p). If it does, that is a regression; send the log.
- If the driver reports per-process GPU memory through DRM fdinfo, compare
  `grep -h '^drm-total-' /proc/$(pidof scoot)/fdinfo/*` before and after
  the screenshot lines. It should not grow by a frame per capture either.
  If there are no such lines, skip this check: scoot's own tests pin the
  fix by counting live GL objects, not by memory.
Quit with `Super+Shift+e` (scoot) or, from a `foot` in niri,
`niri msg action quit --skip-confirmation`.

### Results, 2026-09-25

Same machine and build as Test 5's results; niri 26.04 from nixpkgs. All
numbers are the compositor process's `/proc/PID/stat` total ("process ms",
10 ms ticks), which is the column the PR #237 review said to compare. The
raw TSVs are on the machine under `~/fx/t9a*`, `~/fx/t9b.tsv`.

**Part A (nested in cage `HOST_RENDERER=gles2`, 1600x1000, three rotating
rounds).** Run exactly as written above, plus the gpu-scanout build as its
own `VARIANTS=scoot-gles` pass (`t9a-gpu`).
- Every check passed. `GL Renderer: "Apple M2 (G14G B0)"` appears in both
  niri logs and in scoot-gles's. The gpu build says `presenting to the
  host by dma-buf (no read-back)`, and the default build says `by read-back
  … no gpu-scanout feature`. `notes.log` is empty in all three runs. The
  cage host allocated its output on this GPU, unlike on the dev VM.

Medians [range] over three rounds:

| scene | scoot-pixman | scoot-gles (read-back) | scoot-gles, gpu build (dma-buf) | niri, anim off | niri, anim on |
| --- | --- | --- | --- | --- | --- |
| idle 20 s | 0 | 0 | 0 | 10 [0–10] | 0 [0–10] |
| pointer, 1200 events | 90 [90–90] | 100 [90–110] | 100 [90–100] | 750 [720–750] | 760 [740–780] |
| relayout, 200 actions | 1760 [1760–1780] | 510 [510–510] | **250** [240–250] | 580 [510–590] | 630 [620–660] |
| shot-ipc, 10 | 100 [90–110] | 80 [70–80] | 80 [80–100] | 160 [160–170] | 160 [150–160] |
| shot-grim, 10 | 50 [40–50] | 30 [20–40] | 40 [40–50] | 40 [30–40] | 40 [30–40] |
| animate 10 s | 2940 [2930–2970] | 1270 [1260–1290] | **820** [820–830] | 1290 [1160–1310] | 1270 [1060–1320] |
| RSS, 3 foots → end (MB) | 44.2 → 63.8 | 87.3 → 109.8 | 80.8 → 104.2 | 110.6 → 125.4 | 110.5 → 125.3 |
| startup, IPC answering (ms) | 14.6 | 43.8 | 38.6 | 125.6 | 121.8 |

The DIAG pass counted frames for the pixman, read-back gles and niri
variants (not the gpu build). From it:
- **Relayout**: 201 frames for scoot (one per action) against niri's
  615–626 (about three per action). Per frame that is 8.78 ms (pixman),
  2.53 ms (gles read-back) and 0.94–1.01 ms (niri).
- **Animate**: scoot-pixman presented 41 frames/s, scoot-gles 51 and niri
  54. That is 7.10, 2.47 and 2.35–2.38 ms per frame.
- **Pointer**: niri presented 628 frames, 1.2 ms each. Nested scoot
  presents none, because the host draws the pointer.

What changes on a real GPU, against the dev VM's llvmpipe table in
`docs/benchmarks.md`:
- **niri's per-frame cost drops 6–15x** (14.2 → 0.94 ms relayout,
  14.8 → 2.35 ms animate).
- **scoot-gles on the read-back path now costs about the same per animated
  frame as niri** (2.47 against 2.35 ms), and about 2.7x more per relayout
  frame, where niri draws three frames for scoot's one.
- The **gpu build (dma-buf to the host) used the least total CPU for
  relayout and animate**: relayout 250 ms against niri's 580, animate 820
  against 1290. It was not the lowest everywhere: pointer 100 ms against
  pixman's 90, and shot-grim 40 against the read-back tier's 30. (The DIAG pass did not include the gpu build, so there is no frame
  count for it.)
- **scoot-pixman is now the expensive one.** At 1600x1000 it spends 2.3x
  the CPU of niri while animating, and it delivers fewer frames (41/s
  against 54). Without a GPU, pixman remains the right default. With one,
  it is not.
- Memory at rest is lowest for pixman (44 MB with three foots). The two
  GLES scoot builds use 81–87 MB and niri uses 110 MB. None of them grew by
  a frame per capture (the PR #238 fix holds).

**Part B (`--tty` on the real panel, 2560x1600, scale 1.5 for both).** niri
ran with `output "eDP-1" { scale 1.5; }` added to the benchmark config.
Without it niri picks its own scale and the comparison would not be
like-for-like. Its logical size was 1706x1066, against scoot's 1707x1067.
Each session was one of scoot (dumb + pixman), scoot-gpu (`--renderer
gles`), niri with animations off, and niri with animations on. They ran in
that order, twice, each alone on VT 2 while it was measured. There is no
read-back GLES column on `--tty`: that tier does not exist there, because
the default build's `--tty --renderer gles` keeps pixman with a warning.
So the two scoot tiers are dumb + pixman and GPU scanout. Three `foot`s
were spawned through each compositor's own IPC, `xwayland-satellite` was
off (checked), and scoot ran without `--xwayland`. `sample.sh` was run by
PID from SSH rather than from a `foot` inside the session: it measures the
same process, and the damage clients run as the same user against the same
sockets. ydotool went through a private `ydotoold` (a uinput device). Each
cell is round 1, then round 2:

| scene | scoot-pixman | scoot-gpu | niri, anim off | niri, anim on |
| --- | --- | --- | --- | --- |
| idle 20 s | 0, 0 (0 wakeups) | 0, 0 (0 wakeups) | 0, 10 (65 wakeups) | 0, 0 (74 wakeups) |
| relayout 10 s (focus left/right at ~10 Hz each) | 4390, 4350 | **270, 270** | 470, 460 | 490, 470 |
| pointer 10 s (ydotool ±800 px, as fast as it forks) | 3470, 3160 | **960, 920** | 2210, 2220 | 2210, 2350 |
| shot-ipc, 10 captures | 270, 320 | 200, 250 | 420, 440 | 410, 430 |
| shot-grim, 10 captures | 150, 150 | 120, 120 | 140, 130 | 120, 130 |
| RSS after 3 foots → after shots (MB) | 93.6 → 110.3 | 101.6 → 135.2 | 126.8 → 159.9 | 126.8 → 152.4 |

- **On the real panel, scoot-gpu used the least total CPU for relayout
  and pointer motion.** These are totals: `--tty` has no frame counts, so
  this is not a per-frame comparison. Through `grim` it tied niri
  (anim on) in round 1 (120 = 120). `t9b.sh` ran its scenes back to back, with no idle wait between them.
  Relayout costs 2.7% of a core against niri's 4.6–4.9%. Pointer motion
  costs 9.2–9.6% against niri's 22–23.5%. Both compositors draw the
  pointer here, and neither can use a cursor plane, because `apple,dcp`
  has none.
- **scoot-pixman at 2560x1600 is the most expensive**: 44% of a core for
  relayout and 32–35% for pointer motion. This matches Test 4's dumb-tier
  numbers.
- **The pointer row compares CPU per second, not per event.** ydotool
  forks once per event, so its rate is whatever the machine manages, and
  it was not counted. scoot-pixman took fewer wakeups (19.5k–21.6k) than
  scoot-gpu and niri (29k–31k). That suggests it simply served fewer
  events, which would understate its cost.
- **Screenshots**: the IPC capture costs 20–25 ms of CPU per shot on
  scoot-gpu, 27–32 on pixman and 41–44 on niri. niri encodes on an exiting
  thread, so its cost only shows in the process column (thread-sum
  62–67 ms against 410–440 ms process, as on the VM). Through `grim`, all
  four are within 12–15 ms per capture.
- **Memory across the 20 captures**: scoot-gpu's RSS rose by 33 MB, about
  two frames at 2560x1600. niri's rose by 33 MB (anim off) and 26 MB (anim
  on). None of them climbed by
  a frame per capture (10 frames would be about 164 MB), so the PR #238
  fix holds on real hardware. The asahi driver reports no `drm-total-`
  lines in fdinfo, so the GPU-memory half of that check was skipped, as
  the runbook allows.
- Idle: neither scoot tier woke even once in 20 s. niri woke 65–74 times
  and used at most one 10 ms tick.

**Not measured**: input-to-present latency (the method is still open; see
the ticket), and frame counts on `--tty` (no DIAG equivalent there).
Grim screenshots of each real-panel session are kept with the run
(`t9b-*-panel.png`): the same three half-width columns, two of them on
screen, in both compositors.

## Test 10 — does the GPU driver keep a copy of each imported dma-buf plane

Why: scoot counts the file descriptors each client makes it hold, and a
GLES renderer can hold one more per dma-buf plane it imports. On the dev
VM's software renderer (llvmpipe) it does: a three-plane `YU12` buffer costs
scoot six fds, not three. scoot measures this once per session, around the
first import, logs the answer, and counts the copies against the client
(`docs/protocols.md`, "Per-client limits on what scoot keeps"). A hardware
driver is expected to import into a GEM handle and keep no fd, which would
log `copies_per_plane_per_output=0`. That expectation is reasoned from
Mesa's source, not measured; this machine is the check.

The log line alone is not enough. scoot measures right around the import,
so a driver that makes its copy later, on the first frame that draws the
buffer, would log 0 and still hold the copies uncounted. So this compares
scoot's real dma-buf fds with a client running against what the logged
number predicts.

```sh
mkdir -p /tmp/fx
# on a free VT, as in Test 4
RUST_LOG=scoot=info scoot --tty --renderer gles > /tmp/fx/t10.log 2>&1 &
sleep 3
export WAYLAND_DISPLAY=$(grep -o 'wayland-[0-9]*' /tmp/fx/t10.log | head -1)
ls -l /proc/$(pidof scoot)/fd | grep -c /dmabuf: > /tmp/fx/t10-dmabuf-fds.txt   # before
# a GPU client that renders through dma-bufs, with its wire traffic logged
WAYLAND_DEBUG=1 vkcube --wsi wayland 2> /tmp/fx/t10-vkcube.trace &
sleep 8   # let it draw a few hundred frames
ls -l /proc/$(pidof scoot)/fd | grep -c /dmabuf: >> /tmp/fx/t10-dmabuf-fds.txt  # running
grep -c 'zwp_linux_buffer_params_v1#[0-9]*.add(' /tmp/fx/t10-vkcube.trace > /tmp/fx/t10-planes.txt
grep -c 'wl_buffer#[0-9]*.destroy()' /tmp/fx/t10-vkcube.trace >> /tmp/fx/t10-planes.txt
grep 'learned how many fds the renderer keeps' /tmp/fx/t10.log
kill %2; sleep 1
ls -l /proc/$(pidof scoot)/fd | grep -c /dmabuf: >> /tmp/fx/t10-dmabuf-fds.txt  # after
```

What to compare: let `N` be the logged `copies_per_plane_per_output`, `P`
the first line of `t10-planes.txt` (planes the client added), and `B`,
`R` the first two lines of `t10-dmabuf-fds.txt` (before, running). With
the client still holding every buffer it made (vkcube keeps its swapchain;
the second line of `t10-planes.txt` should be 0 or small),
`R - B` should be about `P x (1 + N)`, give or take the few dma-bufs scoot
allocates for itself once it starts drawing (on the dev VM, 3). If it is
clearly more -- roughly `P x (1 + N + 1)` -- the driver keeps copies scoot
did not see at import, and scoot is under-counting on this hardware: send
all four files. If `N` is 0 and `R - B` is about `P`, the driver keeps
nothing and scoot charges nothing extra, which is the expected result. The
third line of `t10-dmabuf-fds.txt` should be back near `B` once the client
has quit.

### Results, 2026-09-25 — the AGX driver keeps one fd per plane, and scoot counts it

Same machine, build and config as Test 5's results.

**`dmabuf: learned how many fds the renderer keeps of each imported plane
copies_per_plane_per_output=1`.** That is the opposite of the expectation
above, which was reasoned from Mesa's source: the hardware driver keeps a
copy, just like llvmpipe does. The fd accounting was then checked by dma-buf
identity rather than by raw count. Each snapshot lists the `/dmabuf:` fds
in `/proc/<scoot>/fd`, grouped by inode (the same inode means the same
buffer):

Four sessions were counted: the combined Test 6/7 session (`t67.sh`), two
dedicated reruns without screenshots (`t10.sh` tiled, `t10b.sh` with a
fullscreen phase), and a third rerun that also took an IPC screenshot
while tiled and again while fullscreen (`t10c.sh`). The dedicated reruns:

| moment | dma-buf fds | what they are |
| --- | --- | --- |
| before any client | 3 | 1 swapchain buffer (2560x1600x4 B) x 3 fds |
| `vkcube` running, tiled, at 4 s, 8 s and 12 s | 17 | 3 swapchain buffers x 3 fds, plus **4 vkcube buffers x 2 fds** |
| 1 s after vkcube quit (`t10.sh` tiled; `t10b.sh` fullscreen) | 9 | 3 swapchain buffers x 3 fds |
| `es2gears_wayland` running | 13 | 9 own, plus 2 gears buffers x 2 fds |
| after es2gears quit | 9 | 9 own |

**While a client runs, the accounting is exact.** With `P = 4` live
one-plane buffers (vkcube's swapchain; the trace shows four `add(` per
rebuild and the old four destroyed each time) and `N = 1`, the client's
share is `P x (1 + N) = 8`, which is what was measured. Nothing is held
that scoot did not see at import time. The copy is a second fd for the
same dma-buf (same inode), so it costs an fd slot but no memory.

**After a client quits, the picture is less clean, and one extra fd is
unexplained.**
- In the combined session, `t10-dmabuf-fds.txt` reads `3 / 17 / 10`:
  before the client, while vkcube was running tiled, and 2 s after it quit
  from fullscreen. The 17 taken while it was fullscreen is a separate
  file, `t10-dmabuf-fds-fs.txt`. So the after-quit count was **10**, one
  above the steady 9. What held the tenth fd was not identified.
- The two dedicated reruns read 9 at 1 s and stayed there, but they took no
  screenshots, while the combined session did (one tiled, one fullscreen).
  Those two runs therefore cannot rule out a capture holding the fd.
- The rerun that did take screenshots (`t10c-counts.tsv`) does not
  reproduce "a capture holds it": each screenshot left the count unchanged
  (17 before and after the tiled one, 18 before and after the fullscreen
  one). It shows two other things instead:
  - After vkcube quit from fullscreen, **one fd of one vkcube buffer
    stayed open** (11 fds at 1, 2 and 4 s). It closed only after two
    pointer moves made scoot draw another frame, which brought the count
    to 10. So the last frame's client buffer can outlive its client until
    the next redraw: bounded to that frame, but not released "within 1 s".
  - **scoot's own dma-buf fd set is not constant.** During the fullscreen
    phase one swapchain buffer went from 3 fds to 1 and a new buffer
    appeared with 3, so scoot's own share settled at 10, not 9.

  Either effect could account for the combined session's 10. Which one it
  was is not known.

So: charging each client for its planes' copies is correct on this
hardware, and the "hardware drivers are expected not to" line in
`docs/protocols.md` was wrong for AGX; it is corrected in the same change
as this section. Release after quit is prompt when scoot keeps drawing,
but it is not guaranteed within any fixed time: one fd of the last frame's
buffer can wait for the next frame. The raw counts are kept with the runs
(`t10-dmabuf-fds.txt`, `t10-dmabuf-fds-fs.txt`, `t10-counts.tsv`,
`t10b-counts.tsv`, `t10c-counts.tsv`).

## Test 11 — output remove/restore, reconnected modeset, multi-head mode change (dumb tier)

Why: PR #249 (restore windows, workspaces and binds when a monitor
reconnects) merged with its live leg unverified — "needs physical hands".
The virtual-pull rig (proven 2026-09-26, see `ASAHI.local.md`) removes the
hands: forcing DP-1 `off` plus a synthetic udev change event is an unplug,
forcing it `on` plus the same event is a replug, and the kernel state is
readable throughout (`status`, `modes`, the `force` file itself). This runs
the #249 runbook for real — remove with windows on DP-1, return, positional
binds — plus the reconnected-modeset check and a multi-head mode change,
all on the dumb tier (`--renderer gles` untouched; that runtime add is a
separate ticket).

Built on the machine itself (native aarch64) from `main` at `326edd0`,
with `nix build .#scoot .#scootctl`:

| binary | store path | sha256 |
| --- | --- | --- |
| `scoot` | `/nix/store/flpc4ym2bkdr5jpw05i1z3hpjw0hx8bp-scoot-0.1.0` | `99fdb826f37dabe9bc260aa30cfd4f60417d079a8a469389f01726b06f93f7bb` |
| `scootctl` | `/nix/store/1nr7xs7zmy8a9lbadljb1kn9gvzf95bh-scootctl-0.1.0` | `681386457ee620d8fc4cce013b89f1e927e332a1eade1eea476e36925efc9377` |

Machine: Apple M2 (Mac14,2 / j413), NixOS 26.11, kernel 7.1.13
(fairydust), eDP-1 (2560x1600, its only mode) + DP-1 (21 modes, monitor
awake throughout the run). Sessions on VT 2 against a private `seatd`
(`~/fx/vt-run.sh` / `~/fx/vt-stop.sh`), config `[output] scale = 1.5` only
(`~/fx/test.toml`), socket `/run/user/1000/scoot-vp.sock`. Raw logs,
window snapshots, `wlr-randr` outputs and screenshots stay on the machine
under `~/fx/vpull-dumb/` and `~/fx/vpull-mode/`; build log at
`~/fx/build-326edd0.log`.

Rig correction worth recording: `/sys/kernel/debug/dri/2` is a symlink
to `soc:display-subsystem` — both force paths are the same file, so no
kernel mechanism distinguishes writes by path. One write to the alias
path appeared to echo through `tee` but read back `unspecified` (cause
unknown); the retry latched. Every force value below was verified by
reading the file back — that read-back discipline, not the path, is
what makes the rig trustworthy.

### Leg 1 — remove: force DP-1 `off`, windows adopted to the panel

Baseline: `--tty` session up (`wayland-1`, zero `ERROR` lines), both heads
lit — eDP-1 id 1 (2560x1600@60), DP-1 id 2 (1920x1080@60, at logical
x=1707). Two `foot` windows spawned and carried to the second screen with
`move-window-to-output-index 1`: id 1 at x=1719, id 2 at x=2353, both
`output: 2`, `visible: true`, id 2 focused. Screenshots of both heads
(`before-out1.png`, `before-out2.png`).

```sh
echo off | sudo tee /sys/kernel/debug/dri/2/DP-1/force   # reads back: off
# status still connected/21 modes here -- the force needs a redetection
sudo udevadm trigger --subsystem-match=drm --action=change
# 5 s later: status=disconnected, 0 modes
scoot msg outputs   # [{"id":1,"name":"eDP-1"}] -- DP-1 gone
scoot msg windows   # id 1 + id 2, both output: 1, visible: false, focused: false
```

The compositor saw the same transition its udev monitor delivers on a real
pull:

```
INFO scoot::compositor::tty::hotplug: drm: this connector went away connector=DP-1
INFO scoot::compositor::tty::hotplug: drm: a display went away; removing its output output=2
```

`wlr-randr` lists only eDP-1 afterwards. The eDP-1 screenshot is
byte-identical to the baseline empty-panel shot (sha256
`409a0f30…51444` both) — the adopted workspace stays inactive, so nothing
piled onto the visible desktop. Observed, not judged: with nothing on the
panel, both adopted windows report `focused: false`; there is no window on
the active workspace to hold focus, and the design never moves it.

**Leg 1 result: pass.** Output removed, both windows adopted to eDP-1 and
still open.

### Leg 2 — return: force `on`, workspaces restored, binds follow

```sh
echo on | sudo tee /sys/kernel/debug/dri/2/DP-1/force    # reads back: on
# status still disconnected/0 modes until the trigger
sudo udevadm trigger --subsystem-match=drm --action=change
# 6 s later: status=connected, 21 modes
scoot msg outputs   # [{"id":1,"name":"eDP-1"},{"id":3,"name":"DP-1"}]
scoot msg windows   # id 1 + id 2, both output: 3, visible: true
```

The returned monitor comes back under a **fresh id (3, not 2)**, exactly
the case #249 was built for, and the restore fired:

```
INFO scoot::compositor::tty::hotplug: drm: driving a newly connected display connector=DP-1 crtc=crtc::Handle(68) width=1920 height=1080 scanout="dumb"
INFO scoot::compositor::tty::hotplug: drm: a display was connected; added an output for it connector=DP-1 output=3 width=1920 height=1080
INFO scoot::compositor::reconnect: a display came back; restored its workspaces connector=DP-1 output=3 windows=2
INFO scoot::compositor::tty::dumb: drm: modeset (full commit)
```

Column order preserved exactly (id 1 x=1719 w=622, id 2 x=2353 w=622 —
identical rects to the baseline), both `visible: true` (the restored
workspace is active again). The returned head's screenshot shows both
foot windows with the same shell content as the baseline (a ~300-byte PNG
difference is the cursor blink phase, verified by eye); the panel shot is
unchanged. Fresh full-commit modeset, no stale frames.

Bind proof, through real key injection of the **default** binds (a window
was placed on each screen first so both directions are observable):

- `super+shift+period` with a focused window on eDP-1 → the window lands
  on output 3 with focus following it.
- `super+period` with focus on eDP-1 → focus moves to the DP-1 window on
  output 3; `super+comma` → focus returns to the eDP-1 window.

**Leg 2 result: pass.** Fresh-`OutputId` restore by connector identity,
active workspace back, both positional binds reach the returned monitor.

### Leg 3 — multi-head mode change, and the runtime refusal that is by design

First the negative half, against the running session: `wlr-randr` offers
DP-1 exactly one mode (1920x1080, preferred + current), and
`wlr-randr --output DP-1 --mode 1280x720` answers `unknown mode`, rc=1.
That is the documented shape — `wlr-output-management-v1` is read-only
(`README.md`), per-output geometry lives in the config file
(`docs/backlog/core/per-output-scale-mode.md`), and scoot advertises one
mode per output by construction (`set_mode` in
`crates/scoot/src/compositor/headless.rs`: preferred + current, nothing
else). There is no IPC mode path either. The refused session carried on
undisturbed at 1920x1080.

The positive half is the startup `--mode` path, which is the real modeset
mechanism: session restarted with `--mode 1280x720` (a mode DP-1 offers;
eDP-1 offers only 2560x1600):

```
WARN scoot::compositor::tty::gpu: drm: connector offers no mode of the requested size; using its preferred mode connector=eDP-1 width=1280 height=720
INFO drm_atomic:create_surface{crtc=crtc::Handle(68) mode=Mode { name: "1280x720", clock: 74250, ... }}
INFO scoot::compositor::tty: drm: driving this device path=/dev/dri/card2 connector=eDP-1 crtc=crtc::Handle(50) width=2560 height=1600 scanout="dumb"
INFO scoot::compositor::tty: drm: driving this device path=/dev/dri/card2 connector=DP-1 crtc=crtc::Handle(68) width=1280 height=720 scanout="dumb"
```

(`Unable to become drm master, assuming unprivileged mode` appears once in
this log and once in leg 1's — routine, not caused by `--mode`.)

`outputs` reports DP-1 at logical 854x480; `wlr-randr` (itself the client
observing `wl_output.mode`/`done`) lists 1280x720 as preferred + current.
A `foot` spawned and carried to the second screen maps `visible: true`,
focused, and its screenshot shows a correct layout at the new mode with
the focus ring and no garbage; the panel is untouched.

**Leg 3 result: pass**, with the premise corrected: no runtime mode switch
exists to test (refused by design, refusal itself verified harmless); the
`--mode` startup modeset is the change path and works per connector.

### Teardown and final box state

`~/fx/vt-stop.sh`: active VT back to 1, nothing left running,
`/run/seatd.sock` gone, the user's tty1/seat0 login untouched. Then
`sudo reboot` (remote-safe, same generation) to clear the sticky
force override, and re-verification: kernel 7.1.13, force `unspecified`,
eDP-1 connected, no compositor running, no strays.

One honest deviation: DP-1 reads `disconnected`/0 modes after the final
reboot — the monitor entered standby during the ~80 s with no signal and
dropped HPD for real (force `unspecified`, so no override is masking
anything). A forced-`on` probe still read `disconnected`/0, i.e. the panel
answers nothing even driven, which is the documented "needs a power
button" state, not a stuck override; a second reboot cleared the probe's
`on` back to `unspecified`. All three legs ran while the monitor was
awake, so nothing above is affected — but the box was found with DP-1
awake and is left with it asleep.

### Results, 2026-09-26 — #249's live leg is proven, no product change needed

Remove → adopt, return → restore-by-identity under a fresh id with column
order and the active workspace back, positional binds reaching the
returned monitor, reconnected full-commit modeset with fresh pixels, and
the per-connector `--mode` modeset — all on the dumb tier at `326edd0`.
Nothing failed, so there is no fix ticket and no product diff; this
section is the proof ticket. The only surprise was the rig's (one
aliased-path write echoed without latching — same file via symlink,
cause unknown), recorded above so the next run verifies every write by
read-back.

## Test 12 — GPU-tier runtime add, real replug and multi-head mode change (physical hands)

Why: after Test 11 three hotplug paths were still unrun on hardware: a
GPU-tier runtime add (a fresh `DrmCompositor` and EGL context beside a
live one), a real-HPD remove/restore on the GPU tier (Test 11 used the
virtual-pull rig on the dumb tier), and a multi-head mode change on the GPU
tier. The DP-1 monitor was asleep all day and nothing remote could wake it
(`ASAHI.local.md`), so this run waited for someone at the machine. Every
transition below is a real power-on or cable pull. No `force` override was
written, so no reboot was needed afterwards.

Built on the machine from `main` at `87e1935` with
`cargo build -p scoot --features gpu-scanout`, run through `~/fx/dev.sh`
(which sources the flake dev shell's exported environment), into
its own target dir (`~/scoot-live-target`, `scoot` sha256 `57555773…9b1515d`).
Same machine, kernel, VT 2 + private `seatd` recipe and `[output] scale =
1.5` config as Test 11, `--renderer gles`. Scripts: `~/fx/live-start.sh`,
`live-snap.sh NAME` (outputs, windows, fd count, one screenshot per output)
and `live-stop.sh` for legs 1–2. Leg 3 used their parameterised copies
`live-start2.sh` / `live-snap2.sh` / `live-stop2.sh` with
`LIVE_DIR=live-mode SCOOT_EXTRA="--mode 1280x720"`. Every bind below was
sent as IPC key injection (`scoot msg key super+shift+period`), not a
physical keypress. A udev monitor and a 0.5 s DP-1 sysfs poller ran
throughout. Raw logs, snapshots and screenshots are under `~/fx/live/`
(legs 1–2) and `~/fx/live-mode/` (leg 3), one directory per phase.

### Leg 1 — runtime add on the GPU tier: power on a sleeping monitor

scoot came up with only eDP-1 (DP-1 `disconnected`, 0 modes), two foot
windows on it, 51 fds. Then the monitor's power button was pressed. The
sysfs poller read `connected`, 21 modes, at 01:02:57.60Z. scoot logged:

```
INFO scoot::compositor::tty::hotplug: drm: driving a newly connected display connector=DP-1 crtc=crtc::Handle(68) width=1920 height=1080 scanout="gpu"
INFO scoot::compositor::tty::hotplug: drm: a display was connected; added an output for it connector=DP-1 output=2 width=1920 height=1080
```

The first hotplug log line (01:02:57.571Z) was 50 ms ahead of the new output
(57.622Z), with a second EGL context and GLES renderer created beside the
live one. `outputs` lists DP-1 as id 2 at logical x=1707, 1280x720
(scale 1.5). `super+shift+period` carried window 2 there. Its screenshot
shows a correct half-width column with the focus ring. fds: 51 before, 63
after the add, 66 once window 2 was on the new head.

**Leg 1 result: pass.**

### Leg 2 — real unplug and replug on the GPU tier

Cable pulled with window 2 focused on DP-1:

```
INFO scoot::compositor::tty::hotplug: drm: this connector went away connector=DP-1
INFO scoot::compositor::tty::hotplug: drm: a display went away; removing its output output=2
```

The output was removed 57 ms after the change event (01:03:30.390Z →
.447Z). Window 2 was adopted by eDP-1 (`output: 1`, `visible: false`, on
the adopted background workspace: workspace 2 on eDP-1 per `remove_output`'s
`adopted_at`, derived from the code, since `windows` carries no workspace
field) and focus fell to
window 1. fds went back to exactly 51, the count before DP-1 existed.
**Observed by the person at the machine: the window "feels like it
disappears".** That is the designed adopt-as-background-workspace
behaviour. It is filed as a UX ticket,
[`unplug-adopted-windows-invisible`](docs/backlog/core/unplug-adopted-windows-invisible.md),
not as a failure of this leg.

Cable back in:

```
INFO scoot::compositor::tty::hotplug: drm: driving a newly connected display connector=DP-1 crtc=crtc::Handle(68) width=1920 height=1080 scanout="gpu"
INFO scoot::compositor::tty::hotplug: drm: a display was connected; added an output for it connector=DP-1 output=3 width=1920 height=1080
INFO scoot::compositor::reconnect: a display came back; restored its workspaces connector=DP-1 output=3 windows=1
```

The monitor returned under a fresh id (3), with window 2 restored to it,
`visible: true` and the same geometry as before. The default
`super+shift+period` then moved
window 1 onto output 3, so positional binds reach the returned head on
this tier too.

fd counts per phase, raw (`~/fx/live*/N-*/fds`):

| session | start | add | window moved to DP-1 | unplug | replug | bind | final |
| --- | --- | --- | --- | --- | --- | --- | --- |
| legs 1–2 | 51 | 63 | 66 | 51 | 63 | 66 | 66 |
| leg 3 (`--mode`) | 66 (both heads) | — | 66 | 51 | 63 | — | 63 |

Every remove returns to exactly 51, so nothing leaks per head. Two 3-fd
differences are **unexplained**. The replug with window 2 restored reads 63,
while the same state before the unplug read 66. Leg 3 starts at 66 with two
heads and no window on DP-1, against 63 after the runtime add. This may be
Test 10's fd that lingers until the next redraw. It was not chased.

**Leg 2 result: pass.** No `ERROR` lines and no foot `unknown output`
warnings in either session.

### Leg 3 — multi-head mode change on the GPU tier, and replug keeps it

Session restarted with `--mode 1280x720` (DP-1 offers it, eDP-1 does not):

```
WARN scoot::compositor::tty::gpu: drm: connector offers no mode of the requested size; using its preferred mode connector=eDP-1 width=1280 height=720
INFO scoot::compositor::tty: drm: driving this device path=/dev/dri/card2 connector=eDP-1 crtc=crtc::Handle(50) width=2560 height=1600 scanout="gpu"
INFO scoot::compositor::tty: drm: driving this device path=/dev/dri/card2 connector=DP-1 crtc=crtc::Handle(68) width=1280 height=720 scanout="gpu"
```

`wlr-randr`, run against the live session, listed DP-1 at 1280x720
(preferred + current). Its output was read live and not saved. `outputs` gives
logical 854x480, and a foot carried there renders correctly at that mode.
Then a real unplug (output removed in 30 ms, fds back to 51) and replug:

```
INFO drm_atomic:create_surface{crtc=crtc::Handle(68) mode=Mode { name: "1280x720", ... }}
INFO scoot::compositor::tty::hotplug: drm: a display was connected; added an output for it connector=DP-1 output=3 width=1280 height=720
INFO scoot::compositor::reconnect: a display came back; restored its workspaces connector=DP-1 output=3 windows=1
```

The hotplug-add path honours `--mode` (`hotplug.rs` reads
`requested_mode`). The returned head came back at 1280x720, not its
preferred 1920x1080, with window 2 restored to it.

**Leg 3 result: pass.** One cosmetic finding: the eDP-1 "offers no mode of
the requested size" `WARN` is logged once at startup and again on every
hotplug event (twice here), because `reconfigure` re-picks the mode for
every driven head. It is harmless and not ticketed.

### Teardown and final box state

`live-stop.sh` after each session: VT 1, nothing left running, seatd gone.
No `force` override was ever written. Both
`/sys/kernel/debug/dri/2/{DP-1,eDP-1}/force` read `unspecified` afterwards
(read with `sudo cat`), so no reboot was needed.
DP-1 was left connected and awake.

### Results, 2026-09-26 — the GPU-tier hotplug paths are proven, no product change needed

A GPU-tier runtime add, a real-HPD remove/adopt and restore by identity
under a fresh id, binds that follow the returned head, and a two-head
`--mode` modeset that survives a replug all passed on the GPU scanout tier
at `87e1935`. Of the hardware-bound hotplug paths, only #48's `MoveTo`
fallback remains. It needs the only lit screen to be pullable, and the
panel is not. The one product finding is UX, not correctness: adopted
windows are hard to find (ticket above).

## Test 13 — GPU-tier virtual-pull hotplug add + multi-head mode change (remote, no hands)

Why: Test 12 proved the GPU-tier runtime add and multi-head mode change
with physical hands at `87e1935`. This re-proves both at current `main`
via the virtual-pull rig (DP-1 `force` + synthetic `udevadm trigger`,
Test 11's recipe) — no hands needed — and keys the evidence to the new
commit. Dumb-tier re-proof is out of scope (Test 11 owns it); eDP-1 was
never forced; no box config was changed (standing scope).

Built on the machine itself (native aarch64) from `main` at `206d0a3`
(`git archive HEAD` shipped over ssh, extracted to `~/scoot-t13`; the
tree holds PR #272's `shm-viewport-upscale-edge-fade-done.md`, so it is
that commit), `cargo build -p scoot --features gpu-scanout` (debug) into
its own target dir, build log at `~/fx/build-t13.log` (zero warnings,
zero errors):

| binary | path | sha256 |
| --- | --- | --- |
| `scoot` | `~/scoot-t13-target/debug/scoot` | `0321ea416bfe8097d065a38547d103387e9b4b6db6e99a1bcdd4716449d3379e` |

`ldd` names `libgbm.so.1`, so the scanout tier is in. Machine: Apple M2
(Mac14,2 / j413), NixOS 26.11, kernel 7.1.13 (fairydust), eDP-1
(2560x1600, its only mode) + DP-1 (21 modes, monitor awake throughout
both legs). Sessions on VT 2 against a private `seatd`
(`~/fx/vt-run.sh` / `~/fx/vt-stop.sh`), config `[output] scale = 1.5`
only (`~/fx/test.toml`), socket `/run/user/1000/scoot-t13.sock`,
`RUST_LOG=info,scoot::compositor::tty=debug`, Mesa paths from
`~/fx/mesa-env.sh` (`/run/opengl-driver` still absent). Scripts:
`~/fx/t13-start.sh` / `t13-snap.sh` (outputs, windows, fd count,
`wlr-randr`, one screenshot per output) / `t13-stop.sh`. Raw logs,
snapshots, `wlr-randr` outputs and screenshots stay on the machine
under `~/fx/t13-leg1/` and `~/fx/t13-leg2/`.

Rig finding worth recording: a `force` write plus a synthetic trigger
with **nothing running does not flip sysfs status** — `off` latched
(read-back verified) but DP-1 stayed `connected`/21 modes through the
trigger and 20 s of polling, and the session started afterwards drove
DP-1 anyway (both heads `scanout="gpu"` at 01:58:02Z). The force only
took effect once a *running* session's reconfigure re-probed the
connector off the synthetic udev change (Test 11 never hit this because
its session was already up when it forced off). So the trustworthy
sequence is: session up first, then force + trigger. Every force value
below was verified by read-back.

### Leg 1 — GPU-tier remove then runtime add (virtual-pull)

With the both-heads session running, `force=off` + trigger →
`disconnected`/0 modes within ~2 s, and the compositor followed:

```
INFO scoot::compositor::tty::hotplug: drm: this connector went away connector=DP-1
INFO scoot::compositor::tty::hotplug: drm: a display went away; removing its output output=2
```

`outputs` lists only eDP-1 id 1; `wlr-randr` lists only eDP-1; fds back
to exactly 51 (Test 12's invariant holds: every remove returns to 51).

Then `force=on` + trigger → `connected`/21 modes, and the GPU-tier add:

```
INFO egl{native="/dev/dri/card2" platform="PLATFORM_GBM_KHR" version=(1, 5)}:egl_context{ptr=105474095193680}:renderer_gles2: ...
INFO scoot::compositor::tty::hotplug: drm: driving a newly connected display connector=DP-1 crtc=crtc::Handle(68) width=1920 height=1080 scanout="gpu"
INFO scoot::compositor::tty::hotplug: drm: a display was connected; added an output for it connector=DP-1 output=3 width=1920 height=1080
```

A fresh EGL display and GLES renderer beside the live ones (new
`egl_context` pointer), the head on CRTC 68 with `scanout="gpu"`, under
a fresh id 3. `outputs` names eDP-1 id 1 + DP-1 id 3; fds 51 → 63
(Test 12's add count exactly). `wlr-randr` afterwards names both heads:
eDP-1 2560x1600 and DP-1 1920x1080, each preferred + current, DP-1 at
position 1707,0, scale 1.5.

Windows placeable, captures correct: `msg key super+shift+period`
carried window 2 to output 3 (`output: 3`, `visible: true`, focused).
Both heads' IPC screenshots were read by eye: eDP-1 shows the remaining
foot with its ring, DP-1 shows the moved foot with the focused (blue)
ring and the same shell content — correct layouts, no garbage
(`~/fx/t13-leg1/3-window-on-dp1/out{1,3}.png`; sha256 `10d34633…` /
`71eb4d28…`, changed from the pre-move pair as the window crossed).

**Leg 1 result: pass.**

### Leg 2 — multi-head mode change on the GPU tier, replug keeps it

Session restarted with `--mode 1280x720` (DP-1 offers it, eDP-1 does not):

```
WARN scoot::compositor::tty::gpu: drm: connector offers no mode of the requested size; using its preferred mode connector=eDP-1 ...
INFO scoot::compositor::tty: drm: driving this device path=/dev/dri/card2 connector=eDP-1 crtc=crtc::Handle(50) width=2560 height=1600 scanout="gpu"
INFO scoot::compositor::tty: drm: driving this device path=/dev/dri/card2 connector=DP-1 crtc=crtc::Handle(68) width=1280 height=720 scanout="gpu"
```

`wlr-randr` — the client-visible `wl_output.mode` — lists DP-1 at
1280x720 preferred + current beside eDP-1's native mode. The runtime
refusal half, for the record: `wlr-randr --output DP-1 --mode 1920x1080`
answers `unknown mode`, rc=1, and the session carries on undisturbed at
1280x720 (read-only `wlr-output-management-v1` by design, `README.md`;
no IPC mode path exists either). A foot carried to output 2 maps
`visible: true`, focused, and renders correctly at the new mode.

Then a virtual-pull replug at that mode: `force=off` + trigger →
`disconnected`, output 2 removed; `force=on` + trigger → `connected`/21:

```
INFO drm_atomic:create_surface{crtc=crtc::Handle(68) mode=Mode { name: "1280x720", clock: 74250, ... }}
INFO egl{native="/dev/dri/card2" platform="PLATFORM_GBM_KHR" version=(1, 5)}:egl_context{ptr=122183987775504}:renderer_gles2: ...
INFO scoot::compositor::tty::hotplug: drm: driving a newly connected display connector=DP-1 crtc=crtc::Handle(68) width=1280 height=720 scanout="gpu"
INFO scoot::compositor::tty::hotplug: drm: a display was connected; added an output for it connector=DP-1 output=3 width=1280 height=720
INFO scoot::compositor::reconnect: a display came back; restored its workspaces connector=DP-1 output=3 windows=1
```

The hotplug-add path honours `--mode`: the returned head came back at
1280x720, not its preferred 1920x1080, with window 2 restored to it
(`visible: true`). `wlr-randr` confirms 1280x720 preferred + current;
the screenshot shows the restored foot correct at that mode (read by
eye, `~/fx/t13-leg2/2-replug-keeps-mode/out3.png`).

**Leg 2 result: pass.**

fd counts per phase, raw (`~/fx/t13-leg*/N-*/fds`):

| session | start (both heads) | remove | add | window on DP-1 | replug cycle |
| --- | --- | --- | --- | --- | --- |
| leg 1 (preferred) | 66 | 51 | 63 | 66 | — |
| leg 2 (`--mode`) | 66 | 51† | — | 69 | 63 (restored) |

† leg-2 remove 51 was observed live, not snapshotted (the snap script only ran for 0/1/2/9-final); the 69 window-on-DP-1 read is +3 over Test 12's 66, inside the known ±3 jitter class — both PASS verdicts stand (66 reproduces Test 12 exactly; removes return to exactly 51).

No jiffies/CPU sampled this run (functional legs only; Test 4 and the
phase-E numbers stand).

### Teardown and final box state

`t13-stop.sh` after each session: active VT back to 1, nothing left
running, `/run/seatd.sock` gone, the user's tty1/seat0 login untouched.
No `ERROR` or panic lines in either session log; zero foot
`unknown output` warnings in both. Then `sudo reboot` (remote-safe,
same generation) to clear the sticky `force=on`, and re-verification:
kernel 7.1.13, force `unspecified`, eDP-1 connected, no compositor
running, no strays.

One honest deviation, same as Test 11's: DP-1 reads `disconnected`/0
modes after the final reboot — the monitor entered standby during the
reboot window with no signal and dropped HPD for real (force
`unspecified`, so no override is masking anything; that read-back is
what distinguishes standby from residue). Both legs ran while the
monitor was awake, so nothing above is affected — but a future DP-1 run
needs a power-button press first.

### Observations (not failures, not ticketed)

- Smithay's `drm::surface::atomic: Failed to destroy old mode property
  blob: No such file or directory (os error 2)` WARN fires on every
  modeset (twice at startup, once per hotplug add), both sessions. Upstream
  atomic-surface noise, pre-existing, harmless.
- Gamma size is asymmetric: startup logs `crtc gamma size output=2
  size=256`, while a hotplug-added head logs `crtc reports an unusable
  gamma size; advertising 256 instead size=0`. Advertised 256 either way;
  out of scope, recorded for the next reader.
- The eDP-1 "offers no mode of the requested size" WARN logs once at
  startup and again on every hotplug event (three times here: startup,
  remove, add) — Test 12's cosmetic finding, reproduced exactly.
  Harmless, still not ticketed.

### Results, 2026-09-27 — both GPU-tier legs re-proven, no product change needed

GPU-tier runtime add (fresh `DrmCompositor` + EGL context beside the
live one, `scanout="gpu"`, fresh id, windows placeable, both captures
correct) and multi-head `--mode` modeset that survives a virtual-pull
replug all passed at `206d0a3`. Nothing failed, so there is no fix
ticket and no product diff; this section is the proof ticket. Of the
hardware-bound hotplug paths, only #48's `MoveTo` fallback remains (it
needs the only lit screen to be pullable, and the panel is not). The
per-output scale/mode surface is unchanged: still its own open ticket
(`docs/backlog/core/per-output-scale-mode.md`, priority low) — this run
asserts nothing about it beyond the `--mode` startup path above.

## Test 14 — battery/backlight uevents, and `cursor_hide_after_ms` on real hardware

Run 2026-09-29 on the Apple M2 (Mac14,2 / j413), NixOS 26.11, kernel
7.1.13 (fairydust), on the box's AC adapter, with DP-1 (1920x1080, awake, 21
modes) and eDP-1 both connected. Scoot is `main` at `b3f087b43`, built on
the machine (`git archive` shipped over ssh to `~/scoot-res`, `cargo build
--release -p scoot -p scootctl --features scoot/gpu-scanout` into
`~/scoot-res-target`, log `~/fx/build-res.log`). `ldd` names `libgbm.so.1`,
so the scanout tier is in.

| binary | sha256 |
| --- | --- |
| `release/scoot` | `4b76421fd3701041b8ce587cbac28a283db2704717400d128ca31a3486fef389` |
| `release/scootctl` | `f8a3e36a6a94f46a5b7c3158c96181fc674c788f55bc8924f93faed783aeff69` |

Raw logs are on the box, `~/fx/t14-*` (`t14-power.sh`, `t14-udev.log`,
`t14-poll.log`, `t14-cursor.sh`, `t14-all.sh`, per-run `.txt`/`.log`/
`-kms-*.txt`, `t14-drm_info.txt`).

### Part A — power_supply and backlight uevents (read-only)

Script `~/fx/t14-power.sh`: `udevadm monitor -k -u -p
--subsystem-match=power_supply --subsystem-match=backlight` for 300 s, a
sysfs poll every 10 s, and three backlight writes as root in the middle
(`107 -> 120 -> 90 -> 107`; the original, 107, was restored and re-read at
the end of the whole test).

**Attributes.** `macsmc-battery` (`type=Battery`, model `bq40z651`, at
`/sys/devices/platform/soc/23e400000.smc/macsmc-power/`): `capacity`
(0-100), `capacity_level`, `status` (`Full` seen), `present`, `health`,
`charge_now/full/full_design`, `energy_now/full/full_design` (µWh),
`voltage_now/min/max(+design)`, `current_now`, `power_now`, `temp`
(tenths of °C), `cycle_count`, `time_to_empty_now`, `time_to_full_now`,
`charge_behaviour` (`[auto] inhibit-charge`),
`charge_control_{start,end}_threshold`, `charge_term_current`,
`constant_charge_{current_max,voltage}`, `scope=System`, manufacture date,
serial. `macsmc-ac` (`type=Mains`): `online`, `input_power_limit`. Three
`tps6598x-source-psy-0-00{38,3a,3f}` supplies (one per USB-C port) also
exist. Backlight `apple-panel-bl` (`type=platform`, `scale=linear`):
`brightness`, `actual_brightness`, `max_brightness=509`, `bl_power`.

Facts a module must handle: at `status=Full` the SMC reports `capacity=100`
while `charge_now/charge_full` is about 96% (3702000/3840000 on the first read, 3697000/3843000 on a later one), so trust
`capacity`, not a ratio. `time_to_empty_now`/`time_to_full_now` read 0 when
not applicable. `actual_brightness` differs from `brightness` by rounding
(wrote 120, `actual` 119; wrote 90, `actual` 91).

**Do battery uevents fire on capacity change? Not answered.** The box was
on AC at `Full` the whole time and I could not unplug it. Over 300 s
(30 polls) `capacity`, `status`, `energy_now`, `charge_now`,
`current_now` and `power_now` never changed; only `voltage_now` (12786-12788 mV)
and `temp` (219-224) moved. **Zero `power_supply` uevents fired** in 300 s,
so *temperature and voltage jitter does not produce uevents*. Whether a
capacity step while discharging or a plug/unplug does could not be
observed here. The battery-module ticket's fallback (a slow timer, only
while discharging) stays the plan until someone can observe a discharge.
Plug/unplug (`macsmc-ac` `online`) is likewise unobserved.

**Do backlight writes emit uevents? Yes, one each.** Three sysfs writes as
root gave exactly three `KERNEL ... change ... (backlight)` events
(`SOURCE=sysfs`, `ACTION=change`, no `POWER_SUPPLY_*`-style value in the
properties, so the new level must be re-read from sysfs) and three matching
`UDEV` events, ~50 ms after the kernel ones. A consumer therefore does not
need to poll or to be the writer, for writes that go through sysfs (the path
logind's `SetBrightness` also takes). What was not observed: a change
made by the hardware/firmware itself (a brightness key handled by the SMC;
none was pressed), and inotify on the `brightness` file (no `inotifywait`
on the box; not tried). Side effect worth knowing: the change uevent carries
`SYSTEMD_WANTS=systemd-backlight@backlight:apple-panel-bl.service`, so
systemd's save/restore unit is pulled in on every change.

| question | answer | evidence |
| --- | --- | --- |
| battery attributes | listed above | interactive sysfs read, not archived (re-read 2026-09-29: `capacity=100`, `charge_now/charge_full` 3697000/3843000) |
| battery uevents on capacity change | **not observed** (AC, Full) | 0 `power_supply` uevents / 300 s |
| uevents on voltage/temp jitter | none | `t14-poll.log` vs `t14-udev.log` |
| backlight write emits change uevent | **yes, 1 per write** | 3 writes, 3 KERNEL + 3 UDEV |
| brightness restored | 107 (read back) | `final: brightness=107 actual=107` |

### Part B — does `cursor_hide_after_ms` unblock primary-direct (Test 5's trap)?

Scripts `~/fx/t14-cursor.sh` (one session) and `~/fx/t14-all.sh`.
VT 2, private seatd (`~/fx/vt-run.sh`), `scoot --tty --renderer gles`,
Mesa 26.2.2 env (`~/fx/mesa-env.sh`), config `[output] scale = 1.5` (unset)
or the same plus `[appearance] cursor_hide_after_ms = 1000` (set,
`~/fx/test-hide.toml`). mpv `--fs --cursor-autohide=no --vo=gpu
--gpu-context=wayland --hwdec=no --loop`, so **mpv keeps showing a pointer
and only scoot's hide can remove it** (Test 5 had mpv hide its own). Per
session: `scootctl pointer move 400 300`, `420 320`, idle, debugfs `dri/2/state`
(plane 35), two 10 s jiffies samples of the compositor
(`/proc/PID/stat` utime+stime, info logging), then one more pointer move,
debugfs 0.3 s later, 3 s idle, debugfs, one more sample. Two outputs are up
this time (eDP-1 2560x1600 id 1, DP-1 1920x1080 id 2); mpv went fullscreen
on eDP-1 (window `rect` 1707x1067, `output` 1).

**Plane inventory changed since Test 5.** `drm_info` on `card2`, CRTC 0:
the primary (35) plus **two** overlays (40, 45; fixed zpos 1 and 2, `LINEAR`
only, `AR30 AR24 AB24 NV12 NV16 NV24 P010 P210`, no opaque `X` formats),
still no cursor plane. Scoot logs `overlay_planes=2 cursor_planes=0` (Test 5
saw one overlay, on kernel 7.1.5). This kernel gives Test 5's overlay
question more room; see Part C.

**Result: pass.** With it set, mpv's fullscreen buffer goes direct with
the pointer still "shown" to the client; with it unset, it never does.

| run | clip | option | plane 35 idle (after pointer left alone) | 0.3 s after a motion | 3 s idle again | jiffies / 10 s (idle samples) |
| --- | --- | --- | --- | --- | --- | --- |
| clip-hide-r1 | 1080p30 `clip.mkv` | set | `XR30 2561x1601`, client fb, `crtc-pos 2561x1601+0+0` | `AR24 2560x1600` (scoot swapchain, composited) | `XR30 2561x1601` (direct) | 14, 13, 13 |
| clip-nohide-r1 | same | unset | `AR24 2560x1600` | `AR24` | `AR24` | 30, 32, 33 |
| clip-hide-r2 | same | set | `XR30 2561x1601` | `AR24` | `XR30 2561x1601` | 14, 14, 14 |
| clip-nohide-r2 | same | unset | `AR24 2560x1600` | `AR24` | `AR24` | 30, 31, 29 |
| hide-a (`t14-hide-a`) | 60 fps 2560x1600 `clip-panel.mkv` | set | `XR30 2561x1601` | `AR24` | `XR30 2561x1601` | 21, 21, 21 |
| panel-nohide | same | unset | `AR24 2560x1600` | `AR24` | `AR24` | 37, 37, 40 |

Compositor CPU with the option set is 13-14
jiffies against 29-33 with it unset for the 1080p30 clip (**about 55%
lower**), and 21 against 37-40 for the 60 fps panel-resolution clip
(**about 45% lower**). Test 5's direct figure for the 1080p30 clip was
10-11 with mpv hiding its own pointer; the 13-14 here is on a different
kernel with a second output and another Mesa, so do not compare the two
runs' absolute numbers.

**Trace evidence.** Two `RUST_LOG=info,scoot=debug,smithay::backend::drm::compositor=trace`
sessions (`t14-trace-hide`, `t14-trace-nohide`), same script and clip:

- set: 1036 `testing direct scan-out` lines, **all on `plane::Handle(35)`**,
  1036 `successfully assigned element`, 2 `test failed` (startup),
  `eligibility changed NotCovered -> NothingOpaqueCovers -> Eligible`
  (71 ms apart) and back to `NotCovered` when mpv closed.
- unset: **0** `testing direct scan-out` lines, `Eligible` reached once,
  nothing attempted on the primary. That is Test 5's trap, unchanged.

The trace runs cost more CPU (20 / 36 jiffies) than the info-level ones,
as expected; their numbers are not in the table above.

**What a reader should take from it.** The compositor-side hide works on
this hardware as the ticket's option 1 intended: motion re-shows the
pointer and the output composites for the whole delay (plane 35 back on
scoot's `AR24` fb; direct attempts stopped for 1.035 s in
`t14-trace-hide.log`, the measured return latency), and
it returns to the client fb once the pointer has sat still for the delay
(the debugfs read 3 s after motion showed the direct fb again). Not
measured: mpv's `presented` flags (no `WAYLAND_DEBUG` trace was taken this
time, so the `vsync | zero_copy` cross-check of Test 5 is missing; the
debugfs fb is the evidence; Test 15 took that trace), a photo of the
panel, and any client other than mpv. Option 2 (cursor on an overlay) is
untouched by this.

### Part C — overlay window candidates: nothing measurable without implementing

No window is marked `Kind::ScanoutCandidate`, so no window can ride an
overlay; the only thing a measurement could touch is the plane inventory,
which is now recorded (Part B: two overlays, `LINEAR`, no `X` formats).
Implementing it needs: the `ScanoutCandidate` marking, the capture
contract (`Captures::note_direct` for overlays), a per-surface tranche
steering a client to `LINEAR` in an overlay format, and a purpose-built
`AR24` `LINEAR` client (nothing seen on this machine qualifies). The new
two-overlay inventory means the cursor-on-overlay and window-on-overlay
options no longer have to fight for a single plane on this kernel, which
is worth recording on both tickets.

### State left behind

`fgconsole` 1, no `scoot`/`seatd`/`mpv` processes, `/run/seatd.sock` gone,
DP-1 `connected` with 21 modes and `enabled`, brightness 107. DP-1 was never
forced and eDP-1 was never touched beyond a normal `--tty` session; no
`nh os switch`, no reboot, no config change on the box.

## Test 15 — `zero_copy` in mpv's `presented` flags under `cursor_hide_after_ms`

Run 2026-09-29, 14:37-14:40 UTC, on the same Apple M2 (Mac14,2 / j413),
NixOS 26.11, kernel 7.1.13 (fairydust), Mesa 26.2.2, mpv 0.41.0, DP-1
(1920x1080, 21 modes) and eDP-1 both connected. Test 14 left one question
open: does the client see `zero_copy` when scoot's hide lets its buffer go
direct? This test answers it with a `WAYLAND_DEBUG=1` trace of mpv.

**Build: Test 14's binary, reused.** The docs commit this lands in sits on
`main` at `474ad20fa`. `git diff b3f087b43..474ad20fa -- crates/scoot
crates/scoot-core crates/scoot-ipc Cargo.toml Cargo.lock` is empty, so
the compositor is Test 14's build. It was not rebuilt. The sha256s were
re-read on the box before and after the runs, and match Test 14's table:

```
4b76421fd3701041b8ce587cbac28a283db2704717400d128ca31a3486fef389  /home/steve/scoot-res-target/release/scoot
f8a3e36a6a94f46a5b7c3158c96181fc674c788f55bc8924f93faed783aeff69  /home/steve/scoot-res-target/release/scootctl
```

**Recipe.** `~/fx/t15-zc.sh TAG CFG` runs one session and `~/fx/t15-all.sh`
runs four, alternating: `t15-hide-r1`, `t15-nohide-r1`, `t15-hide-r2`,
`t15-nohide-r2`. `~/fx/t15-state.sh` checks the box before the first
session and after each one: `fgconsole`, seatd socket, leftover processes,
DP-1/eDP-1 status and mode count, `loginctl`. Each session:

- Start scoot the way Test 14 did: `~/fx/vt-run.sh` on VT 2 with a private
  seatd, `scoot --tty --renderer gles --config CFG`, the Mesa env from
  `~/fx/mesa-env.sh` and libglvnd on `LD_LIBRARY_PATH`. Log at
  `RUST_LOG=info,scoot=debug,smithay::backend::drm::compositor=trace`.
- CFG is `~/fx/test.toml` (`[output] scale = 1.5`, called "unset" below) or
  `~/fx/test-hide.toml` (the same plus `[appearance] cursor_hide_after_ms =
  1000`, called "set").
- Run `WAYLAND_DEBUG=1 mpv --fs --cursor-autohide=no --vo=gpu
  --gpu-context=wayland --hwdec=no --loop --really-quiet
  --log-file=… ~/fx/clip.mkv 2> TAG-mpv.trace`. The clip is the 1080p30
  one. mpv never hides its own pointer.
- Phase **A**: 4 s after launch, with no pointer motion yet.
- Phase **B**: `scootctl pointer move 400 300`, then `420 320` (mpv's first
  pointer focus), 3 s settle, a 10 s window, and debugfs `dri/2/state` in
  the middle of that window.
- Phase **C**: 32 moves 0.25 s apart, so the pointer never sits still for
  the 1 s delay. Debugfs is read after the 16th move.
- Phase **D**: 3 s settle, a 10 s window, debugfs in the middle.
- Quit mpv, then `~/fx/vt-stop.sh`.
- Every phase boundary writes a `MARK` line to `TAG.txt`: local and UTC
  time, plus the line counts of the mpv trace and the scoot log at that
  instant. `~/fx/t15-analyze.sh TAG` (output in `TAG-analysis.txt`) splits
  both files at those counts. It then counts the last argument of each
  `wp_presentation_feedback….presented(` line against the scoot log's
  `testing direct scan-out` and `successfully assigned element … to
  plane::Handle(35)` lines. The flags are those Test 6 used: `9` is
  `vsync | zero_copy`, `1` is `vsync` only.
- Jiffies were not sampled. Trace logging plus `WAYLAND_DEBUG` distorts
  CPU, and Test 14 has the CPU numbers.

**Result: pass.** With the option set, mpv is told `zero_copy` on every
frame its buffer went direct, and on no other frame. With it unset, it is
never told `zero_copy`. Over whole sessions:

| session | option | `presented` flags `9` | flags `1` | scoot `successfully assigned … plane::Handle(35)` | `discarded` | other flag values |
| --- | --- | --- | --- | --- | --- | --- |
| t15-hide-r1 | set | 733 | 422 | 733 | 0 | none |
| t15-nohide-r1 | unset | 0 | 1156 | 0 | 0 | none |
| t15-hide-r2 | set | 732 | 422 | 732 | 0 | none |
| t15-nohide-r2 | unset | 0 | 1158 | 0 | 0 | none |

The number of `9` frames equals the number of primary-plane assignments in
both set sessions. There were no `test failed` lines, and no direct attempt
on any plane other than 35. Per phase, from `t15-hide-r1-analysis.txt`
(r2 matches to within a frame; nohide r1/r2 show `flags1` only, in every
phase, with `direct-tests=0 assigned-35=0`):

```
A-startup              trace     0-1915  log    75-1137  | presented: 112×flags1                  discarded: 0   | scoot: direct-tests=0 assigned-35=0 test-failed=0
B-motion+delay         trace  1915-3358  log  1137-2056  | presented: 42×flags1 60×flags9        discarded: 0   | scoot: direct-tests=61 assigned-35=60 test-failed=0
B-idle                 trace  3358-7581  log  2056-4772  | presented: 302×flags9                  discarded: 0   | scoot: direct-tests=301 assigned-35=302 test-failed=0
C-motion               trace  7581-11086 log  4772-6986  | presented: 245×flags1                  discarded: 0   | scoot: direct-tests=0 assigned-35=0 test-failed=0
C-to-D                 trace 11086-12350 log  6986-7795  | presented: 23×flags1 67×flags9        discarded: 0   | scoot: direct-tests=67 assigned-35=67 test-failed=0
D-idle                 trace 12350-16573 log  7795-10513 | presented: 302×flags9                  discarded: 0   | scoot: direct-tests=302 assigned-35=302 test-failed=0
tail                   trace 16573-16652 log 10513-10541 | presented: 2×flags9                    discarded: 0   | scoot: direct-tests=2 assigned-35=2 test-failed=0
```

(The 61/60 and 301/302 splits are a single frame whose test and assign
lines fall on opposite sides of a boundary. The totals are equal.)

Runs of consecutive flag values, with the event times from the trace
(UTC; this box's libwayland stamps wall-clock time):

| session | runs |
| --- | --- |
| t15-hide-r1 | `1` x154 (20.968-26.074), `9` x362 (26.104-38.138), `1` x268 (38.171-47.089), `9` x371 (47.108-59.443) |
| t15-hide-r2 | `1` x155 (46.717-51.854), `9` x361 (51.888-03.887), `1` x267 (03.921-12.808), `9` x371 (12.841-25.160) |
| t15-nohide-r1 | `1` x1156 (whole session) |
| t15-nohide-r2 | `1` x1158 (whole session) |

- **Hide → `zero_copy`, in the very frame it goes direct.** In r1, the
  last motion was sent at 14:37:25.094 (the `B-last-motion` mark). Scoot's
  first `successfully assigned … to plane::Handle(35)` is at
  `14:37:26.103546Z`, and mpv's first `9` is at 26.103994, 1.010 s after
  the motion. That is the 1000 ms delay plus the frame in flight. The
  handover, as mpv saw it:

  ```
  [14:37:26.074200] {Default Queue} wp_presentation_feedback#50.presented(0, 120166, 559087913, 16666667, 0, 158, 1)
  [14:37:26.103994] {Default Queue} wp_presentation_feedback#50.presented(0, 120166, 588895198, 16666667, 0, 159, 9)
  ```

  The second phase (D) behaves the same way. r1's last C motion is about
  14:37:46.09, and its first `9` is 47.108. In r2 the gaps are 1.018 s
  (50.870 to 51.888) and about 1.0 s.
- **Shown → no `zero_copy`, within a frame or two of the motion.** In r1,
  C's first move went out right after 14:37:38.149 and the last `9` was
  38.138. From 38.171 on, every frame is `1`, and stays so across all 32
  moves:

  ```
  [14:37:38.137704] {Default Queue} wp_presentation_feedback#50.presented(0, 120178, 622609205, 16666667, 0, 520, 9)
  [14:37:38.171353] {Default Queue} wp_presentation_feedback#50.presented(0, 120178, 656216861, 16666667, 0, 521, 1)
  ```

  In C, scoot logs no direct attempt at all (`direct-tests=0`). That is
  Test 5's trap, now caused by scoot's own shown pointer.
- **Debugfs agrees.** Plane 35 in the B and D snapshots of both set
  sessions shows `format=XR30`, `size=2561x1601`, `modifier=0x0`,
  `crtc-pos=2561x1601+0+0`: mpv's buffer. The C snapshot shows `AR24
  2560x1600`, scoot's swapchain. All six snapshots from the unset
  sessions show `AR24 2560x1600`.
- **Phase A (no motion yet) composited in all four sessions: 112-115 `1`,
  no direct attempt.** mpv had no pointer focus then. Its first
  `wl_pointer.enter` comes with the first `scootctl pointer move`. Where
  the pointer sat before that, and whether it was drawn, was not recorded.
  This fits the hide arming only once the pointer is over the covering
  window, but it proves nothing either way. It does not count toward the
  result.

**The hide was scoot's, not mpv's.** None of the four traces contains a
`set_cursor(`. mpv 0.41 binds `wp_cursor_shape_manager_v1` and sets its
pointer once, on enter, and never clears it:

```
[14:37:24.689057] {Default Queue} wl_pointer#8.enter(14, wl_surface#5, 400.00000000, 300.00000000)
[14:37:24.689069] {Default Queue}  -> wp_cursor_shape_device_v1#45.set_shape(14, 1)
```

Each of the four traces has exactly one `set_shape(`, one
`wl_pointer.enter`, no `leave`, and 33 `wl_pointer.motion` (1 in B, 32 in
C). Nothing else touches the cursor until the `destroy` at exit. So the client
asked for a visible arrow the whole time, and every `9` above comes from
the compositor-side hide.

**The buffer qualified in every session.** mpv's first buffer was the
tiled-size `836x1043`, `APPLE_GPU_TILED_COMPRESSED`
(`add(…, 3344, 201326592, 2)`). In three sessions (both hide runs and
nohide-r1), one fullscreen `2561x1601` compressed buffer was also created
before `tranche_flags(1)` arrived. In nohide-r2 the tranche arrived first.
From `tranche_flags(1)` on, every buffer in every session was `LINEAR`
`XR30` (`808669784`) at 2561x1601. From r1:

```
[14:37:20.967706]   -> zwp_linux_buffer_params_v1#54.add(fd 20, 0, 0, 10244, 201326592, 2)
[14:37:20.967711]   -> zwp_linux_buffer_params_v1#54.create_immed(new id wl_buffer#51, 2561, 1601, 808669784, 0)
[14:37:20.968248]  zwp_linux_dmabuf_feedback_v1#47.tranche_flags(1)
[14:37:20.986838]   -> zwp_linux_buffer_params_v1#55.add(fd 21, 0, 0, 10368, 0, 0)
[14:37:20.986845]   -> zwp_linux_buffer_params_v1#55.create_immed(new id wl_buffer#56, 2561, 1601, 808669784, 0)
```

That is Test 6's steering, unchanged. The buffer count did differ by option.
Each set session created five `LINEAR` 2561x1601 buffers, and each unset
session three. The two extra in each set session came about 50-65 ms into
each direct phase (r1: 14:37:26.154 and 14:37:47.172). Presumably mpv's EGL
swapchain grows while the display holds a buffer for scanout. The extra
buffers did not affect the flags, and no session sent a `discarded`.

**What this does not show.** Only one client was tested, mpv, at one
delay (1000 ms), at scale 1.5, on eDP-1. Also not tested: button and
scroll reshows (only motion was driven) and a photo of the panel. The
`zero_copy` flag is scoot's own report of Smithay's plane assignment
(`presentation_time.rs`, "`zero_copy`"), and debugfs is the kernel's view
of the same commit. Neither reads photons. What the test does establish
is that the two agree frame for frame, and that the client receives
exactly that answer.

Raw files are on the box under `~/fx/t15-*`: the scripts, and per session
`TAG.txt` (marks, windows JSON, debugfs extract), `TAG.log` (scoot trace,
ANSI stripped), `TAG-mpv.trace`, `TAG-mpv.log`, `TAG-kms-{B,C,D}.txt`,
`TAG-analysis.txt`, and `t15-state.log`.

### State left behind

`t15-state.sh` after the last session (the last recorded entry, 10:40:09 local):

```
state 10:40:09: fgconsole=1 seatd.sock=gone
no scoot/seatd/mpv/niri/ydotoold processes
card2-DP-1 connected enabled modes=21
card2-eDP-1 connected enabled modes=1
```

The same four lines were recorded after each of the four sessions.
`loginctl` then listed only the tty1 seat session, the user manager,
session 26 (it predates this test) and the ssh session reading the
state. The `runuser` session each run opens was gone. Brightness was 107.
DP-1 stayed on and was never forced. There was no `nh os switch`, no
reboot, and no config change on the box. The only new files are the
`~/fx/t15-*` scripts and outputs.

## Test 16 — per-output scale and mode (`[[outputs]]`) on eDP-1 + DP-1

Run 2026-09-29, 16:06–16:20 UTC, on the same Apple M2 (Mac14,2 / j413),
NixOS 26.11.20260914.efe6f07, kernel 7.1.13 (fairydust), DP-1 (1920x1080
preferred, 21 modes) and eDP-1 (2560x1600, 1 mode) both connected. This
is the hardware gate `docs/backlog/core/per-output-scale-mode.md` was
blocked on (now `docs/backlog/resolved/per-output-scale-mode-done.md`).

**Build.** Branch `per-output-scale-mode` at `71c4a957a`, the last
commit that touches code. Every later commit touches only `Asahi.md` and
`docs/`. It was built on the machine from an rsync of that tree with
`cargo build --release -p scoot -p scootctl --features gpu-scanout`, into
`~/scoot-pos-hw-target`:

```
a0d9edba44e4ec0464b709d93aa3aa14cb82b4f74fa50cc676a3e6081e9fa2c6  release/scoot
f8a3e36a6a94f46a5b7c3158c96181fc674c788f55bc8924f93faed783aeff69  release/scootctl
```

The quoted output below is from sessions `entries-r2` and `control-r2`
at that SHA. Both sessions were first run at `e4a2b668f` (scoot
`b6d2e305…`, kept as `~/fx/t16/*-e4a2b668f`), and every line read the
same. The only code between the two is `71c4a957a`, which tells a
tablet tool's cursor surface a scale; no tablet was attached here.

**Recipe.** `~/fx/t16.sh TAG CONFIG [full|control]` runs one session:
`~/fx/vt-run.sh` on VT 2 with a private seatd, `scoot --tty --renderer
gles` (the GPU tier), the Mesa env from `~/fx/mesa-env.sh`, and
`RUST_LOG=info,scoot=debug`. The config is copied to the session's own
directory, so reloads edit the copy and never `~/.config/scoot`. Clients
are `foot -e sleep 900` under `WAYLAND_DEBUG=1`, started from ssh into
the session. `~/fx/t15-state.sh` ran before and after each session.
Raw output is in `~/fx/t16/{entries-r2,control-r2}.txt`; logs, traces,
`outputs.json`, `wlr-randr.txt`, `wayland-info.txt` and every screenshot
are in `~/fx/t16/<tag>/`.

The config for `entries-r2`:

```toml
[output]
scale = 1.5

[[outputs]]
name = "eDP-1"
scale = 2.0

[[outputs]]
name = "DP-1"
scale = 1.0
mode = "1280x720"
```

### Leg 1 — each output at its own scale and mode

```
drm: driving this device path=/dev/dri/card2 connector=eDP-1 crtc=crtc::Handle(50) width=2560 height=1600 scanout="gpu"
drm: driving this device path=/dev/dri/card2 connector=DP-1 crtc=crtc::Handle(68) width=1280 height=720 scanout="gpu"
```

With no `--mode`, the entry picked DP-1's 1280x720. `scootctl outputs`
reported eDP-1 as `x 0, 1280x800, scale 2.0` and DP-1 as `x 1280,
1280x720, scale 1.0`. `wlr-randr` agreed: eDP-1 2560x1600 at position
0,0 with `Scale: 2.000000`, and DP-1 1280x720 at 1280,0 with `Scale:
1.000000`. So did `wayland-info`: `wl_output` `scale: 2` for eDP-1 and
`scale: 1` for DP-1, and `xdg_output` logical 1280x800 at 0,0 and
1280x720 at 1280,0. **Pass.**

### Leg 2 — a client is told each output's scale, and re-told when it moves

With the pointer at the primary's centre, the first foot opened on
eDP-1 and was told:

```
wl_surface#3.preferred_buffer_scale(2)
wp_fractional_scale_v1#33.preferred_scale(240)
```

Then `scootctl pointer move 1920 360` put the pointer on DP-1, and a
second foot opened there (`"output": 2`). It was told
`preferred_scale(120)` only, since 1 is the integer default and nothing
is sent for it. Then `focus-window-id 1` + `move-window-to-output 2`,
and back again:

```
[16:20:03.418547] wp_fractional_scale_v1#33.preferred_scale(120)   # carried to DP-1
[16:20:03.420371] wl_surface#3.preferred_buffer_scale(1)
[16:20:05.443465] wp_fractional_scale_v1#33.preferred_scale(240)   # and back to eDP-1
[16:20:05.445045] wl_surface#3.preferred_buffer_scale(2)
```

Screenshots, read by eye: `a-out1.png` (2560x1600) shows foot 1 at 2x on
eDP-1: a half-width column of 1280 physical pixels and a 2x text cursor.
`b-out2.png` (1280x720) shows both foots on DP-1 at 1x, each with the
same 1x text cursor, so the carried window re-rendered at its new
scale. **Pass.**

### Leg 3 — a VT switch keeps the entry's mode

`sudo chvt 1; sleep 2; sudo chvt 2`:

```
session paused; drm master released
session activated
drm: hotplug changed nothing this head is driving connector=eDP-1
drm: hotplug changed nothing this head is driving connector=DP-1
Setting new mode: "2560x1600"
Setting new mode: "1280x720"
```

The re-probe on reactivation planned `Keep` for both heads. Nothing
re-modeset DP-1 back to its preferred 1920x1080. The two `Setting new
mode` lines are the usual full modeset on reactivation, each at the mode
the head already had. The debugfs `dri/2/state` read afterwards shows
`crtc[68] mode: "1280x720"` and `crtc[50] mode: "2560x1600"`, and
`outputs` is unchanged. **Pass.** This is the `Tty::modes` path: the
`--tty` re-probe resolves each connector's requested mode by name.

### Leg 4 — reload: one output rescaled live, a mode change refused

The copy's DP-1 `scale` was edited from 1.0 to 1.5, then `scootctl
reload`:

```
{"type":"reloaded","applied":["outputs.DP-1.scale"],"refused":[]}
```

`outputs` now showed DP-1 at `x 1280, 854x480, scale 1.5`, and eDP-1
unchanged at 1280x800, scale 2.0. Foot 2, on DP-1, was re-told
`preferred_scale(180)` and `preferred_buffer_scale(2)`. Its usable width
read 853, one short of the 854 `rect`. That happens on `main` too, with
a single default-scale reload, so it is filed as
[`usable-area-shrinks-after-rescale`](docs/backlog/core/usable-area-shrinks-after-rescale.md)
and not counted here. Then DP-1's `mode` was edited to 1920x1080:

```
{"type":"reloaded","applied":[],"refused":["outputs.DP-1.mode (takes effect on restart: a reload does not modeset a running output; kept the mode the session started with)"]}
```

DP-1 stayed at 1280x720. **Pass.**

### Leg 5 — `wlr-output-management` stays read-only

`wlr-randr --output DP-1 --scale 2` printed `failed to apply
configuration`, rc=1, and `outputs` was unchanged. **Pass.**

### Leg 6 — the lock covers both screens, each at its own scale

`swaylock -c 7a1fa0`:

```
locking the session
session lock confirmed: every output's blanked frame reached scanout
```

`lock-out1.png` (2560x1600) and `lock-out2.png` (1280x720) are both
solid swaylock purple, read by eye. `swaylock.log` has no error and no
`dimensions_mismatch`, so each surface's configured size (eDP-1 1280x800
logical at 2, DP-1 854x480 at 1.5 by then) matched the buffer swaylock
drew. **Pass.**

### Control — no `[[outputs]]` entries

The same recipe with `~/fx/test.toml` (`[output] scale = 1.5` only),
session `control-r2`. This is Test 11/12's layout exactly: eDP-1 at
`1707x1067, scale 1.5` and DP-1 at `x 1707, 1280x720, scale 1.5` from
its preferred 1920x1080. Both `wl_output` `scale: 2`. Both foots were
told `preferred_scale(180)` / `preferred_buffer_scale(2)`. Carrying one
across and back sent **nothing** more, which is the gated per-window
refresh never running when every output agrees. **Pass.**

### State left behind

`t15-state.sh` after both sessions: `fgconsole=1`, `/run/seatd.sock`
gone, no scoot/seatd/mpv/niri/ydotoold, DP-1 `connected enabled
modes=21`, eDP-1 `connected enabled modes=1`. `ERROR` lines: 0 in both
logs. DP-1 stayed on the whole time. No `force` override was written,
and there was no reboot and no `nh os switch`. Brightness read 107
before and was not touched. `swaylock` was realised back into the store
with `nix-store -r` of the path `lock-live.sh` names; no system change.
New files are `~/fx/t16*`, `~/scoot-pos*` and the `~/scoot-pos*-target`
build directories.

**Not run here:** a hotplug add picking up an entry. That needs a
replug, and the force rig is off-limits for this run. The headless suite
pins it (`a_window_moved_onto_a_hotplugged_output_hears_its_scale`,
`a_reload_of_an_absent_outputs_entry_stores_it_and_moves_nothing`), and
`create_output` is the one path for startup, `--outputs N` and hotplug
alike. The pixman (dumb) tier was also not run: the GPU tier only.

## Test 17 — the drawn cursor on an overlay plane, pointer visible

Run 2026-10-06, 15:17-16:16 local (19:17-20:16 UTC), on the same Apple M2
(Mac14,2 / j413), NixOS 26.11, kernel 7.1.13 (fairydust), Mesa 26.2.2, mpv
0.41.0, DP-1 (1920x1080) and eDP-1 (2560x1600) both connected, config
`[output] scale = 1.5` and nothing else (`cursor_hide_after_ms` unset, so
the pointer never hides). The question is
[the ticket](docs/backlog/resolved/gpu-direct-blocked-by-composited-cursor-done.md)'s
option 2: with scoot's drawn cursor in a `LINEAR` dma-buf
(`render/cursor_plane.rs`, on the fork's `UnderlyingStorage::Dmabuf`), does
`apple,dcp` take it on an overlay plane, and does a fullscreen window under
a visible pointer then go direct?

**Builds.** Both on the machine, `cargo build --release --locked -p scoot
--features gpu-scanout` in the flake dev shell, each from a `git archive` of
the commit into its own directory and `CARGO_TARGET_DIR`:

| build | commit | `release/scoot` sha256 |
| --- | --- | --- |
| before | `main` at `29579f343` | `40a4b477f8d2e4c19c87dc6f814d35589fbf399fc429647b838d0c82ea0ab180` |
| after, first cut | `a72918aca` (no lit gate) | `a0b694841ff870cdef0fe1817b538b99ebabebb1416519a0cef08743027c7d0e` |
| after | `267f43749` | `601090162e67ee30bf3d59f981426a332dd95e9d65e7b0fd470197589b03fc4d` |

Every number below the first-cut row is from `267f43749` unless it says
otherwise. Later commits on the branch are docs only.

**Method.** VT 2 with a private seatd and a private `XDG_RUNTIME_DIR`,
`scoot --tty --renderer gles`, `mpv --fs --cursor-autohide=no --vo=gpu
--gpu-context=wayland --hwdec=no --loop` on a 1080p30 H.264 `testsrc2` clip
(mpv never hides its own pointer, as in Tests 14 and 15). One session:
`scoot msg pointer move 400 300`, then `420 320`, then 3 s settle,
debugfs `dri/2/state` (planes 35, 40 and 45), two 10 s samples of the
compositor's `utime + stime` from `/proc/PID/stat` (info logging), 10 s of
pointer motion (a `scoot msg pointer move` every 30 ms, about 26 a second
once the CLI's own start-up is counted), debugfs mid-motion, 3 s settle,
debugfs, one more 10 s sample, a screenshot with and without the pointer.
Sessions alternate before and after.

**Plane state.** Before, in every snapshot: plane 35 on scoot's `AR24
2560x1600` swapchain, both overlays empty. After, in every snapshot, idle
and mid-motion: plane 35 on mpv's `XR30 2561x1601` buffer
(`crtc-pos=2561x1601+0+0`) and plane 45 (the overlay with zpos 2) on a
`AR24 32x32` buffer at `crtc-pos=48x48+626+479`, the 32 px padded cursor
at scale 1.5, moving with the pointer. Plane 40 stayed free.

**CPU** (jiffies per 10 s; `r1`/`r2` alternate):

| run | build | idle 1 | idle 2 | under motion | idle again |
| --- | --- | --- | --- | --- | --- |
| r1 | before | 22 | 28 | 38 | 32 |
| r1 | after | 15 | 14 | 24 | 14 |
| r2 | before | 30 | 30 | 34 | 33 |
| r2 | after | 14 | 13 | 25 | 14 |

The same shape held in the earlier rounds of the day (before 29-35 idle,
37-40 under motion; `a72918aca` 13-15 and 23-25). With the pointer visible
and still over a fullscreen video, compositor CPU is about 55% lower, the
same as Test 14's hidden pointer (13-14), and about 30% lower under
motion, where the old build composited every frame.

A second session shape has no fullscreen window: one tiled `foot`, the
pointer moved over the desktop for 10 s the same way. Before: 17 and 19
jiffies (earlier rounds 19, 20, 20, 20). After: 11 and 11 (earlier 9, 10,
12, 12), with the cursor on plane 45 and the primary on the swapchain. Idle
was 0 both ways. Moving the pointer no longer redraws anything.

**Trace and `zero_copy`.** One session at
`RUST_LOG=info,scoot=debug,smithay::backend::drm::compositor=trace`, with
`WAYLAND_DEBUG=1` on mpv:

```
testing direct scan-out ... plane::Handle(35)   1499
testing direct scan-out ... plane::Handle(45)   1506
successfully assigned ... plane::Handle(35)     1499
successfully assigned ... plane::Handle(45)     1506
test already known to fail                      0
test failed                                     2   (startup, frame level, both CRTCs)
mpv presented flags: 9 (vsync | zero_copy) x1493, 1 (vsync) x1
```

The cursor's first `testing direct scan-out` exports a framebuffer from
the dma-buf (`underlying storage Dmabuf(...)`), and every later frame uses
the cached one. The one `1` frame is mpv's first.

**What the lit gate fixed.** The first cut (`a72918aca`) offered the plane
from the first frame. That frame carries the modeset, and DCP fails the
overlay test against it. Smithay then keeps the failure for as long as the
element and its position stay the same ("test already known to fail", 444
lines in one session). So a pointer nobody had moved yet stayed
composited, and the video under it with it: in the first cut's trace run
mpv got 108 `1` frames before the first pointer move. `vt.sh`, pointer
never moved, debugfs 5 s after mpv starts, then after `chvt 1`/`chvt 2`,
then after one move:

| build | at startup | after the VT switch back | after one move |
| --- | --- | --- | --- |
| `a72918aca` | `AR24` swapchain, overlays empty | `AR24` swapchain, overlays empty | mpv's `XR30`, cursor on 45 |
| `267f43749` | mpv's `XR30`, cursor on 45 at `+1276+799` (the centre) | the same | the same, cursor moved |

**Edges.** The pointer parked at eight points, 1.5 s each, debugfs at each:

| pointer (logical) | cursor plane `crtc-pos` | plane 35 |
| --- | --- | --- |
| 800,500 | `48x48+1196+749` | mpv direct |
| 1680,500 | `48x48+2516+749` (44 px on screen) | mpv direct |
| 1695,500 | none: 22 px would be on screen, not offered | composited |
| 1706,1066 | none (corner) | composited |
| 0,0 | `48x48-5-2` (the hotspot's offset) | mpv direct |
| 1,1066 | none: 2 px tall on screen | composited |
| 800,0 | `48x48+1196-2` | mpv direct |
| 800,500 | `48x48+1196+749` | mpv direct |

No `test failed` past the 2 at startup. The kernel clips a plane that runs
off the screen and refuses one smaller than 32x32 after clipping
(`apple_plane_atomic_check`), and scoot now does not offer one that would
end up that small. An earlier, uncommitted build without that check
(binary sha256 `2be04ed2…`) offered them, got the refusal, and the kernel logged its
once-per-boot `Plane operation would have crashed DCP! Rejected!` line
once. That line refers to the rejected test, not to a crash. Nothing froze,
and the frame composited as it should.

**Captures.** A `scoot msg screenshot` taken while the cursor rode plane
45 shows the arrow at the plane's position, and `--no-cursor` shows the
video there. The capture re-renders the cursor's region
(`render::capture_cursor`, which records an overlay cursor's footprint).

**What this does not show.** One client (mpv), one scale (1.5), eDP-1
only. No photo of the panel: debugfs and `zero_copy` are the evidence that
the planes hold what is claimed, not that the panel shows it right. The
right and bottom edges of the shape are filtered against the padding when
scaled (headless test `scaled_the_twin_differs_only_along_the_images_right_and_bottom_edge`).
Not looked at closely on the panel. Not tested: a client's own cursor
image as a dma-buf (already able to ride an overlay, unchanged), DPMS
off and on with the pointer still, DP-1, and hotplug. The cursor-element
cost per frame is measured off hardware: 250 ns to build the drawn element
and 302 ns to build it and swap it (pixman renderer, release, 200,000
frames, `bench_the_swap_per_frame`).

### State left behind

Checked at 16:33 local, after the last session and the test-suite runs:
`fgconsole` 1, `/run/seatd.sock` gone, no `scoot`, `seatd`, `mpv` or `foot`
process, greetd (pid 1396068) and its greeter session on tty1 (session
60542) as found, DP-1 and eDP-1 `connected`. greetd was never stopped; the
VT switches in this test were `chvt 1`/`chvt 2` onto the greeter's VT and
back. No `nh os switch`, no reboot, no config change. The test directory
`~/fx/cursor-k7q2/` (trees, target dirs, scripts, logs) and
`/tmp/k7q2-cargo.toml` were deleted. The scripts, the per-session `.txt`
and debugfs files, and the xz-compressed trace and mpv logs were copied off
the box first, into the implementing session's scratchpad
(`cursor-m2-evidence.tar.xz`). Those copies are not in the repository.

## Keys for the 2026-09-25 runs

Built on the machine itself (native aarch64) from `main` at
`e1dce6ffff2c77cbbf50a362f3b65bdd6d8c8882`, with `nix build .#scoot
.#scoot-gpu .#scootctl`:

| binary | store path | sha256 |
| --- | --- | --- |
| `scoot` | `/nix/store/5fsph0ax28df3ncd4w0jk03m88936yvg-scoot-0.1.0` | `0f363f74769bdb5f82c4a420fa8769a1ad9a070dddabd9ed176db7190e8c5b72` |
| `scoot` (`scoot-gpu`) | `/nix/store/zd2226z0hjk1j5w8kwmw7icd1whkjqzp-scoot-gpu-0.1.0` | `acde9262db3bbe1a823d833c1c4b24afc188833d9267267bd624339ad2471326` |
| `scootctl` | `/nix/store/bgq24fqxs1yrwj9kdwar6jfnbc4jn0gm-scootctl-0.1.0` | `3e0e22e00faa5784656d9fb1f095d401cc48f9882c1d4f25f2d0604bac4dab76` |

Machine: Apple M2 (Mac14,2 / j413), NixOS 26.11 (nixpkgs `efe6f07`),
kernel 7.1.5, Mesa 26.2.2 (`asahi` GL 4.6, honeykrisp Vulkan 1.4.354),
niri 26.04, 8 CPUs, 7.3 GiB. The test tools (niri, cage, seatd, ydotool,
drm_info, mpv, vulkan-tools, mesa-demos, …) were added through a
packages-only NixOS module. Sessions ran on VT 2 against a private `seatd`
(`sudo seatd -u "$USER"`, then `sudo openvt -c 2 -s -- runuser -u "$USER" --
env LIBSEAT_BACKEND=seatd …`), because the machine had no greeter and no
logind seat session that an SSH-started process could join. Every
`--tty` run used a config holding only `[output] scale = 1.5`. The raw logs,
traces and debugfs dumps stay on the machine under `~/fx`.

## What to send back

- `ghostty --version`
- `/tmp/fx/ghostty-1.5.log`, `/tmp/fx/ghostty-2.0.log`
- `/tmp/fx/g15.png`, `/tmp/fx/g20.png`
- `/tmp/fx/tty-auto.log`, and `/tmp/fx/tty-gpu.log` if you needed it
- the `/dev/dri` and driver listings from Test 2
- for Test 3: the log, plus which connector it started on and which you pulled
- for Test 5: `/tmp/fx/t5.clean.log`, both `t5-kms-*.txt`, `t5-fs.png`,
  and the grep output
- for Test 7: `/tmp/fx/t7.log`, `/tmp/fx/t7-global.txt`, the two
  `t7-vkcube*.png`, `t7-kms-fs.txt`, and the grep output (or the whole
  `t7-vkcube.trace` if something went wrong)
- for Test 8: both `/tmp/fx/t8-*.txt`, and the `t8-*-inner.log`,
  `t8-*-host.log` and `t8-*.png` files beside them; for Part B,
  `/tmp/fx/t8b.txt`, both `t8b-*.log` and `t8b-*.png`, your desktop's own
  screenshot, and which desktop it was
- for Test 9: the whole `/tmp/fx/t9a`, `/tmp/fx/t9a-diag` and
  `/tmp/fx/t9a-gpu` directories (raw TSVs and every `inner.log`),
  `/tmp/fx/t9a.md`, and `/tmp/fx/t9b.tsv` with a note of which session each
  block of lines came from
- for Test 6: `/tmp/fx/t6-table-*.txt`, `/tmp/fx/t6-*.log`,
  `/tmp/fx/t6-gears.trace` (or just its `add(` lines), `t6-gears.png`,
  `t6-mpv.log`, `t6-mpv.png`, `t6-kms-mpv.txt`; for Part C `t6c.log`,
  both `t6c-*.trace` (or their `tranche_*`, `add(` and `presented(` lines),
  `t6c-kms-*.txt`, `t6c-fs.png`, and the grep output
- for Test 10: `/tmp/fx/t10.log`, `/tmp/fx/t10-dmabuf-fds.txt`, `/tmp/fx/t10-planes.txt` and `/tmp/fx/t10-vkcube.trace`

Raw logs beat a summary here. Both open entries were written after earlier
investigations went wrong in ways only the raw output showed — a harness
artifact that looked exactly like the bug in one case, and a test whose own
trigger refreshed the kernel state it was meant to be reading in another.
The 2026-09-18 run added a third of the same kind: the default IPC socket
silently addressing the live session instead of the test instance (Test 1's
first trap). Assume the next one exists too, and keep the raw output.

## Verification set on this hardware (2026-09-18)

First time the suite had ever run on aarch64/Asahi. All green against `main`
at `f688ac9` — where the package was still called `flexwm`, so these are the
commands as they were actually run, not as they would be typed today
(`-p flexwm` is `-p scoot` on current `main`): `cargo test -p flexwm` 964
passed; `cargo nextest run --workspace` 1069 passed, 2 skipped;
`cargo clippy -p flexwm --all-targets -- -D warnings` clean;
`cargo fmt --check -p flexwm` clean; `scripts/smoke-test.sh` 16/16 `ok`.

One snag worth knowing: `smoke-test.sh` needs `jq`, which the flake's dev
shell does not provide. Without it the window-count check compares an empty
string and the script fails with `the second foot never mapped a window` —
a harness artifact, not a compositor bug. Run it under
`nix shell nixpkgs#jq --command …` until the dev shell carries `jq`.
