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
whole of it as one command that writes its own report.

| What it unblocks | Priority | Needs a VT? | Status |
| --- | --- | --- | --- |
| [Ghostty fails at `scale = 1.5`](docs/backlog/resolved/ghostty-fails-at-1-5-done.md) | high → none | no | **RESOLVED, not reproducible** (2026-09-18) |
| [Does `--gpu` actually fix this machine](docs/backlog/resolved/tty-gpu-config-key-done.md) — the residual on a resolved entry | — | yes | **closed**: not needed, the search works (2026-09-18) |
| Issue #48's unconfirmed connector fallback | — | yes | still open — needs an external display |
| [Test 4: CPU vs GPU on a real GPU](docs/backlog/resolved/gpu-vs-cpu-measured-done.md) | high → none | yes | **ANSWERED** (2026-09-21): scanout comes up on the split topology and costs 4–5x less CPU |
| [Test 5: a fullscreen video scanned out directly](docs/backlog/resolved/gpu-primary-direct-format-gate-done.md) | medium | yes | open — seen on the dev VM only |
| [Test 6: what the GLES tier advertises, and what GPU clients do with it](docs/backlog/resolved/gles-dmabuf-full-formats-done.md) (Part C: [the scanout tranche](docs/backlog/resolved/gpu-scanout-candidates-done.md)) | high | partly | open — seen on the dev VM's llvmpipe only (57 formats, all `LINEAR`; scanout tranche `XR24`/`AR24` at `LINEAR`) |
| [Test 7: explicit sync on a real GPU](docs/backlog/resolved/linux-drm-syncobj-done.md) | medium | yes | open — seen on the dev VM's virtio-gpu with a test client only |

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

Issue #48 is the last open issue in the repo. PR #51 implemented DRM hotplug
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
1280,800 × 1.5 = physical 1920,1200, the crop holds 118 distinct colours
(an antialiased glyph, not a flat block), and the same crop taken anywhere
else differs by exactly zero. The control matters as much as the result —
the same tier captured in different rounds gives `AE = 0`, byte-identical,
so the scene really is pinned and the 18×34 box is the whole difference.
For scale, a whole-frame one-least-significant-bit difference at this
resolution measures `AE ≈ 4016` if one channel of four differs everywhere,
or `AE ≈ 12044` if all three colour channels do (measured, not derived — add
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
since landed — see the resolved record.)

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
First, the compositor coloured its log output unconditionally, so
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

## Test 8 — `--nested --renderer gles` handing its frames to the host as dma-bufs

Why: in a `gpu-scanout` build, a `--nested --renderer gles` scoot whose
host composites on the same GPU no longer reads each frame back into
`wl_shm`: it copies the frame on the GPU into a buffer shared with the host
(`docs/tty.md`, "Which renderer draws the frames"). On the dev VM that works
end to end -- the host's screenshot is byte-identical to the nested one, and
the protocol trace shows the buffers created and attached -- but its GPU is
llvmpipe, where drawing the frame in software is nearly all of the cost, so
per frame it measured the same as read-back (and a little dearer per resize,
~0.4-1.1 ms of the ~14 ms a resize costs there). Whether dropping the copy
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
  `t8-*-host.log` and `t8-*.png` files beside them
- for Test 6: `/tmp/fx/t6-table-*.txt`, `/tmp/fx/t6-*.log`,
  `/tmp/fx/t6-gears.trace` (or just its `add(` lines), `t6-gears.png`,
  `t6-mpv.log`, `t6-mpv.png`, `t6-kms-mpv.txt`; for Part C `t6c.log`,
  both `t6c-*.trace` (or their `tranche_*`, `add(` and `presented(` lines),
  `t6c-kms-*.txt`, `t6c-fs.png`, and the grep output

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
