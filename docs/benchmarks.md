# Benchmarks

Measured resource usage: the method, the raw numbers, and what each number
can and cannot show. Every row says what it measured. The caveats sit next
to the results because some rows are not like-for-like, and each place where
they aren't is marked.

- [scoot vs niri, nested on the dev VM (2026-09-24)](#scoot-vs-niri-nested-on-the-dev-vm-2026-09-24)
- scoot's own tiers on real hardware (dumb buffers + pixman vs GPU scanout
  on an Apple M2) are in [`Asahi.md`](../Asahi.md) Test 4, summarised in
  [tty.md](tty.md#which-renderer-draws-the-frames).

## scoot vs niri, nested on the dev VM (2026-09-24)

scoot's layout comes from [niri](https://github.com/niri-wm/niri), which
makes niri the obvious reference point. The question here is how much
running each one costs on the same workload, on the one machine where both
could run. That machine is the dev VM, and it can host only half of the
answer. niri renders only through GLES, and it refuses a software renderer
on a real `--tty` session. The VM's GPU has no 3D, so niri could run there
only nested, and every GLES row (niri's and scoot's `--renderer gles`)
rasterises on llvmpipe, a software renderer. The real-GPU half, `--tty`
included, is a runbook step for real hardware ([`Asahi.md`](../Asahi.md)
Test 9). Nothing on this page is a claim about it.

niri is a mature, much fuller compositor: animations, an overview,
screencasting through PipeWire, a hotkey overlay, rich per-window rules and a
lot more that scoot does not have. Some of the footprint below pays for
those features. This page compares costs on a narrow workload, not the value
each compositor gives you.

### Summary

- **Without a GPU,** niri's only option is software GLES, and scoot's
  default pixman renderer is far cheaper than that. Here, scoot spent 1.9–2.5
  ms of CPU per presented frame against niri's 14–16 ms. At rest with three
  terminals it used 49 MB RSS against niri's 185 MB, committed its first frame
  to the host 27 ms after starting against niri's 113 ms, and idled with no wakeups at all
  against niri's ~3 a second (which is already negligible).
- **On the same GL stack (scoot `--renderer gles` against niri, both on
  llvmpipe), niri is the more efficient renderer.** A frame cost 14.2 ms in
  niri against 20.5 ms in scoot during relayout, and 14.8 ms against 18.4 ms
  with a client animating. niri also delivered more frames to that client (54
  a second against scoot-gles's 43 and scoot-pixman's 50). scoot-gles used
  about 20% less memory at rest (149 MB RSS against 185 MB). However, it
  **grows by one frame of memory per screenshot while nothing is redrawing**,
  which is a bug this benchmark found and
  [filed](backlog/core/gles-capture-leaks-a-frame-per-shot.md). niri stayed
  bounded under the same captures.
- **Part of scoot's lower totals comes from doing less work, not doing
  the same work more cheaply.** Nested, scoot draws no pointer of its own
  (the host draws the host's pointer), while niri composites its pointer into
  every frame. At 594 frames of about 16.2 ms, that is nearly all of the
pointer row's gap. During
  a relayout storm, niri also presented about three frames per action where
  scoot presented one.
- **Screenshots:** through each compositor's own IPC, a scoot capture took
  11 ms of wall time and 8 ms of compositor CPU, against niri's 34 ms and 35
  ms. niri's figure includes putting the image on the clipboard, and it
  writes a PNG a third the size. **Through `grim`, niri was faster** (43 ms
  against 56 ms). scoot parks a `grim` capture until its next frame tick by
  design, although it spent less CPU on the capture (4 ms against 31 ms).

### What ran

| | |
|---|---|
| Machine | the dev VM (`vm/`): NixOS aarch64 under QEMU on an Apple-silicon Mac, 4 vCPUs, 3.9 GB RAM, kernel 6.18.50, virtio-gpu with no 3D |
| scoot | `main` at `fe41921`, `cargo build --release -p scoot -p scootctl` under the repo's own release profile (fat LTO, `codegen-units = 1`), built on the VM with `CARGO_BUILD_JOBS=1`, peak RSS 1.66 GB. Binary sha256 `e6e30f71…dbc65`, 5,854,232 bytes. |
| niri | 26.04 from nixpkgs (`/nix/store/ww71z668r7kprqxwncl8xhsyjg6sxgr7-niri-26.04`), built by nixpkgs under its own profile |
| Host | cage 0.3.1 (wlroots), headless backend, **pixman** renderer, run with `-d` (see below), one 1600x1000 output at scale 1 and 60 Hz |
| Clients | foot 1.28.0 with an empty config, grim 1.5.0 |
| GLES | Mesa 26.2.2 llvmpipe (LLVM 21.1.8) for niri and for scoot `--renderer gles` alike, as each one's own log reports |

The four variants alternated in a rotating order for three rounds (ABCD,
BCDA, CDAB), each in a fresh session:

- **scoot-pixman**: `scoot --nested --width 1600 --height 1000`, built-in
  defaults (no config file).
- **scoot-gles**: the same with `--renderer gles`.
- **niri-off**: niri with
  [`scripts/niri-ab/niri-anim-off.kdl`](../scripts/niri-ab/niri-anim-off.kdl),
  written for this benchmark to match scoot's defaults as far as niri
  allows. It sets 12 px gaps, half-width new columns, a 3 px ring in scoot's
  two colours around every window (niri's `border`; its `focus-ring`, which
  rings only the focused window, is off), no shadows, `prefer-no-csd`, and
  `animations { off; }`.
- **niri-on**: the same file with only the `animations` line removed, so
  niri runs its default animations.

Each session: start, three `foot`s, then six measured scenes, each after
waiting for the compositor to go idle (under 2 ms of CPU in each of two
consecutive half-seconds):

| Scene | What happens |
|---|---|
| idle | 20 s with nothing happening |
| pointer | 1200 absolute pointer motions at 120 Hz for 10 s, alternating between two points in two different windows. They are injected into the **host** by one persistent `zwlr_virtual_pointer_v1` device ([`scripts/niri-ab/vptr`](../scripts/niri-ab/vptr)), so both compositors get the same `wl_pointer` events from the same source. |
| relayout | 200 layout actions at 20 a second, cycling focus left, left, right, right, then move column left and right, through each compositor's own IPC client |
| shot-ipc | 10 captures through each compositor's own screenshot path, 200 ms apart, pointer omitted |
| shot-grim | 10 captures by `grim` against the nested session, 200 ms apart |
| animate | a fourth `foot` prints a line about every 16 ms for 10 s |

A second, separate pass of the same script (`DIAG=1`) ran with the host
logging its protocol traffic. It counts how many frames each compositor
presented in each scene and times the first frame. That log slows the host
down, so none of the CPU numbers come from that pass.

### Results

Every cell is the median of three rounds, with the range across rounds in
brackets. "cpu" is the compositor process's on-CPU time, summed over all of
its threads from `/proc/PID/task/*/schedstat` and cross-checked against
`/proc/PID/stat`. "Wakeups" counts how many times any of its threads was put
on a CPU. Neither figure includes the clients, the IPC client processes or
the host. "Frames" are the compositor's own commits to the host, from the
DIAG pass.

#### CPU per scene

| scene | variant | cpu ms | cpu % of a core | wakeups/s | µs per event | frames/s (DIAG) |
|---|---|---|---|---|---|---|
| idle (20 s) | scoot-pixman | 0.0 [0.0–0.0] | 0.0 | 0.0 [0.0–0.0] | – | 0 |
| | scoot-gles | 0.0 [0.0–0.0] | 0.0 | 0.0 [0.0–0.0] | – | 0 |
| | niri-off | 6.9 [6.9–6.9] | 0.0 | 3.1 [3.1–3.1] | – | 0 |
| | niri-on | 6.8 [6.6–6.9] | 0.0 | 3.1 [3.1–3.1] | – | 0 |
| pointer (1200 events) | scoot-pixman | 82.8 [82.2–84.3] | 0.8 | 228 [228–228] | 69 [68–70] | 0 |
| | scoot-gles | 93.8 [91.9–96.8] | 0.9 | 229 [228–229] | 78 [77–81] | 0 |
| | niri-off | 9625 [9577–9628] | 91.5 | 1586 [1584–1594] | 8021 [7980–8023] | 56.5 |
| | niri-on | 9664 [9568–9736] | 91.9 | 1589 [1588–1591] | 8053 [7973–8114] | 56.7 |
| relayout (200 actions) | scoot-pixman | 375 [370–379] | 3.7 | 95 [94–95] | 1875 [1850–1895] | 20.0 |
| | scoot-gles | 4119 [4104–4213] | 41.1 | 522 [521–522] | 20595 [20521–21065] | 20.1 |
| | niri-off | 8454 [8407–8496] | 84.2 | 1633 [1615–1640] | 42484 [42248–42694] | 59.3 |
| | niri-on | 8520 [8380–8562] | 85.0 | 1949 [1947–1955] | 43025 [42322–43032] | 59.3 |
| shot-ipc (10) | scoot-pixman | 83.3 [82.1–86.5] | 3.7 | 22 [22–23] | 8332 [8207–8645] | 0 |
| | scoot-gles | 59.8 [58.3–59.9] | 2.7 | 18 [18–18] | 5977 [5835–5987] | 0 |
| | niri-off | 352 [342–360] | 14.3 | 133 [130–136] | 35200 [34175–35961] | 0 |
| | niri-on | 349 [347–361] | 14.2 | 133 [128–134] | 34921 [34695–36122] | 0 |
| shot-grim (10) | scoot-pixman | 41.5 [40.5–41.9] | 1.6 | 26 [26–26] | 4146 [4050–4193] | 0 |
| | scoot-gles | 47.1 [44.7–48.6] | 1.8 | 26 [26–26] | 4708 [4471–4860] | 0 |
| | niri-off | 314 [311–314] | 12.4 | 112 [105–115] | 31389 [31121–31428] | 0 |
| | niri-on | 299 [296–315] | 11.9 | 110 [108–113] | 29911 [29578–31464] | 0 |
| animate (10 s) | scoot-pixman | 1226 [1215–1263] | 12.2 | 151 [151–151] | – | 49.8 |
| | scoot-gles | 7892 [7753–7893] | 78.8 | 1010 [1002–1021] | – | 43.0 |
| | niri-off | 8020 [7935–8051] | 80.1 | 1688 [1686–1693] | – | 54.1 |
| | niri-on | 8030 [8029–8038] | 80.2 | 1684 [1681–1690] | – | 54.1 |

#### CPU per presented frame

Main-pass CPU median divided by DIAG-pass frame median. Because the two
come from separate passes, this is a ratio of medians, not a per-round
figure.

| scene | scoot-pixman | scoot-gles | niri-off | niri-on |
|---|---|---|---|---|
| pointer | no frames | no frames | 16.20 ms (594 frames) | 16.21 ms (596) |
| relayout | 1.87 ms (201 frames) | 20.49 ms (201) | 14.16 ms (597) | 14.34 ms (594) |
| animate | 2.46 ms (499 frames) | 18.35 ms (430) | 14.82 ms (541) | 14.81 ms (542) |

#### Memory

`/proc/PID/smaps_rollup`, in MB (the raw kB values are in the evidence).
Rss / Pss, with Pss split into anonymous and file-backed. Medians of three
rounds; every range was within 0.2 MB except where one is shown.

| when | variant | Rss | Pss | Pss anon | Pss file | threads |
|---|---|---|---|---|---|---|
| empty session | scoot-pixman | 29.1 | 26.0 | 13.9 | 5.9 | 1 |
| | scoot-gles | 122.5 | 118.9 | 39.9 | 72.7 | 10 |
| | niri-off | 165.5 | 142.9 | 54.7 | 75.7 | 16 |
| | niri-on | 165.5 | 142.9 | 54.7 | 75.7 | 16 |
| three `foot`s | scoot-pixman | 48.8 | 38.6 | 14.0 | 5.8 | 1 |
| | scoot-gles | 148.8 | 140.9 | 52.8 | 72.3 | 10 |
| | niri-off | 185.3 | 155.5 | 63.7 | 74.4 | 16 |
| | niri-on | 185.6 | 155.8 | 64.1 | 74.4 | 16 |
| after 20 screenshots | scoot-pixman | 61.7 | 48.3 | 20.4 | 5.9 | 3 |
| | scoot-gles | **283.6** | 274.0 | **184.2** | 72.4 | 12 |
| | niri-off | 217.9 [205.4–218.0] | 205.9 | 89.6 | 92.6 | 17 |
| | niri-on | 212.9 | 201.0 | 84.7 | 92.6 | 17 |
| end (after animate) | scoot-pixman | 68.0 | 51.4 | 20.4 | 5.8 | 3 |
| | scoot-gles | 289.8 | 277.0 | 184.2 | 72.3 | 12 |
| | niri-off | 208.6 [208.6–214.7] | 193.4 | 74.1 | 92.4 | 17 |
| | niri-on | 219.2 | 203.9 | 84.7 | 92.3 | 17 |

About 72–75 MB of both scoot-gles's and niri's Pss is file-backed Mesa and
LLVM. Compare those two when you want a like-for-like GL stack; pixman
against niri is the comparison for a machine without a GPU. The
scoot-gles jump after screenshots is the leak described above. A probe on
its own shows it linear and unbounded: 6.25 MB per capture, reaching 885 MB
after 120 captures of a still screen. Drawing frames afterwards did not
bring RSS back down (compare the "end" row). It did stop further captures
from growing it. The ticket records the details, one of which is still
unexplained.

#### Startup

Milliseconds from `exec` of the compositor.

| variant | IPC answering | first frame committed to the host (DIAG) |
|---|---|---|
| scoot-pixman | 15.6 [6.7–17.4] | 27.1 [24.8–27.2] |
| scoot-gles | 41.7 [41.1–47.2] | 62.8 [62.4–63.2] |
| niri-off | 112.7 [111.8–114.6] | 112.9 [111.5–114.7] |
| niri-on | 114.6 [113.7–123.9] | 111.6 [109.7–112.8] |

#### Screenshot latency

Wall milliseconds per capture, across all 30 captures of each kind, from
the start of the client command to a complete PNG on disk.

| variant | own IPC | PNG bytes | grim | PNG bytes |
|---|---|---|---|---|
| scoot-pixman | 11.3 [7.0–13.0] | 40,537 | 55.9 [50.1–58.0] | 10,240 |
| scoot-gles | 9.4 [8.7–10.4] | 40,537 | 56.1 [48.2–59.6] | 10,239 |
| niri-off | 34.3 [28.7–38.2] | 11,774 | 43.0 [35.4–44.6] | 10,225 |
| niri-on | 33.9 [29.8–38.1] | 11,775 | 42.6 [31.4–44.7] | 10,225 |

#### Binaries

| | scoot | niri |
|---|---|---|
| size | 5.85 MB (stripped by the release profile) | 35.4 MB as shipped by nixpkgs (not stripped); 23.4 MB stripped |
| shared libraries (`ldd`) | 22 | 59 |

scoot's list is libinput, libseat, libudev, libxkbcommon, pixman and their
dependencies. libEGL and libGLESv2 are loaded at runtime only under
`--renderer gles`, so `ldd` does not show them; the Pss-file column above
does. niri also links EGL, GBM, PipeWire, pango/cairo, fontconfig/freetype
and the X11 client libraries. Its runtime closure in the Nix store is 738 MB.

### What these numbers do not show

- **This is a VM and llvmpipe.** Every GLES number, niri's and
  scoot-gles's alike, is software rasterisation on 4 vCPUs. On a real GPU,
  per-frame GLES costs change completely. That half is Asahi.md Test 9.
- **Nested, not `--tty`.** Neither compositor's KMS path, cursor plane or
  vblank pacing is exercised. Frames are paced by the host's 60 Hz frame
  callbacks.
- **The pointer row compares different work.** Nested, niri composites its
  own pointer into every frame (56.5 frames a second while it moves). scoot
  `--nested` draws no pointer at all; the host shows its own. So 9.6 s
  against 83 ms is niri re-rendering its output 57 times a second against
  scoot only dispatching input. It does not mean niri's input handling costs
  100x. On a real session both draw a pointer.
- **Relayout goes through two different IPC clients**, `scootctl action
  focus-column left` against `niri msg action focus-column-left`. Their own
  CPU is not counted, but they differ: in a separate probe, one `niri msg
  action` invocation took 16.3 ms end to end (about one frame) against
  0.5 ms for `scootctl action`. Both kept the 20-a-second
  pace (niri 198–199 of 200, scoot 200). niri presented about three frames
  per action and scoot one, which shows in the per-action column. The
  per-frame table removes that difference.
- **The screenshot paths are not the same work.** niri's also puts the image
  on the clipboard, answers before the file is written, and compresses
  harder: its PNG is 11.8 kB against scoot's 40.5 kB. Latency is therefore
  measured to a complete PNG on disk, polled every 1 ms. For `grim`, scoot
  speaks ext-image-copy-capture and niri wlr-screencopy. scoot deliberately
  parks each capture until its next frame tick (see the module doc of
  `crates/scoot/src/compositor/screencopy.rs`), which accounts for most of
  its extra `grim` latency.
- **The configs are as close as niri allows, not identical.** niri's ring
  is its `border`, which is a different implementation from scoot's ring:
  niri's takes its width out of the layout, while scoot draws its ring in the
  gap. Window sizes therefore differ by a few pixels. niri also runs
  machinery that scoot lacks, such as its config-file watcher (see the
  artifact below).
- **The `gpu-scanout` build was not measured separately, because nested it
  changes nothing here.** Its dma-buf presenter needs the host's
  `zwp_linux_dmabuf_v1` at version 4 (`crates/scoot/src/compositor/nested/gpu.rs`,
  `try_negotiate`). cage on pixman offers no `linux-dmabuf` (its globals are
  in the evidence), so that build logs `presenting to the host by read-back`
  and presents exactly as the default build does. Asahi.md Test 9 Part A
  gives the host GLES so that the path comes up.
- **Input-to-present latency was not measured.** Nested, there is no signal
  both compositors expose for "this input reached the screen". scoot draws
  no nested pointer, and the host's presentation feedback times the host's
  frames rather than the nested compositor's. It remains an open question
  for real hardware.
- **scoot presented fewer frames than niri for the animating client** (49.8
  a second against 54.1). The client prints about one line per 16 ms.
  scoot-pixman is far from CPU-bound there (12% of a core), so the cause is
  pacing rather than cost. It is not investigated here and is
  [filed](backlog/core/nested-frame-rate-vs-client.md).
- **A harness artifact was caught and fixed.** A first full run had niri-off
  reading its config from the VM's 9p mount of this checkout. niri watches
  its config file, and on 9p that cost it about 55 extra wakeups a second at
  idle (1153 against 62 over 20 s in a direct probe). niri-on already read
  a local copy. Both configs are now copied to local disk, and every number
  above comes from the re-run. The first run is kept in the evidence, marked
  superseded.

### Reproducing

On a machine with `cage`, `wlr-randr`, `foot` and `grim` on `PATH`:

```sh
cargo build --release -p scoot -p scootctl
cargo build --release --manifest-path scripts/niri-ab/vptr/Cargo.toml --target-dir /tmp/vptr
NIRI=$(nix build --no-link --print-out-paths nixpkgs#niri)/bin/niri
common="SCOOT=$PWD/target/release/scoot SCOOTCTL=$PWD/target/release/scootctl NIRI=$NIRI VPTR=/tmp/vptr/release/nab-vptr"
env $common OUT=/tmp/niri-ab scripts/niri-ab-bench.sh
env $common OUT=/tmp/niri-ab-diag DIAG=1 scripts/niri-ab-bench.sh
scripts/niri-ab/summarize.sh /tmp/niri-ab /tmp/niri-ab-diag
```

About 16 minutes per pass at the defaults. Keep `OUT` on a local disk. The
script copies niri's config there for the reason given in the last caveat
above.

The evidence for the run above is on the dev VM under `~/evidence/niri-ab/`:
- `run-main/` and `run-diag/`: every raw TSV, and each session's compositor
  and host logs (the DIAG host logs gzipped);
- `summary.md`: the tables above, before rounding;
- `versions.tsv` in each run directory;
- `scripts-used/`: the exact scripts, with their sha256.
  `bench-script-after-runs.diff` holds every change made to the benchmark
  script since the run: a per-session watchdog, `timeout` on the calls that
  spawn clients and count windows, and each session directory recreated
  from scratch (a re-run into the same `OUT` used to read the previous run's
  pid). Neither the measured calls nor the startup poll changed. The pointer helper has changed
  only by a clippy fix since (`events % 2 == 0` became
  `events.is_multiple_of(2)`);
- `static-facts.txt`: sizes and `ldd`;
- `cage-host-wayland-info.txt`: the host's globals;
- the `probe-*.txt` files behind the leak, the IPC client costs and the 9p
  artifact;
- `superseded-9p-config/`: the discarded first run.
