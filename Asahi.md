# Running flexwm on Apple Silicon (Asahi Linux)

Three things in the backlog were blocked on an Asahi machine and cannot be
answered anywhere else. This is the runbook for answering them. Written
against `main` at `1b706b8`; the flags and bindings below are quoted from
`README.md` at that commit. **Two of the three were answered on 2026-09-18**
— see the results table and section immediately below; the runbook itself is
kept intact for re-runs on other Apple Silicon models.

| What it unblocks | Priority | Needs a VT? | Status |
| --- | --- | --- | --- |
| [Ghostty fails at `scale = 1.5`](docs/backlog/resolved/ghostty-fails-at-1-5-done.md) | high → none | no | **RESOLVED, not reproducible** (2026-09-18) |
| [Does `--gpu` actually fix this machine](docs/backlog/resolved/tty-gpu-config-key-done.md) — the residual on a resolved entry | — | yes | **closed**: not needed, the search works (2026-09-18) |
| Issue #48's unconfirmed connector fallback | — | yes | still open — needs an external display |

## Results so far (run 2026-09-18, `main` at `f688ac9`)

The machine is an Apple M2 (`apple,t8112`, j413), NixOS aarch64. The DRM
split is exactly as this document predicted: `card1` → `asahi` (render, no
KMS), `card2` → `apple-drm` (display, owns `eDP-1`), `renderD128` → render
node.

- **Test 2 — answered, no VT needed after all.** flexwm was *already the
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
- **Test 3 — cannot run.** Only one connector exists (`card2-eDP-1`,
  connected). Nothing to fall back to until an external display is attached,
  and it also needs the `--tty` seat the live session holds.

Everything below is the runbook as written, plus traps found while running
it. Re-running any of it is still worthwhile on a different Asahi model or
a newer Ghostty.

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
  under suspicion, so the backend costs nothing here. *(That reasoning held
  and did its job — the GL stack was exercised over dmabuf and the GL
  hypothesis was refuted. But refuting it is exactly what makes the backend
  matter again: with GL eliminated, `--headless`-vs-`--tty` is now the
  largest untested variable. Test 1's result note says what is left.)*
- **Test 2 may need no VT either** — if flexwm is already your desktop
  session, the read-only shortcut in that section answers it from inside the
  session you are in. Check before booking a VT.
- **Tests 2 and 3 take a VT**, absent that shortcut. Under `--tty`, flexwm binds `Ctrl+Alt+F1`
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
FLEXWM_SOCKET=/tmp/fx/t15.sock WAYLAND_DEBUG=1 \
  ./result/bin/flexwm --headless --width 1280 --height 800 \
  --config /tmp/fx/s15.toml \
  -- ghostty --gtk-single-instance=false -e sh -c "sleep 60" \
  > /tmp/fx/ghostty-1.5.log 2>&1 &

sleep 12
FLEXWM_SOCKET=/tmp/fx/t15.sock ./result/bin/flexwm msg windows
FLEXWM_SOCKET=/tmp/fx/t15.sock ./result/bin/flexwm msg screenshot --out /tmp/fx/g15.png
```

Then the same with `--config /tmp/fx/s20.toml` and a *different* socket
(`/tmp/fx/t20.sock`), writing `/tmp/fx/ghostty-2.0.log` and `/tmp/fx/g20.png`.

**Three traps. The first two produced false failures during the original
investigation; the third was found running this document on 2026-09-18. All
are recorded so they are not repeated:**

- **Set `FLEXWM_SOCKET` per instance if flexwm might already be running.**
  `socket_path()` defaults to `$XDG_RUNTIME_DIR/flexwm.sock`, so on a machine
  where flexwm *is the desktop session* — which is how this one is set up —
  a bare `flexwm msg windows` talks to **the live session, not the test
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
the defect is not flexwm's scaling and the fix (if any) is Ghostty-side or a
workaround (`scale = 2.0`, or Ghostty's own `window-scale`/font-size
setting). A Wayland protocol error means the opposite, and is a flexwm bug.

**Don't reach for `--nested` as a middle ground.** It looks like a way to
test fractional scale against a real compositing backend without taking the
VT, and it is not — flexwm refuses output scaling there by design, because
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
around are therefore refuted, and the remaining suspect is `--tty` at 1.5.
See the backlog entry for the full table and what is left to test.

---

## Test 2 — which DRM device `--tty` drives

**The question.** Does the automatic search find the display controller on
Apple Silicon, and if not, does `--gpu` fix it?

> **Answered 2026-09-18: yes, the search works; `--gpu` is not needed.**
> And it needed no VT, because flexwm was already the live session. **Check
> that first** — if flexwm is already running as your desktop, the cheapest
> and most authoritative version of this test is read-only, on the session
> you are already in:
>
> ```sh
> pgrep -ax flexwm                              # was --gpu passed?
> ls -l /proc/$(pgrep -x flexwm)/fd | grep dri  # which DRM node is open?
> flexwm msg outputs                            # which connector?
> ```
>
> On this machine that gave `--tty` with no `--gpu`, one DRM fd
> (`/dev/dri/card2`, the `apple-drm` display controller), and `eDP-1` —
> i.e. the fallback rejected the `asahi` render node unattended, in a real
> daily-driven session. That is better evidence than a test run, and it
> costs nothing. The steps below are for the case where flexwm is *not*
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
> before blaming flexwm.
>
> Second constraint: it needs the `--tty` seat, which only one process can
> hold. If flexwm is already your desktop session, that session *is* the
> one to test — don't start a second `--tty` instance, it will be refused.

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
The 2026-09-18 run added a third of the same kind: the default IPC socket
silently addressing the live session instead of the test instance (Test 1's
first trap). Assume the next one exists too, and keep the raw output.

## Verification set on this hardware (2026-09-18)

First time the suite had ever run on aarch64/Asahi. All green against `main`
at `f688ac9`: `cargo test -p flexwm` 964 passed; `cargo nextest run
--workspace` 1069 passed, 2 skipped; `cargo clippy -p flexwm --all-targets
-- -D warnings` clean; `cargo fmt --check -p flexwm` clean;
`scripts/smoke-test.sh` 16/16 `ok`.

One snag worth knowing: `smoke-test.sh` needs `jq`, which the flake's dev
shell does not provide. Without it the window-count check compares an empty
string and the script fails with `the second foot never mapped a window` —
a harness artifact, not a compositor bug. Run it under
`nix shell nixpkgs#jq --command …` until the dev shell carries `jq`.
