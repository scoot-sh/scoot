# scootbar

A status bar for scoot, and for any compositor with `wlr-layer-shell-v1`:
the lightest bar that is still beautiful and configurable. It starts as a
clock, gains workspaces, and grows by modules, one small daemon of a
composable shell.

> **Status: early.** `scootbar daemon` puts a bar with a clock on every
> output (or the ones you list), reserving its space, across outputs coming and going; the flags
> are in [cli.md](cli.md). Workspaces are one flag away
> (`--left workspaces`: each output's numbers with the active one
> marked, click to switch). That is M1's
> [skeleton](backlog/resolved/skeleton-layer-surface-done.md) and its
> [module API and clock](backlog/resolved/module-api-and-clock-done.md),
> and its [Nix package](backlog/resolved/nix-package-done.md), which
> ships M1's `nix run`: a clock you can run in one command. Its
> [testing and CI](backlog/resolved/testing-and-ci-done.md) closes M1:
> snapshot, harness and fuzz layers, a CI path of the bar's own, and the
> benchmark script ([testing.md](testing.md)). M0, the
> measuring milestone, is done: the competitor baselines below, and the
> font rasterizer and clock choices, recorded in
> [the dependency record](backlog/resolved/dependencies-done.md). The
> plan, milestone by milestone, is the [backlog](backlog/README.md).

```sh
scootbar daemon --font /path/to/DejaVuSans.ttf &            # 3:07 pm, centered
scootbar daemon --clock-format '%a %d %b %H:%M' --right clock &
scootbar daemon --left workspaces --center clock --font /path/to/DejaVuSans.ttf &
                                                            # workspaces on the left, the clock centered
nix run github:scoot-sh/scoot#scootbar-demo                 # with a font, from Nix
```

From Nix, `packages.<system>.scootbar` is the bar with no font in its
closure (give it `--font`), and `scootbar-demo` the same bar with DejaVu
Sans as its default font ([docs/nix.md](../nix.md#the-status-bar-scootbar)).
`programs.scootbar` (home-manager and NixOS modules, a restarting user
service, Stylix defaults when Stylix is in use) is documented there too
([the modules](../nix.md#the-modules-programsscootbar)).

Every option lives in `$XDG_CONFIG_HOME/scoot/bar.toml` too (the flags
override it, at start-up and on every reload), and the running bar answers
`scootbar msg query` with each module's state as JSON — the agent hook,
the bar read as data instead of OCR. The keys, the file and the commands
are in [cli.md](cli.md#the-config-file).

## What it is for

- **The lowest resource use of any bar at the same scope**, checked at
  every milestone against the last one and against the bars below, in the
  [resource ratchet](backlog/lightest.md). A loss is a finding to fix.
- **Event-driven, no polling**: one `poll(2)` loop, and idle wakeups are a
  measured row. A minute clock's timer wakes it once a minute (M0's spike
  clock [measured](backlog/resolved/dependencies-done.md#2a-idle-one-wakeup-per-minute)
  it), and each frame it draws brings one `wl_buffer.release` from the
  compositor, so the bar idles at 2 wakeups a minute (the target, [ratified](backlog/lightest.md#decisions)); with no module, 0.
- **Standard protocols first** (`ext-workspace-v1`, layer shell), so it
  runs on other compositors as scootbg does.
- **Configurable without restarting, and readable as data**: the file
  holds every option, `scootbar msg reload` live-applies an edit, and
  `scootbar msg query` reads each module's state as JSON — for daily
  driving and for agents alike.
- **GPU-free**: `wl_shm` buffers at the output's real device pixels.

## Baselines

The bars scootbar is measured against, before scootbar exists, so each
milestone has something to beat. Measured 2026-09-29 on one machine (a 4-vCPU
VM), each bar configured to show a clock (`%a %d %b %H:%M`) and, where it
can, the workspaces, with two windows on two workspaces. The method, the
raw runs and the configs are in the record's
[§4](backlog/resolved/dependencies-done.md#4-baselines); treat times as
noisy. RSS and heap are close to exact; PSS is not, because it divides shared
pages among whatever else maps them (sway and other store processes co-ran in
these runs), so the ratchet's primary memory rows are RSS and heap. "Wakeups"
are voluntary context switches, which a blocking syscall inside a wake
inflates on a busy machine. scootbar's own column arrives with M1.

What each shows: Waybar and ashell show the workspaces on both compositors;
yambar and ironbar show only the clock on scoot (neither speaks
`ext-workspace-v1`) and both on sway.

**On scoot** (`--headless`, one 1920×1080 output, pixman):

| | yambar 1.11.0 | Waybar 0.15.0 | ironbar 0.19.0 | ashell 0.10.0 |
|---|---|---|---|---|
| Idle RSS | 13.9 MiB | 52.6 MiB | 58.2 MiB | 29.8 MiB |
| Idle PSS | 7.2 MiB | 42.2 MiB | 47.8 MiB | 26.3 MiB |
| Idle heap (`RssAnon`) | 1.8 MiB | 8.1 MiB | 11.4 MiB | 4.9 MiB |
| Peak memory (`VmHWM`) | 13.9 MiB | 52.6 MiB | 58.2 MiB | 30.4 MiB |
| Idle wakeups per minute | 4.0 | 5.0 | 310.0 | 227.8 |
| Idle CPU, 300 s window | 3.6 ms | 9.1 ms | 170.6 ms | 124.7 ms |
| CPU, 240 workspace switches in 60 s | 0.6 ms (no workspaces shown) | 420.4 ms | 83.1 ms (no workspaces shown) | 312.1 ms |
| Startup to first frame (median of 5) | 28.5 ms | 142.5 ms | 124.4 ms | 30.1 ms |
| Binary | 407 KB | 3.9 MB | 37.2 MB | 41.6 MB |
| Installed closure (nixpkgs) | 771 MB | 1,037 MB | 1,232 MB | 720 MB |
| Threads | 3 | 8 | 15 | 9 |

**On sway 1.12** (headless, one 1920×1080 output, pixman):

| | yambar 1.11.0 | Waybar 0.15.0 | ironbar 0.19.0 | ashell 0.10.0 |
|---|---|---|---|---|
| Idle RSS | 13.9 MiB | 52.7 MiB | 57.6 MiB | 29.0 MiB |
| Idle PSS | 7.2 MiB | 42.4 MiB | 47.3 MiB | 25.8 MiB |
| Idle heap (`RssAnon`) | 1.8 MiB | 8.3 MiB | 11.4 MiB | 4.5 MiB |
| Peak memory (`VmHWM`) | 13.9 MiB | 52.7 MiB | 57.6 MiB | 29.4 MiB |
| Idle wakeups per minute | 2.6 | 3.0 | 307.8 | 192.6 |
| Idle CPU, 300 s window | 3.2 ms | 8.1 ms | 174.9 ms | 112.7 ms |
| CPU, 240 workspace switches in 60 s | 153.5 ms | 399.0 ms | 268.7 ms | 311.3 ms |
| Startup to first frame (median of 5) | 20.7 ms | 132.4 ms | 111.7 ms | 25.3 ms |
| Binary, closure, threads | as on scoot (4 threads for yambar, its `i3` module's) | as on scoot | as on scoot | as on scoot |

**scootbar's skeleton** (M1's first step: a solid bar, no clock, no
workspaces, so not yet a like-for-like column), measured the same way on
the same machine the same day, release build at `62181e6`:

| | on scoot | on sway |
|---|---|---|
| Idle RSS | 2.7 MiB | 2.7 MiB |
| Idle PSS | 1.0 MiB | 1.1 MiB |
| Idle heap (`RssAnon`) | 0.16 MiB | 0.17 MiB |
| Peak memory (`VmHWM`) | 2.7 MiB | 2.7 MiB |
| Idle wakeups per minute | **0** | **0** |
| Idle CPU, 300 s window | 0.00 ms | 0.00 ms |
| CPU, 240 workspace switches in 60 s | 0.00 ms (no workspaces shown) | 0.00 ms (no workspaces shown) |
| Startup to first frame (median of 5) | 7.2 ms | 7.0 ms |
| Binary | 603 KB (links only glibc and libgcc_s) | as on scoot |
| Threads | 1 | 1 |

The raw runs are in the
[skeleton's record](backlog/resolved/skeleton-layer-surface-done.md#evidence).

**scootbar with the clock** (M1's
[module API and clock](backlog/resolved/module-api-and-clock-done.md)),
showing `%a %d %b %H:%M` as the other bars did, in DejaVu Sans 2.37 from
the pinned nixpkgs, measured the same way on the same machine the same
day, release build at `44fc656`. Like yambar and ironbar on scoot, it
shows no workspaces yet, so the switching row is a bystander's:

| | on scoot | on sway |
|---|---|---|
| Idle RSS | 3.9 MiB (3.4 MiB with the font mapped) | 3.9 MiB |
| Idle PSS | 2.1 MiB (1.6 MiB mapped) | 2.1 MiB |
| Idle heap (`RssAnon`) | 0.92 MiB (0.18 MiB mapped) | 0.92 MiB |
| Peak memory (`VmHWM`) | 3.9 MiB | 3.9 MiB |
| Idle wakeups per minute | **2.0** | **2.0** |
| Idle CPU, 300 s window | 1.96 ms | 1.46 ms |
| CPU, 240 workspace switches in 60 s | 0.30 ms (no workspaces shown) | 0.24 ms (no workspaces shown) |
| Startup to first frame (median of 5) | 9.9 ms | 8.7 ms |
| Binary | 849 KB (links only glibc and libgcc_s) | as on scoot |
| Installed closure (this flake) | 49 MB, no font (measured at the [Nix package](backlog/resolved/nix-package-done.md#evidence)) | as on scoot |
| Threads | 1 | 1 |

- **Two wakeups a minute, not one**: the tick, and about a millisecond
  later the compositor's `wl_buffer.release` for the buffer the tick's
  frame replaced, on both compositors (the record has the trace). Every
  `wl_shm` client gets that reply once per frame. It is still the fewest of
  any bar here (yambar's 2.6 to 4).
- **The font's bytes**: DejaVu Sans is read into the heap (742 KiB) unless
  it is a root-owned, unwritable file on a read-only mount, where it is
  mapped: NixOS's `/nix/store` is that, this machine's store is not
  mounted read-only, so the main column is the read one, and the mapped
  figures come from a root-owned `0444` copy on a read-only bind mount,
  measured again at `de27775` after review tightened the rule
  ([cli.md](cli.md#fonts) says why).
- **The binary** is 246 KB over the skeleton's: the rasterizer and font
  parser about 103 KB (M0 predicted 115), scootbar's own clock, text,
  layout and render code about 52 KB, generic code from `core`, `std` and
  `alloc` the rest. yambar's 407 KB links `libwayland-client`, `pixman`
  and `fcft` besides; scootbar's is the whole of it.

How the rows were taken, in short (the full method is in the record):

- **Wakeups** are voluntary context switches summed over every thread of
  the bar, over a 300 s window started after a fixed 30 s settle.
  Involuntary switches were 0–15 in every window and are not counted.
- **CPU** is on-CPU time summed over the bar's threads
  (`/proc/PID/task/*/schedstat`) over the same window.
- **Memory** is read at the end of the idle window; peak is the process's
  `VmHWM`, startup included. MiB are 1024² bytes.
- **Startup** is from `exec` to the first `commit` of a buffer on the bar's
  surface, from `WAYLAND_DEBUG` output: the first frame, whatever it shows.
- **Binary** is the executable itself (nixpkgs' unwrapped one); the
  **closure** depends on nixpkgs' default features as much as on the bar.
  scootbar's own, this flake's `packages.x86_64-linux.scootbar` on the
  pinned nixpkgs, measured the competitors' way (`nix path-info -S`), is
  49 MB (49,078,696 bytes): the bar 826 KiB (the package's build, 845,152
  bytes), glibc with libidn2 and libunistring 36.0 MiB, gcc's runtime
  10.2 MiB (`gcc-lib` 9.8 MiB, which carries libgcc_s, plus two 193 KiB
  `libgcc` paths), and no font. It is the size row
  as [ruled](backlog/lightest.md#decisions): the binary plus what it links.
- One idle and one switching run per bar and compositor; startup is the
  median of 5.

Not measured: **i3status-rust** (a status-line generator that needs
`swaybar`, so sway only and not a like-for-like bar; see the record), any
real hardware, a third compositor, and the competitors' lines of code and
dependency counts ([lightest](backlog/lightest.md) wants those rows for
scootbar itself). yambar, Waybar and ironbar are
MIT; ashell and i3status-rust are GPL, and were only run as binaries.

### M1, like for like, by the benchmark script

From M1 on, each milestone's numbers come from
[`scripts/scootbar-bench`](testing.md#benchmark), with every bar showing
exactly what scootbar shows: here the clock alone (`%a %d %b %H:%M`), so
yambar and Waybar run without their workspaces. Same machine, same stage
as above; release scootbar (identical to `main`'s), a debug scoot, sway
1.12, one idle and one switching run per bar, startup the median of 5
[with the range]. The raw run is kept as the next milestone's baseline in
[`bench/m1-clock`](bench/m1-clock/table.md). Startup is timed where the
compositor receives the frame, with the compositor printing its protocol,
so it is slower for every bar than the tables above and is compared only
within this run. **No competitor beats scootbar on any gated row.**

| | scootbar | yambar 1.11.0 | Waybar 0.15.0 |
|---|---|---|---|
| Idle RSS, scoot / sway | 3.9 / 3.9 MiB | 14.1 / 13.8 MiB | 52.5 / 52.4 MiB |
| Idle PSS | 2.2 / 2.2 MiB | 8.6 / 7.8 MiB | 45.8 / 43.2 MiB |
| Idle heap (`RssAnon`) | 0.9 / 0.9 MiB | 1.8 / 1.8 MiB | 8.1 / 8.3 MiB |
| Idle wakeups per minute | 2 / 1 | 4 / 3 | 5 / 3 |
| Idle CPU, 300 s window | 1.6 / 1.6 ms | 3.5 / 3.4 ms | 8.2 / 7.3 ms |
| CPU, 240 workspace switches in 60 s | 0.1 / 0.2 ms | 0.7 / 0.5 ms | 1.6 / 1.3 ms |
| Startup to first frame | 21.4 [18.0–37.2] / 24.2 [12.7–27.5] ms | 44.3 [41.3–81.1] / 49.8 [31.2–77.0] ms | 178 [165–267] / 165 [158–216] ms |
| Size: stripped binary + non-glibc `ldd` closure | 1.03 MB | 19.7 MB | 71.4 MB |
| Threads | 1 | 3 | 8 |

scootbar is 10,628 lines of Rust (6,995 outside `tests.rs` files) with 6
direct dependencies on Linux. Its one wakeup a minute on sway (two in the
clock's own run above) is most likely timing, not a change: sway sends the
`wl_buffer.release` within a quarter of a millisecond of the commit (the
clock's record traced it), which can land before the bar has gone back to
sleep. The run's 5 involuntary switches beside 5 voluntary ones fit that;
it was not traced again here.

### M3, clock and workspaces, on the Asahi M2

The M3 ratchet run, by [`scripts/scootbar-bench`](testing.md#benchmark) at
`--scope clock-workspaces` (scootbar `--left workspaces --right clock`;
Waybar's `ext/workspaces` on scoot and `sway/workspaces` on sway; yambar's
`i3` module on sway). **Machine**: the maintainer's Asahi MacBook Air, Apple
M2 (4 Blizzard + 4 Avalanche cores, 16 KiB pages), Linux 7.1.13 aarch64,
`schedutil`, on mains power with the battery full, release builds of scootbar
(`main` at `3211551`; `main` has since moved only by a backlog claim, so
the code is the same) under headless scoot (release, same binary for
every run) and sway 1.12, yambar 1.11.0 and Waybar 0.15.0 from the pinned
nixpkgs. Raw runs, with `meta.json` (every reading of the machine) and
`run.log`: [`bench/m3-asahi-clock-workspaces`](bench/m3-asahi-clock-workspaces/table.md).
A fanless machine: the readings the kernel gives are in
[the record](#what-the-machine-did), none shows a throttle.

| | scootbar | yambar 1.11.0 | Waybar 0.15.0 |
|---|---|---|---|
| Idle RSS, scoot / sway | 3.5 / 3.5 MiB | not compared / 13.4 MiB | 51.4 / 51.4 MiB |
| Idle PSS | 2.1 / 2.1 MiB | not compared / 8.0 MiB | 45.5 / 43.2 MiB |
| Idle heap (`RssAnon`) | 0.4 / 0.4 MiB | not compared / 3.1 MiB | 10.7 / 10.8 MiB |
| Idle wakeups per minute | 2 / 2 | not compared / 3 | 5 / 3 |
| Idle CPU, 300 s window | 1.0 / 0.8 ms | not compared / 2.5 ms | 5.1 / 4.1 ms |
| CPU, 240 workspace switches in 60 s | 26.3 / 22.2 ms | not compared / 177 ms | 464 / 1425 ms |
| Startup to first frame | 37.3 [33.2-45.1] / 18.2 [9.2-21.4] ms | not compared / 28.4 [17.4-33.1] ms | 97.8 [82.4-110] / 62.9 [59.7-70.7] ms |
| Size: stripped binary + non-glibc `ldd` closure | 1.38 MB | 20.95 MB | 73.97 MB |
| Threads | 1 | 4 (sway) | 8 / 9 |

**yambar is not compared on scoot**: 1.11.0 has no ext-workspace-v1 module,
so it cannot show workspaces there, and the harness neither runs it nor
invents a number ([testing.md](testing.md#benchmark)). **No competitor beats
scootbar on any row it was measured on**: three ties, each within the noise
rule (startup against yambar, idle wakeups against yambar and Waybar, both on
sway; scootbar's 2 a minute meets the ratified target). The rule's second
half still fails, since one competitor and compositor pair was not compared
at all. scootbar's cost of a workspace switch at this scope is 26.3 ms for 240
switches against Waybar's 464 ms and yambar's 177 ms (sway).

**Against its own M1, like for like.** `bench/m1-clock` was measured on a
different machine (a 4-vCPU Xeon container, debug scoot, kernel 6.18) and is
not comparable with this box's numbers: M1's scootbar read 3.9 MiB there and
reads 2.9 MiB here. So M1's scootbar was rebuilt on the box (`3801c12`,
release, `cargo build --release -p scootbar`, today's toolchain) and both it
and M3's (`3211551`, built the same way) ran **the same harness, at the clock
scope M1 measured, one after the other, A-B-B-A** with 180 s between runs, so
each side has two runs and each pairing of a run with the other side's is a
compare. The table is the first run of each side; the second is in the
directories below.

| Clock scope, scoot / sway | M1 `3801c12` | M3 `3211551` | regressed in how many of the 4 pairings |
|---|---|---|---|
| Idle RSS | 2.9 / 2.9 MiB | 3.5 / 3.5 MiB | 4 and 4 |
| Idle PSS | 1.5 / 1.5 MiB | 2.1 / 2.1 MiB | 4 and 4 |
| Idle heap (`RssAnon`) | 0.3 / 0.3 MiB | 0.4 / 0.5 MiB | 4 and 4 |
| Peak memory (`VmHWM`) | 2.9 / 2.9 MiB | 3.5 / 3.5 MiB | 4 and 4 |
| **CPU, 240 workspace switches in 60 s** | 0.2 / 0.1 ms | **12.0 / 10.5 ms** | 4 and 4 |
| Wakeups over those switches (not gated) | 2 / 2 | 242 / 242 | (not gated) |
| Idle CPU, 300 s window | 0.9 / 0.6 ms | 1.2 / 0.6 ms | 3 and 1: noise (see below) |
| Idle wakeups per minute | 2 / 2 | 2 / 2 (sway 1.8 in the first run) | 0 and 0 |
| Startup to first frame | 31.4 / 17.6 ms | 34.9 / 8.4 ms | 0 and 0 |
| Size: stripped binary + non-glibc `ldd` closure | 924,320 B | 1,383,080 B | 4 |
| Bare executable, stripped (not gated) | 791,224 B | 1,249,984 B | 4 |
| Lines of Rust, all / outside `tests.rs` | 10,628 / 6,995 | 25,347 / 16,312 | (reported) |
| Direct dependencies on Linux | 6 | 10 (adds `png`, `serde`, `serde_json`, `toml`) | (reported) |

**The no-regression rule fails at M3.** Memory is up by about 0.6 MiB
(20%), the stripped binary and its closure by 50%, and the **CPU and wakeups
of a workspace switch at the clock scope, where the bar shows no workspaces,
went from 2 wakeups to 242 over 240 switches and from about 0.2 ms to 12 ms**.
The likely cause (read, then confirmed by a protocol trace of one run of
`scootbar daemon --right clock` on headless scoot: six switches produced
eight `ext_workspace_manager_v1.done` events and the matching
`ext_workspace_handle_v1.state` events, all received by a bar with no
workspaces module): `daemon/wayland.rs` binds `ext_workspace_manager_v1`
whenever the `workspaces` feature is compiled in, placed or not, so the bar
is woken for, and parses, every workspace change it never shows (its open
descriptors went from 5 to 8 too). The harness measured this and did not
change it; the fix is for the maintainer to schedule. The idle-CPU row is not
called a regression: on scoot it flags in 3 of the 4 pairings and on sway
in 1 of 4, with values of 0.8 to 1.2 ms against a noise rule built for
runs with one idle sample each.

**Noise, measured.** M1 against its own rerun flags one row as regressed
(sway `RssAnon`, 272 kB against 288 kB: one 16 KiB page, with one idle
sample so the spread is 0 and only the unit's floor applies) and exits 1;
M3 against its own rerun flags none (exit 0) and two as better. Every row
called a regression above is a difference of at least 0.1 MiB (memory),
50 times (switching CPU) or 459 KB (size) and reproduces in all four
pairings.

#### What the machine did

Every record carries the readings before and after it
(`hw_start`/`hw_end`), summarized by `report`. Across the five runs
(60 to 24 readings each): governor `schedutil`; no cpufreq policy cap below
the hardware maximum (2424 MHz on the efficiency cores, 3204 MHz on the
performance ones) in any of them; mains online and the battery full at the
start and end of every run; hwmon temperatures 22 to 27 C (the NAND,
battery, charger and WiFi sensors: **the M2 exposes no CPU temperature**, so
a throttle could only show as a policy cap, and none did; the current
frequency ranged 600 to 3204 MHz, which is the governor, not a cap); load
average at most 0.14 at any run's start. The runs had 180 s of cool-down
between them and each ran about 13 to 16 minutes (clock scope, scootbar
alone) or 37 minutes (clock and workspaces, three bars). The cores are not
pinned: a bar's CPU rows can differ with which cluster it landed on, which
is what the repeated runs and the rule's margin are for.

Runs, all under [`bench/`](bench/README.md): `m3-asahi-m1-baseline-clock` and
`-rerun` (M1's scootbar), `m3-asahi-clock` and `-rerun` (M3's), and
`m3-asahi-clock-workspaces`.
