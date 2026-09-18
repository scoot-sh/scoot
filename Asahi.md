# Running flexwm on Apple Silicon (Asahi Linux)

Three things in the backlog are blocked on an Asahi machine and cannot be
answered anywhere else. This is the runbook for answering them. Written
against `main` at `1b706b8`; the flags and bindings below are quoted from
`README.md` at that commit.

| What it unblocks | Priority | Needs a VT? |
| --- | --- | --- |
| [Ghostty fails at `scale = 1.5`](docs/backlog/protocols/ghostty-fails-at-1-5.md) | high | no |
| [Does `--gpu` actually fix this machine](docs/backlog/resolved/tty-gpu-config-key-done.md) — the residual on a resolved entry | — | yes |
| Issue #48's unconfirmed connector fallback | — | yes |

## Why this machine

Two of the three are Apple-Silicon-specific by construction, not by accident:

- **The DRM split.** The usual "the GPU whose PCI parent has `boot_vga=1`"
  heuristic assumes the 3D GPU and the display controller are one DRM device.
  Under Asahi they are not: `asahi`/AGX owns the render node, `apple,dcp`
  owns the CRTCs and connectors, and there is no PCI GPU or VGA BIOS for the
  rule to match. flexwm has a fallback that tries every DRM device on the
  seat, and `--gpu PATH` to skip the search entirely — **both are built and
  unit-tested, and neither has ever run on Apple Silicon.**
- **The GPU/GL path.** The dev VM falls back to software rendering (Mesa
  `swrast`/`zink` failures throughout its logs). The leading hypothesis for
  the Ghostty failure is a client-side EGL/GL problem at fractional scale,
  which software rendering structurally cannot reproduce.

## Safety

Nothing here can strand you out of your own desktop, and it is ordered so the
risky part comes last:

- **Test 1 needs no VT at all.** It runs `--headless` inside your normal
  session. Ghostty still uses the real Asahi GL stack, which is the part
  under suspicion, so the backend costs nothing here.
- **Tests 2 and 3 take a VT.** Under `--tty`, flexwm binds `Ctrl+Alt+F1`
  through `Ctrl+Alt+F12` to VT switching. These are added *after* the config
  file loads and always win over a colliding config bind (logging a warning
  naming what they displaced), precisely so this recovery path cannot be lost
  to a typo. `Ctrl+Alt+F<your desktop's VT>` gets you back.
- `Super+Shift+e` quits flexwm. (Deliberately not `Super+Shift+q` — one
  slipped Shift from `Super+q`, close window.)

## Build

```sh
git clone https://github.com/yackey-labs/flexwm && cd flexwm
nix build                      # -> ./result/bin/flexwm
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
`wp_fractional_scale_v1` object exists, which flexwm always advertises, so
**the fix probably does not explain the symptom and the root cause is
unknown.** A dev-VM reproduction attempt failed to reproduce at all: Ghostty
1.3.1 mapped and rendered real content at both scales.

So the job here is not to confirm a fix. It is to get the one piece of
evidence that tells us whether this is a Ghostty-version thing or a GPU/GL
thing — a client-side EGL/GL error would mean flexwm's scaling is not the
defect.

```sh
printf '[output]\nscale = 1.5\n' > /tmp/fx/s15.toml
printf '[output]\nscale = 2.0\n' > /tmp/fx/s20.toml

ghostty --version                   # record this
```

Run at 1.5:

```sh
WAYLAND_DEBUG=1 ./result/bin/flexwm --headless --width 1280 --height 800 \
  --config /tmp/fx/s15.toml \
  -- ghostty --gtk-single-instance=false -e sh -c "sleep 60" \
  > /tmp/fx/ghostty-1.5.log 2>&1 &

sleep 10
./result/bin/flexwm msg windows
./result/bin/flexwm msg screenshot --out /tmp/fx/g15.png
```

Then the same with `--config /tmp/fx/s20.toml`, writing
`/tmp/fx/ghostty-2.0.log` and `/tmp/fx/g20.png`.

**Two traps, both of which produced false failures during the original
investigation and are recorded so they are not repeated:**

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
the defect is not flexwm's scaling and the fix (if any) is Ghostty-side or a
workaround (`scale = 2.0`, or Ghostty's own `window-scale`/font-size
setting). A Wayland protocol error means the opposite, and is a flexwm bug.

---

## Test 2 — which DRM device `--tty` drives

**The question.** Does the automatic search find the display controller on
Apple Silicon, and if not, does `--gpu` fix it?

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
cd ~/flexwm
RUST_LOG=info ./result/bin/flexwm --tty -- foot 2>&1 | tee /tmp/fx/tty-auto.log
```

If that fails, name the display controller explicitly (adjust the path from
the listing above):

```sh
RUST_LOG=info ./result/bin/flexwm --tty --gpu /dev/dri/card1 -- foot 2>&1 \
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

**Once you know the right device, stop retyping it.** `~/.config/flexwm/config.toml`:

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
otherwise).

Issue #48 is the last open issue in the repo. PR #51 implemented DRM hotplug
and deliberately said `Refs #48`, not `Closes`: two of its four code paths
have never run on real hardware, because nothing in a QEMU/virtio-gpu VM can
change a connector's mode list at runtime (EDID override, off/detect cycles
and `vkms` were all tried and documented as dead ends). A laptop with an
external display is the only thing that can produce one of them.

With flexwm running under `--tty` and an external display plugged in, unplug
the one flexwm is currently driving. Expected: it falls back to the other
connector and keeps running, rather than black-screening until restart.
Capture the log the same way (`tee`), and note which connector it started on
(`flexwm msg outputs` names it — `eDP-1`, `HDMI-A-1` and so on).

This exercises `Plan::NewConnector`, which is the only path that runs
`set_pending` against real DRM. Plugging a *second* display in without
unplugging the first should keep the session where it is: this is a
one-output backend, and staying on the panel the user is looking at is
deliberate.

---

## What to send back

- `ghostty --version`
- `/tmp/fx/ghostty-1.5.log`, `/tmp/fx/ghostty-2.0.log`
- `/tmp/fx/g15.png`, `/tmp/fx/g20.png`
- `/tmp/fx/tty-auto.log`, and `/tmp/fx/tty-gpu.log` if you needed it
- the `/dev/dri` and driver listings from Test 2
- for Test 3: the log, plus which connector it started on and which you pulled

Raw logs beat a summary here. Both open entries were written after earlier
investigations went wrong in ways only the raw output showed — a harness
artifact that looked exactly like the bug in one case, and a test whose own
trigger refreshed the kernel state it was meant to be reading in another.
