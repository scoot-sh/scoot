# scootbar

A status bar for scoot, and for any compositor with `wlr-layer-shell-v1`:
the lightest bar that is still beautiful and configurable. It starts as a
clock, gains workspaces, and grows by modules, one small daemon of a
composable shell.

> **Status: early.** `scootbar daemon` puts a bar with a clock on every
> output (or the ones you list), reserving its space, across outputs coming and going; the flags
> are in [cli.md](cli.md). Workspaces are one flag away
> (`--left workspaces`: each output's numbers with the active one
> marked, click to switch). Every module answers the pointer: clicks,
> scrolls and hover run a module action, a command or a request to scoot
> that the config binds ([pointer input](cli.md#pointer-input), M4's
> [first step](backlog/resolved/pointer-and-interactions-done.md)), and
> the config can define a `button`, a `push` target for `scootbar msg set`
> and an `exec` module that streams a command's output, with no Rust
> ([how](cli.md#button-push-and-exec-modules)). That is M1's
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
the bar read as data instead of OCR. `msg layout` says where each module
is, `msg invoke` presses one, and `msg subscribe` streams what changes
([the agent interface](cli.md#the-agent-interface)). The keys, the file and
the commands are in [cli.md](cli.md#the-config-file).

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
- **Interactive at no cost until used**: a module's `on-click` and
  `on-scroll-*` keys run an action or a command, hover repaints one module,
  and a bar with no binding never takes the pointer at all.
- **Configurable without restarting, and readable as data**: the file
  holds every option, `scootbar msg reload` live-applies an edit, and
  `scootbar msg query` reads each module's state as JSON, `layout` and
  `invoke` let an agent press a module with no pixels to hunt, and
  `subscribe` streams changes, one batch a frame at most and nothing with no
  subscriber — for daily driving and for agents alike.
- **GPU-free**: `wl_shm` buffers at the output's real device pixels.

## Baselines

The bars scootbar is measured against, before scootbar exists, so each
milestone has something to beat. Measured 2026-09-29 on one machine (a Claude Code
cloud sandbox VM, 4 vCPUs, not hardware the maintainer owns), each bar configured to show a clock (`%a %d %b %H:%M`) and, where it
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
scootbar on any gated row** (the bare stripped yambar executable, 396 kB, is smaller than scootbar's 1.25 MB; the gated size row counts the closure, where yambar is 20.95 MB on nixpkgs' default features): three ties, each within the noise
rule (startup against yambar, idle wakeups against yambar and Waybar, both on
sway; scootbar's 2 a minute meets the ratified target). The rule's second
half still fails, since one competitor and compositor pair was not compared
at all. scootbar's cost of a workspace switch at this scope is 26.3 ms for 240
switches against Waybar's 464 ms and yambar's 177 ms (sway).

**Against its own M1, like for like.** `bench/m1-clock` was measured on a
different machine (a Claude Code cloud sandbox VM (4-vCPU Xeon), not hardware the maintainer owns; debug scoot, kernel 6.18) and is
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
descriptors also went from 5 to 8; binding a global opens none, so that is not attributed to this). The harness measured this and did not
change it. Fixed since: the daemon binds the workspace manager and the seat only while a workspaces module is placed; the switching rows are back to M1's (`bench/m3-asahi-clock-bindfix`, one run, and [lightest](backlog/lightest.md#m3-gate-clock-and-workspaces-measured-2026-09-30-does-not-pass)'s after-the-fix note), and the memory and size rows are not changed by it. The idle-CPU row is not
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

### M3, all five bars, on the Asahi M2

The same run as [M3 above](#m3-clock-and-workspaces-on-the-asahi-m2), with
ironbar and ashell added, and nothing else changed: scope
`clock-workspaces`, 5 startup rounds, a 30 s settle, a 300 s idle window,
240 switches at 4 a second, 180 s of cool-down before. **ironbar and ashell
are informational columns.** The ratified competitors are still yambar and
Waybar ([Decisions](backlog/lightest.md#decisions)); the gate does not count
the extra two (they cannot lose a row for scootbar, and a pair they cannot
show is not "not compared"), and whether to promote them into the rule is the
maintainer's call. They run as the binaries nixpkgs builds (ironbar 0.19.0,
MIT; ashell 0.10.0, GPL-3.0-or-later), and their configs set only the keys
that give the common look ([testing.md](testing.md#benchmark)).

**Machine and commit**: the same Asahi MacBook Air as above (Apple M2, 8
cores, 16 KiB pages, Linux 7.1.13 aarch64, `schedutil`, on mains power with
the battery full; 96 readings around the runs show no cpufreq cap below the
hardware maximum, 600 to 3204 MHz seen, hwmon temperatures 22 to 31 C, load
average 0.00 at the start). Measured on 2026-09-30 from 20:47 EDT, by the
harness at commit `80241e5f` (branch `feat/scootbar-bench-ironbar-ashell`);
scootbar's release binary was built from that tree, whose `crates/` is
identical to `main` at `90a96ca0` (M3's code).
**The bind fix (#363, `e6ca6e35`) landed on `main` after this run was built
and is not in it**: it drops the workspace-manager bind when the bar places no
workspaces module, which only matters at the `clock` scope, not at this one,
where scootbar shows workspaces. Raw runs, `meta.json` and `run.log`:
[`bench/m3-asahi-clock-workspaces-all`](bench/m3-asahi-clock-workspaces-all/table.md).

| scoot / sway | scootbar | yambar 1.11.0 | Waybar 0.15.0 | ironbar 0.19.0 (info) | ashell 0.10.0 (info) |
|---|---|---|---|---|---|
| Idle RSS | 3.5 / 3.5 MiB | not compared / 13.4 MiB | 51.3 / 51.4 MiB | cannot show / 56.2 MiB | 28.9 / 28.1 MiB |
| Idle PSS | 2.1 / 2.1 MiB | not compared / 8.0 MiB | 45.6 / 43.2 MiB | cannot show / 47.9 MiB | 26.4 / 25.6 MiB |
| Idle heap (`RssAnon`) | 0.4 / 0.4 MiB | not compared / 3.1 MiB | 10.7 / 10.8 MiB | cannot show / 14.8 MiB | 6.7 / 6.2 MiB |
| Idle wakeups per minute | 2 / 2 | not compared / 3 | 5 / 3 | cannot show / 313 | 276 / 194 |
| Idle CPU, 300 s window | 1.1 / 0.6 ms | not compared / 2.3 ms | 4.3 / 5.5 ms | cannot show / 56.0 ms | 95.1 / 81.4 ms |
| CPU, 240 workspace switches in 60 s | 27.5 / 22.3 ms | not compared / 187 ms | 482 / 1455 ms | cannot show / 127 ms | 267 / 234 ms |
| Wakeups over those switches (not gated) | 482 / 481 | not compared / 1202 | 965 / 2298 | cannot show / 1942 | 3208 / 2912 |
| Startup to first frame | 34.4 [33.6-45.4] / 17.1 [7.8-17.7] ms | not compared / 27.2 [18.8-36.3] ms | 91.7 [85.9-101] / 69.5 [54.4-86.0] ms | cannot show / 61.0 [51.2-66.2] ms | 28.6 [27.3-38.1] / 30.5 [21.1-35.4] ms |
| Size: stripped binary + non-glibc `ldd` closure | 1.38 MB | 20.95 MB | 73.97 MB | 100.86 MB | 39.52 MB |
| Bare executable, stripped (not gated) | 1.25 MB | 396 kB | 2.95 MB | 24.75 MB | 29.11 MB |
| Threads | 1 | 4 (sway) | 8 / 9 | 19 (sway) | 12 / 12 |

scootbar's, yambar's and Waybar's columns are this run's own, a second
measurement of what the [earlier M3 table](#m3-clock-and-workspaces-on-the-asahi-m2)
has (for instance scootbar's startup on scoot reads 34.4 here against 37.3 ms
there, Waybar's switching CPU on scoot 482 against 464 ms). **The extra two
columns, read against scootbar** (by the harness's noise rule): neither
beats scootbar on any gated row on either compositor. Two rows are ties,
both startup against ashell (scoot: ashell's median 28.6 ms is lower than
scootbar's 34.4 ms but inside the combined spread; sway 30.5 against 17.1
ms): the number is lower on one side and the rule calls it a tie, not a win
for ashell. Everywhere else scootbar is ahead: memory 8 times (ashell) to 16
times (ironbar) larger than scootbar's; idle wakeups 194 to 313 a minute
against scootbar's 2; the switching CPU 5.7 times (ironbar, sway) to 10 times
(ashell) scootbar's. ironbar's switching CPU on sway (127 ms) is below
yambar's 187 ms and Waybar's 1455 ms and still 5.7 times scootbar's 22.3 ms.

**ironbar is not run on scoot at this scope**, as yambar is not: it speaks
compositor IPCs (sway, Hyprland, Niri) and has no ext-workspace-v1 support
(M0: `failed to create module Workspaces`), so it cannot show the workspaces
there, and the harness neither runs it nor invents a number.

**Things that make these two columns not like for like**, none changed
to flatter anyone:

- ashell's bar height and font size are not options (iced theme tokens): its
  bar is not 26 px at 14 px; only the font family (DejaVu Sans), the colors
  (`#1e1e2e`, `#cdd6f4`), the clock format and the modules are set. ironbar's `height` is a minimum that GTK may grow.
- ironbar and ashell have no setting for the clock's update interval; they
  run their own timers. ashell's 194 to 276 and ironbar's 313 voluntary
  wakeups a minute are what they do idle in this session, not something the
  config asked for. ashell also retries a PulseAudio connection every 5 s
  and logs `Failed to start PulseAudio thread: Access denied` (the bench
  session runs no sound server); how much of its idle
  cost that is was not separated, and a session with one could read lower.
- **M0's ashell config did not do what it said.** ashell 0.10.0 names its
  clock module `Tempo` and configures it under `[tempo] clock_format`;
  M0's `[clock] format` is reported on its stderr as `Unknown configuration
  field ignored: clock`, so the format was ashell's default (`%a %d %b %R`,
  which draws the same text). It is a presence-only difference here; the
  harness's config uses the real keys and starts without that warning.
- Rendering: ashell (iced/wgpu) and ironbar (GTK 4) needed no software-rendering
  variables on this box's headless sessions (no `WGPU_BACKEND`, no
  `LIBGL_ALWAYS_SOFTWARE`): sampled 8 to 12 s into a run, neither had a DRM
  device open (`/proc/PID/fd`), ashell had loaded libEGL and ironbar libvulkan
  (to probe, it seems) and neither drew on the GPU. That is as measured, not a
  guarantee of what they do with a GPU session.
- The screenshot presence check passes for both on the sides they run (the
  bar's color at the top, something drawn on each placed part): the content
  is only ever checked for presence.

### M4, pointer input, `exec` and the agent interface, on the Asahi M2

The resource ratchet's first run on hardware for M4's stack: three draft PRs
(#364 pointer input, #366 `button`/`push`/`exec`, #367 the agent interface),
each layer's tip, against `main` and against M3's post-fix run
([`m3-asahi-clock-bindfix`](bench/m3-asahi-clock-bindfix/table.md)).
`scripts/scootbar-bench` at `--scope clock --bars scootbar`, the defaults (5
startup rounds, 30 s settle, a 300 s idle window, 240 switches at 4 Hz), the
same machine as M3 (Apple M2, 8 cores, 16 KiB pages, Linux 7.1.13,
`schedutil`, on mains), release scootbar (`lto = "fat"`, stripped) built per
tree in its own worktree and target directory, **one headless release
`scoot`/`scootctl` built from `main` (`7a1f9030a`) for every run** (the same
binary, sha256 `4e105065f5b2...`), and **the harness from `main` for every
run** (its scootbar config and its measuring code are unchanged since
`cbbeffd`; only the competitors' tables differ, `bars.py`/`bench.py`/
`tables.py` adding ironbar and ashell). 180 s of cool-down between runs.
**Eleven runs in three rounds.** Rounds one and two ran **`--compositors scoot`
only** (about 6.5 minutes a run), **a departure from the M3 post-fix baseline,
which ran scoot and sway**: `main`, #364, #366, #367, the experiment below, then
the experiment, #367 and `main` again. Round three restores sway for three
builds (`--compositors scoot,sway`, about 13 minutes a run): `main`, #367 and
the experiment. **#364 and #366 have no sway run.** Nothing on the box but the
run (no bar, compositor or cgroup of anyone's), load average at most 0.28 at
any start. The raw runs, each with `meta.json`, `runs.jsonl`, `table.md` and
`run.log`, are `bench/m4-asahi-clock-*` (`-rerun` is the second round, `both`
the third). Every `compare` as it ran, and the pooled verdicts with the script
that computed them, are in [`bench/m4-asahi-compares.md`](bench/m4-asahi-compares.md).
The stack's tips are those at the time of the run: `meta.json` records #364
`47b413a99`, #366 `cdb9936bd` and #367 `b6dee9710`; #367's tip has since gained
one docs-only commit (its `crates/` tree is identical).

The median of each run, scoot (a build's runs side by side, in order: first
round, second, third):

| Row, scoot | M3 post-fix `cbbeffd` | `main` `7a1f903` | #364 `47b413a` | #366 `cdb9936` | #367 `b6dee97` | `opt-level = "s"` on #367 (experiment) |
|---|---|---|---|---|---|---|
| Startup to first frame (ms) | 31.0 | 30.5 / 37.8 / 29.8 | 31.2 | 37.4 | 39.2 / 32.3 / 38.1 | 29.5 / 31.6 / 30.4 |
| Idle RSS (MiB) | 3.52 | 3.53 / 3.52 / 3.53 | 3.67 | 3.73 | 3.67 / 3.66 / 3.67 | 3.59 / 3.59 / 3.59 |
| Idle PSS (MiB) | 2.12 | 2.06 / 2.06 / 2.06 | 2.20 | 2.26 | 2.20 / 2.20 / 2.20 | 2.12 / 2.13 / 2.12 |
| Idle heap, `RssAnon` (MiB) | 0.42 | 0.44 / 0.44 / 0.44 | 0.45 | 0.45 | 0.45 / 0.45 / 0.45 | 0.44 / 0.45 / 0.44 |
| Peak memory, `VmHWM` (MiB) | 3.52 | 3.53 / 3.52 / 3.53 | 3.67 | 3.73 | 3.67 / 3.66 / 3.67 | 3.59 / 3.59 / 3.59 |
| Idle wakeups per minute | 2 | 2 / 2 / 2 | 2 | 2 | 2 / 2 / 2 | 2 / 2 / 2 |
| Idle CPU, 300 s window (ms) | 1.04 | 0.93 / 0.94 / 0.95 | 0.96 | 0.98 | 1.04 / 0.99 / 1.10 | 1.09 / 1.30 / 1.06 |
| CPU, 240 workspace switches (ms) | 0.25 | 0.25 / 0.20 / 0.25 | 0.20 | 0.23 | 0.21 / 0.24 / 0.21 | 0.25 / 0.24 / 0.25 |
| Wakeups over those switches *(not gated)* | 2 | 2 / 2 / 2 | 2 | 2 | 2 / 2 / 2 | 2 / 2 / 2 |
| Threads *(not gated)* | 1 | 1 / 1 / 1 | 1 | 1 | 1 / 1 / 1 | 1 / 1 / 1 |
| **Size: stripped binary + non-glibc `ldd` closure (B)** | 1,383,080 | 1,383,080 | **1,514,152** | **1,579,720** | **1,645,256** | 1,448,648 |
| Bare executable, stripped *(not gated)* | 1,249,984 | 1,249,984 | 1,381,056 | 1,446,624 | 1,512,160 | 1,315,552 |
| Loaded code and data: `.text` + `.rodata` + `.eh_frame` + `.data.rel.ro` (B) | n/a | 1,160,027 | 1,223,171 | 1,327,699 | 1,358,147 | 1,175,787 |
| `.text` (B) | n/a | 930,792 | 984,552 | 1,079,816 | 1,105,320 | 927,720 |
| Lines of Rust: all / outside tests / direct deps | 25,549 / 16,514 / 10 | 25,550 / 16,515 / 10 | 29,182 / 18,721 / 10 | 32,603 / 21,034 / 10 | 34,909 / 22,779 / 10 | same as #367 |

Sway, round three only (one run each; M3 post-fix, `main`, #367, experiment):
startup 16.8, 16.9, 17.6, 17.9 ms; idle RSS 3.52, 3.55, 3.66, 3.59 MiB; PSS 2.15,
2.11, 2.22, 2.16; heap 0.42, 0.45, 0.44, 0.44; wakeups 2 in all; **idle CPU 0.89,
0.68, 0.83, 0.94 ms**; 240 switches 0.24, 0.20, 0.25, 0.23 ms; size as above.

`main` now reproduces M3 post-fix: `compare` of each of its three runs against
it exits 0, 0 and 1, **the one flag being sway's idle heap, 0.4 to 0.5 MiB, with
code that is byte-identical to the baseline's** (a single 16 KiB page: the row's
flags below are the same size), and the idle CPU row reads better (1.04 to
0.93 to 0.95 ms; sway 0.89 to 0.68). The pinned `scoot` and `main`'s harness
are what make the comparison fair.

**`compare` against M3 post-fix (rule 1), run by run, each as the harness
prints it; nothing is waived.**

| Run | `compare` exit | Gated rows it flags as regressed |
|---|---|---|
| `main` (three) | 0, 0, 1 | only sway's idle heap, in the third (above) |
| #364 | 1 | idle heap 0.4 to 0.5 MiB (0.42 to 0.45: one or two 16 KiB pages); **size** 1,383,080 to 1,514,152 B (+9.5%) |
| #366 | 1 | idle RSS 3.5 to 3.7, idle PSS 2.1 to 2.3, idle heap 0.4 to 0.5, peak memory 3.5 to 3.7 MiB; **size** to 1,579,720 B (+14.2%) |
| #367 (three) | 1, 1, 1 | idle heap 0.4 to 0.5 MiB in all three; **size** to 1,645,256 B (+19.0%) in all three |
| experiment (three) | 0, 1, 0 | second: idle heap and idle CPU (1.0 to 1.3 ms); the experiment's bare executable is flagged in all, not gated |

**Per-PR compares cannot see the cumulative creep, and these show it.** Each
layer against the one under it: #366 against #364 exits **0** (+65,568 B is
inside the size row's 5% margin, 75,708 B), #367 against #366 exits **0**
(+65,536 B), while #364 against `main` exits 1. The **stack total** against
`main` (#367's first run against `main`'s first) exits **1**: size
+262,176 B (+19.0%), idle PSS 2.06 to 2.20 MiB, and idle CPU 0.93 to 1.04 ms
(one run each; pooled below, the idle CPU is not a regression).

**Idle CPU, pooled with the repo's own `verdict()`** (`scripts/scootbg-bench/report.py`,
the runs of a build as one series, margin the largest of 5% of the baseline's
median, the two spreads and 0.1 ms), scoot: `main`'s three runs [0.926, 0.943,
0.951] ms against M3 post-fix's one sample (1.04), better; #364 (0.962) and
#366 (0.979) against `main` pooled, same; **#367's three [1.044, 0.993, 1.096]
against `main` pooled, same** (margin 0.128; the medians differ by 0.10); the
experiment's three [1.086, 1.302, 1.062] against `main` pooled, **same** (margin
0.265; medians 1.086 against 0.943), and against M3 post-fix, same. **An earlier
reading of this table had the experiment regressed: with the two runs of rounds
one and two it was (margin 0.233, difference 0.26); the third run (1.062) moved
the pooled verdict, which is what a row this noisy does.** On sway, with one
run each, #367 (0.83) and the experiment (0.94) flag against `main`'s 0.68 and
read same against M3 post-fix's 0.89: `main` itself differs from the baseline
by 0.21 ms there. A per-run flag at this row is a 0.1 ms edge on a single sample.

**What the stack costs, by row.**

- **Size, every layer.** The closure goes up by 131,072, 65,568 and 65,536
  bytes: the layers are +9.5%, +4.3% and +4.1% of the gated row, +19.0% in
  all. The file grows in **steps of about 64 KiB**: the segments are aligned
  to 64 KiB (`readelf -l`: `Align 0x10000`) and the writable one starts at the
  next boundary after the executable one, so a layer's step is its real
  growth rounded across boundaries. The loaded code and data grew by 63,144,
  104,528 and 30,448 bytes (+5.4%, +8.5%, +2.3%; +17.1% in all), of which
  `.text` is 53,760, 95,264 and 25,504 bytes. #364's +131,072 is a 63 KB
  growth that fell across a boundary; #366's 104 KB and #367's 30 KB both read
  as about 65 KB.
- **Idle memory**: RSS, PSS and peak are +0.13 to +0.20 MiB at every layer
  (3.53 to 3.67/3.73/3.67 MiB RSS; PSS 2.06 to 2.20/2.26/2.20), and `RssAnon` one
  to two 16 KiB pages. `compare` flags different rows in different runs because the
  margin (5% of the median, 0.18 MiB for RSS) sits inside the effect; the effect
  itself is the same every run (all five runs of #364 to #367 are 0.13 to 0.20
  MiB above `main`'s three; on sway, #367 is 0.11 above `main`). It tracks the
  growth of the executable's mapping (the dev VM's `smaps` for #364 put all of it
  there; not repeated here).
- **Unchanged**: idle wakeups (2 a minute at every layer), threads (1), the
  wakeups and CPU of 240 workspace switches (0.2 ms, as M3 post-fix), and
  startup (a median 29 to 39 ms in every run, inside the rule's margin of 20 to
  38 ms).

**Experiment, not shipped: `opt-level = "s"` for scootbar alone.** The PR 1
review reported that this shrank scootbar's R-E segment by 13% and made the
stripped binary smaller than `main`'s. **The first part reproduced; the second
did not**: the stripped bare executable here is 1,315,552 B against `main`'s
1,249,984 (+65,568, +5.2%); only **`.text`** comes in below `main`'s (927,720
against 930,792 B, -0.3%), and 16.1% below #367's (1,105,320). The change, on
top of #367 (it was a local commit, `854f3394`, on the Asahi box, in no PR and
since removed there; the diff below is all there was):

```toml
# EXPERIMENT (not for the stack): scootbar alone at opt-level "s".
[profile.release.package.scootbar]
opt-level = "s"
strip = true
```

On this machine: the bare executable 1,512,160 to 1,315,552 B (-13.0%), the
closure to 1,448,648 B (+4.7% over `main`, inside the row's margin, so `compare`
against M3 post-fix does not flag it), loaded code and data +1.4% over `main`
(against +17.1%), idle RSS/PSS 3.59/2.12 MiB (+0.06 MiB over `main`, inside the
margin), startup, wakeups and the switching CPU unchanged. **Idle CPU is the
open question**: its three runs [1.086, 1.302, 1.062] ms against `main`'s
[0.926, 0.943, 0.951] are **the same by the pooled verdict** (margin 0.265), as
they are against #367's pooled; the 1.302 run also flagged against the
experiment's own first run (exit 1), and with only the first two runs the
pooled verdict had said regressed. The median is +0.14 ms (about 15%) in a
300 s window; whether that is a cost or noise needs more runs than three. What this
did **not** measure: the cost of a full redraw or of text-heavy frames at
`opt-level = "s"`, which no row of the harness exercises. Whether to take 197 KB
off the binary for a possible ~0.14 ms of idle CPU is the maintainer's call.

**Could the three table shapes share code?** An estimate, from `nm --size-sort`
of unstripped release builds of #364 and #366: of the +106 KB of new
`.text` and `.rodata` symbols, **53 KB are the `toml` deserialization of
`BTreeMap<String, ButtonFile>`, `PushFile` and `ExecFile`** (18.3, 16.5 and 18.4 KB:
`TableMapAccess::next_entry_seed`, `ValueDeserializer::deserialize_any` and the
`MapVisitor`, each monomorphized once per shape); the modules, the payload
and `Command` are the rest. Deserializing the three tables into one
`BTreeMap<String, CustomFile>` (a struct with the union of the keys, the kind
taken from the table's name, the per-kind unknown-key refusal done by hand)
would keep one instance and drop two: **about 33 to 36 KB of `.text`
(3% of it), net of the hand-written checks.** Not implemented: it rewrites
`config/custom.rs`'s error reporting (each kind refuses its own unknown keys
by dotted name today, through `deny_unknown_fields`) and is not clearly
small or safe.

**Noise, measured.** Each build against its own other runs (scoot, the
compositor all runs share): `main` exits 0 and 0, #367 exits 0 and 0, and the
experiment exits 1 (its second run against its first: idle CPU 1.09 to 1.30 ms)
and 0. A single run against another build's single run is a different matter:
in round three #367 against `main` flags idle PSS and idle CPU on both
compositors (scoot 0.95 to 1.10 ms, sway 0.68 to 0.83), which the pooled scoot
verdict above does not call a regression. The idle CPU and idle heap rows are the
noisy ones on this box, as at M3.

**What the machine did** (readings around the first eight runs, from `meta.json`
and `runs.jsonl`; the third round's meta records the same kind): governor
`schedutil` throughout; no cpufreq policy cap below the hardware maximum in any
reading; mains online in every one; the battery sensors' hwmon temperatures 22 to
29 C (the M2 exposes no CPU temperature, so a throttle could only show as a
cap, and none did); the current frequency 600 to 3204 MHz, which is the
governor.

**What this does not settle.** It is one machine and one scope (the clock,
scootbar alone), and sway was run for three builds, once each: `exec`, `button`,
`push` and the agent interface cost what a build that configures none of them
costs here, which is what the default config does; what *using* them costs is
the dev VM's table in
[exec-push-button-modules-done](backlog/resolved/exec-push-button-modules-done.md#the-ratchet),
not remeasured here. Rule 2 (no competitor beats scootbar) was not rerun;
nothing in this stack touches what it compared. **The ratchet's rule 1 fails
on size and idle memory for this stack, as M3's did; the PRs are drafts.
Since: the maintainer accepted the size on 2026-10-01 (lightest's
[Decisions](backlog/lightest.md#decisions)); the idle memory rows remain
regressed and are tracked by [m4-usage-optimization](backlog/resolved/m4-usage-optimization-done.md)
(resolved since: [M4 usage optimization](#m4-usage-optimization));
nothing else is waived** ([lightest](backlog/lightest.md#m3-gate-clock-and-workspaces-measured-2026-09-30-does-not-pass)'s 2026-10-01 note; the
[final stack](#m4-final-stack-the-fix-round-re-measured) re-measured the fixed heads).

#### M4 final stack: the fix round, re-measured

The runs above measured earlier heads of the three PRs. A fix round has landed
since (subscribe `dropped` and eviction, the fake-daemon guard in the tests,
and the review-driven changes: #364 `3ccd1c35b`, #366 `8e7682f44`, #367
`d0c4f35d1`), so the stack's tip was measured again: three runs, **`main`, #367's
tip `d0c4f35d1`, `main` again** (A-B-A, so drift shows), 2026-10-01 on the same
Asahi M2, **scoot and sway** in every run, the harness's defaults (5 startup
rounds, 30 s settle, 300 s idle, 240 switches at 4 Hz), `--scope clock --bars
scootbar`, 180 s of cool-down between runs, each run about 13 minutes. Raw runs:
[`m4-asahi-final-main`](bench/m4-asahi-final-main/table.md),
[`-pr367`](bench/m4-asahi-final-pr367/table.md),
[`-main-rerun`](bench/m4-asahi-final-main-rerun/table.md); every `compare` and the
pooled verdicts, as run, in
[`bench/m4-asahi-final-compares.md`](bench/m4-asahi-final-compares.md).

**What differs from the earlier M4 runs, stated plainly:**

- **`main` moved**: it is now `98c4b7a32`, which carries #370 (`scootbar daemon
  --check`, 251 added lines in `crates/scootbar`) that the #367 tip (merge base
  `7a1f9030a`) does not. The comparison is #367's tip against a `main` that
  has a little more code than its own base; the size row (1,383,080 B) and the
  bare executable (1,249,984 B) are byte-identical to the earlier `main`'s, the
  Rust line count is not (25,643 against 25,550).
- **The pinned `scoot` is a different binary** (sha256 `25e6a225a07d...` against the
  earlier runs' `4e105065f5b2...`), rebuilt from `main` `98c4b7a32`; `crates/scoot`
  is unchanged between the two mains, but every comparison with
  `m3-asahi-clock-bindfix` or an earlier `m4-*` run crosses a compositor
  rebuild. The earlier build directory was gone, so the old binary could not
  be reused.
- **The first run began warm**: `main`'s first run started at load average
  0.44/0.61/0.30 (from a smoke run and the builds just before it); the other
  two started at 0.00 to 0.12. The A-B-A rerun is what makes it
  interpretable. The run and the builds shared the box with nothing else (no
  `scoot`, `sway`, `foot` or bar process, no bench cgroup, before the runs).
- **#367 has one run**, so its pooled verdict is one sample against two `main`
  runs; the earlier #367 runs (`b6dee9710`) are not pooled in: the code
  differs.
- The harness is the same (`bars.py`, `bench.py`, `machine.py`, `measure.py`,
  `stage.py`, `tables.py` sha256-identical to the earlier runs'), and the
  harness ran under the dev shell's tools with a `python3` 3.14.7 from the Nix
  store (`/nix/store/3fl7bdkdxk2k4nssy1d1161isbzn6bsr-python3-3.14.7`), the box having no
  `python3` on its `PATH`.

Medians of each run (scoot; the earlier columns are copied from the table
above, M3 post-fix and the earlier M4 round three):

| Row, scoot | M3 post-fix | earlier M4 `main` (round three) | final `main` | final `main` rerun | earlier #367 (round three) | final #367 `d0c4f35` |
|---|---|---|---|---|---|---|
| Startup to first frame (ms) | 31.0 | 29.8 | 31.8 | 29.4 | 38.1 | 29.0 |
| Idle RSS (MiB) | 3.52 | 3.53 | 3.52 | 3.53 | 3.67 | 3.64 |
| Idle PSS (MiB) | 2.12 | 2.06 | 2.11 | 2.13 | 2.20 | 2.24 |
| Idle heap, `RssAnon` (MiB) | 0.42 | 0.44 | 0.42 | 0.44 | 0.45 | 0.42 |
| Peak memory, `VmHWM` (MiB) | 3.52 | 3.53 | 3.52 | 3.53 | 3.67 | 3.64 |
| Idle wakeups per minute | 2 | 2 | 2 | 2 | 2 | 2 |
| Idle CPU, 300 s window (ms) | 1.04 | 0.95 | 0.99 | 0.97 | 1.10 | 0.91 |
| CPU, 240 workspace switches (ms) | 0.25 | 0.25 | 0.23 | 0.20 | 0.21 | 0.25 |
| Size: binary + non-glibc `ldd` closure (B) | 1,383,080 | 1,383,080 | 1,383,080 | 1,383,080 | 1,645,256 | **1,645,256** |
| Bare executable, stripped (B) | 1,249,984 | 1,249,984 | 1,249,984 | 1,249,984 | 1,512,160 | 1,512,160 |

Sway (idle CPU in ms: M3 post-fix 0.89; earlier `main` 0.68, earlier #367
0.83; final `main` 0.83 and 0.63 on the rerun, final #367 0.76): idle RSS 3.50,
3.52, 3.64 (final `main`, rerun, #367), PSS 2.15, 2.16, 2.29, heap 0.42,
0.44, 0.44, wakeups 2 in all (the rerun's sway median is 1.8).

**What the final stack costs against `main`, and what it does not:**

- **Regresses**, by `compare` and by the pooled verdict, on both compositors,
  against either `main` run: the **size row** (+262,176 B, +19.0%, the same
  bytes as the earlier run) and **idle PSS** (2.11 and 2.13 to 2.24 on scoot,
  2.15 and 2.16 to 2.29 on sway). `compare` of the final #367 against
  M3 post-fix exits **1**, against the final `main` exits **1** (three flags:
  size and idle PSS on both compositors) and **1** against the rerun (four:
  the same three, and sway's idle CPU, 0.6 to 0.8 ms, a single-sample row
  where the two `main` runs themselves differ by 0.2 ms).
- **Same** on both: startup, idle RSS (+0.11 to 0.13 MiB, inside the 0.18 MiB
  margin this time), idle heap (0.42 MiB, equal to `main`'s first run), peak
  memory, idle wakeups, the CPU and wakeups of 240 switches, threads. Idle CPU
  on scoot is 0.91 ms against `main`'s 0.99 and 0.97 (and 0.93 to 0.95
  earlier): **not worse**. The fix round does not change the idle numbers:
  the final #367 against the earlier #367 (`b6dee9710`) exits **0**, and the
  final `main` against the earlier `main` exits 0 on its rerun and 1 on its
  first run, the one flag being sway's idle CPU (0.7 to 0.8 ms) in a run that
  started warm; the rerun, against the first, exits 0 (the A-B-A drift:
  scoot idle CPU 0.99 to 0.97 ms, sway 0.83 to 0.63 ms, everything else
  within the margin).
- **The size is accepted** by the maintainer (2026-10-01: "Keep the button
  and exec inside the bar. Accept it. Aim for optimization in usage more than
  pure disk space."; [lightest's Decisions](backlog/lightest.md#decisions)).
  **The memory rows are not waived**: idle PSS is still +0.1 MiB over
  `main` on every compositor, and the whole class is tracked by
  [m4-usage-optimization](backlog/resolved/m4-usage-optimization-done.md) (resolved since:
  [M4 usage optimization](#m4-usage-optimization)). Nothing else is
  waived.

**What could not be settled here:** the PSS step's cause at the fixed tip was not
re-derived (the PR 1 review's `smaps` finding is for #364); #364 and #366 were not
re-measured at their fixed heads, only the stack's tip; and idle CPU, one
sample against two, cannot rule a 0.1 ms cost in or out.

### M4 usage optimization

[m4-usage-optimization](backlog/resolved/m4-usage-optimization-done.md): the maintainer's ruling
of 2026-10-01 ("Aim for optimization in usage more than pure disk space") sent
the effort at what the bar holds and wakes for, with `button`, `push`, `exec`,
the pointer layer and the agent interface still in the default build. This
section is what was measured, the one change that moved the rows, and every
lever that was tried and dropped. The machine is the one of every run above (the
Asahi M2: 8 cores, **16 KiB pages**, Linux 7.1.13, `schedutil`, on mains), release
scootbar (`lto = "fat"`, `codegen-units = 1`, `panic = "abort"`, stripped) built
per tree in its own target directory, **one headless release `scoot`/`scootctl`**
(sha256 `33aa667c...`, built from `main` at `c9d2cd361`) for every run, nothing
else on the box. The tools were `smaps`, `smaps_rollup`, `pagemap` and `stat` of the
running bar, `valgrind` (callgrind), `qemu-user` (`-d in_asm`) and `lld`: the
last three fetched into the Nix store for this, and nothing else. The raw files
are in [`bench/m4-usage-attribution`](bench/m4-usage-attribution/README.md).

#### Where the memory went

The PR 1 review's finding (the `r-xp` mapping, 960 kB on `main` against 1088 kB at
#364) holds at the tip, and it is **all of it**. `main` before M4 (`98c4b7a32`)
against the tip (`c9d2cd361`, which carries #364, #366 and #367), three starts
each, 20 s after the first frame (the numbers did not differ between starts
except the `Pss_Shmem` of the shm buffer, 104 or 208 kB by chance):

| | `main` | tip |
|---|---|---|
| `scootbar` `r-xp` (headers, `.rela.dyn`, `.text`, `.rodata`, `.eh_frame`): size / `Rss` | 1184 / **960** kB | 1392 / **1152** kB |
| `scootbar` `r--p` + `rw-p` (relro, `.data`): `Rss` | 64 kB, all private dirty | 64 kB, the same |
| `Pss_Anon` (heap, stack, the glyph cache) | 448 to 464 kB | 448 to 464 kB |
| the font, libc, libm, libgcc_s, ld.so | the same | the same |
| minor faults at the first frame | 154 to 155 | 157 to 158 |

So there is **no new anonymous memory** (the `RssAnon` 0.42 to 0.45 MiB of every run
is one or two 16 KiB pages of noise), **no new faults** (three more), and nothing
in the other mappings: **the idle bar's memory step is file pages of its own
executable.** It does not run much more code: callgrind counts **231
functions and 244 KB** executed by `main`'s bar from its start through a minute
of idle, and **232 and 257 KB** at the tip (qemu on the shipped binary: 235 and
261 KB). Only 11 functions are in the tip's list and not in `main`'s, three of them
8-byte trait stubs (`Exec`'s `custom_draw` and `on_dispatch`, `tints_on_hover`), the rest
`pointer_x`, `note_output`, `modules::start` and a few `std` functions; none is from `config::custom`,
`exec`, the payload or the agent. **The unconfigured modules' code is not run, and
nothing is built or deserialized for them** (the harness gives the bar flags and no
config file at all; the `Option<toml::Value>` fields are parse-time temporaries,
and the heap is the same as `main`'s to the page).

**Why 257 KB of code takes 1152 kB.** On a read fault of a file-backed page the
kernel maps the cached neighbours too, up to `fault_around_bytes` (64 KiB: four of
these pages), aligned. rustc lays a crate's functions out in definition order and
the generics other crates instantiate in name order, so the functions a bar runs
are spread over the whole `.text`: **14 of its 17 64 KiB windows hold at least one**
(`pagemap`: every page resident but three windows), and the kernel maps the lot.
The tip's +192 kB is the +175 KB of new text, which brought its own windows. It
follows that the **resident text is about the size of the text, however little of
it runs**, that a cold function moved from one place to another moves nothing
unless it empties a window, and that what shrinks the number is the hot functions
being *together*.

#### What was tried

Each build measured the same way: `smaps` 15 to 20 s after the first frame (so the
`r-xp` `Rss` is exact and repeats to the page), three starts each; the redraw column
is [`redraw.py`](testing.md#redraw-cost), 3000 `set`s a round, five rounds, run twice in the
order tip, `opt-level = "s"`, `opt-level = 2`, this PR's, tip, and so on (raw files
in [`m4-usage-attribution`](bench/m4-usage-attribution/levers.txt)):

| Lever | `.text` (B) | `r-xp` `Rss` (kB) | `Pss_File` (kB) | redraw CPU per set (us), two runs | Kept |
|---|---|---|---|---|---|
| `main` before M4, for reference | 932,584 | 960 | 1655 | n/a (no `push`) | |
| the tip, as merged | 1,107,848 | 1152 | 1843 to 1851 | 82.6, 81.4 | |
| (a) `opt-level = "s"` for scootbar | 929,160 | 1024 | 1648 to 1654 | **107.8, 107.1 (+31%)** | **no**: per-event CPU |
| (a) `opt-level = 2` for scootbar | 1,054,984 | 1088 | 1716 | **96.9, 96.0 (+18%)** | **no**: per-event CPU |
| (b) `opt-level = "s"` for `toml*`, `serde*` alone | 1,092,328 | 1216 | 1844 to 1848 | not run | no: no gain |
| (b) `codegen-units = 16` for scootbar | 1,154,856 | 1216 | 1907 to 1915 | not run | no: worse |
| (b) `--sort-section=name` (linker) | n/a | the hot functions in 15 64 KiB windows, were 14 | | | no |
| gold, `--section-ordering-file`, exact names | | 576 | 1288 to 1291 | | no: gold is deprecated, `rustc` warns of known bugs with Rust |
| lld, `--symbol-ordering-file`, exact names | | 320 to 576 | 1240 to 1491 | | no: needs lld, and the names do not carry (below) |
| GNU ld, `--section-ordering-file`, exact names | | 576 to 640 | 1204 to 1264 | | no: the names do not carry (below) |
| **GNU ld, the same as globs: the order file** | 1,107,848 (the same bytes, in another order) | **640** | **1223** | **82.9, 85.4** | **yes** |

(The gold and lld rows are of the build valgrind can run, rustix on its libc
backend: indicative, not the shipped binary; the others are the shipped one.)

What each lever of the ticket came to:

- **(a) `opt-level = "s"`**: smaller (`.text` -16%) and 128 kB less resident, **and
  31% more CPU per redraw** (`opt-level = 2`: 18%), where the harness's rows
  could never have shown it. A regression in per-event CPU is not acceptable for
  memory. Dropped, with its numbers.
- **(b) cold-path outlining.** `#[cold]` puts a function in `.text.unlikely.*`, which
  the linker keeps apart, and that is all it does: the callees (the 62 KB of
  monomorphized `toml` deserializers `config::custom` instantiates) stay where
  they are, and a window empties only when *everything* in it is cold. Not done.
  **Sharing the three table shapes** (33 to 36 KB of text): that code is not run at
  idle, so with the order file below it costs no page, and without it the
  resident text follows the text size, at most those 35 KB out of 1.1 MB (0.03
  MiB, inside every margin) for a rewrite of the per-kind unknown-key errors.
  Not implemented. Linker ordering is what worked, below.
- **(c) Per-module state, buffers, retained config, leaked strings.** The heap is
  the same as `main`'s to the page (`Pss_Anon` 448 kB), nothing is built for an
  unconfigured module, and nothing is held after the parse; there was nothing
  to make lazy.
- **(d) `madvise`/`munmap` of start-up data.** `rustix`'s `madvise` is an `unsafe fn` and
  the crate is `#![forbid(unsafe_code)]`: not done, no dependency added.
- **(e)** the PIE relocations and the relro data, measured and **not taken**, each on
  top of the order file (against the same file's `Pss`, quick runs): `-C
  relocation-model=static` (no `.rela.dyn` to read, `.data.rel.ro` read only)
  **-0.09 MiB** and it gives up the bar's ASLR; `-Wl,-z,pack-relative-relocs`
  (DT_RELR; `.rela.dyn` 57 KB to 8 KB) **-0.10 to -0.13 MiB**, and the binary then
  refuses to start on a glibc before 2.36. Neither is for an optimization PR to
  decide; each is one `RUSTFLAGS` entry.

#### The order file

`crates/scootbar/orderfile/hot-text.ld` is a GNU ld `--section-ordering-file`: a
mini linker script that puts the listed input sections first in `.text`, in
order. It lists 449 patterns: the 240 functions an idle bar runs first (one 260
KB stretch, five windows), then what the workspaces, the pointer, `msg`, `push`
and a config file run, 520 KB in all. The binary is the same size and the same
code in another order: **`r-xp` `Rss` 1152 kB to 640 kB, idle PSS 2.25 to 1.8
MiB, idle RSS 3.72 to 3.28**, and the redraw row is 82.9 and 85.4 against the tip's
82.6 and 81.4 us (the rounds of one run span 77 to 85: the same). Under a
workload that touches more (the redraw run: workspaces, a `push` module, the
control socket) the bar's RSS is 3.50 MiB against the tip's 3.81.

**How it is made, and why it is not a plain symbol list.**
`scripts/scootbar-orderfile/orderfile.py gen` runs the bar under `qemu-user`
(`-d in_asm` logs each translation block with its function's name, the first time
it executes) in four scenarios, the bench's idle clock first, and merges the
functions in the order they first ran. **The names do not carry between
builds**: a v0-mangled Rust symbol embeds a hash per crate and `B<n>_`
back-references that move with those hashes' lengths. Measured on one source,
**182 of the 235 hot names do not exist in the Nix package's build** (every
cargo-built crate's hash differs; only `std`, `core` and `alloc`'s agree), while two
`cargo` builds of one source in two directories are byte-identical. A list of
exact names (lld's `--symbol-ordering-file` takes no glob) would work for the build
it was made on and, silently, for no other, the Nix package included. So each
symbol becomes a **glob** with those two tokens as `*` (`glob()`, unit-tested to
match both builds' names of one function and not another's), which GNU ld's file
takes, and `check` reports **449 of 449 patterns matching a symbol in a `cargo`
build and 446 of 449 in the Nix package's**, whose shipped binary (`nix build
.#scootbar`) has `r-xp` `Rss` 576 kB against 1152 kB without the file, and
`nix build .#checks.aarch64-linux.scootbar-modules` passes.

**It is an optimization, never a requirement.** `build.rs` passes the flag only
after a trial link of an empty program with this file has worked through the
linker `rustc` will use, with the link arguments `RUSTFLAGS` add (so binutils before
2.43, lld, mold and gold link as they always did), and stays out of the way on an
unstable `-Z` flag or `link-self-contained`, on a cross build, off Linux, and with
`SCOOTBAR_NO_ORDERFILE` set. It adds a build script to a crate that had none: it
compiles nothing and runs `cc` once.

**What it does not reach.** `r-xp` is 640 kB, not the 576 the idle set alone
would give, and `rodata` (hot strings, anonymous sections) is not ordered. The
unlisted functions that run only sometimes (which `Vec` happens to grow, an event
that arrives in two reads) each cost a window when they do; the tool lists whole
families of the likeliest (`grow_one`, `smallvec`...) and `check --profile` says what
an idle bar ran that no pattern lists (nothing, in the build it was made from).

**It decays, gracefully.** A function that is renamed, or inlined differently after a
change, is no longer listed and goes wherever the linker puts it: the file never
breaks a build and never changes what the bar does; it stops saving memory for that
function. The row that would show it is idle PSS; CI's `check` step fails when
under 80% of the patterns match a symbol of the release build (not when functions
are added), and regenerating it is [a command](testing.md#the-hot-text-order-file).

#### The ratchet, against `main` before M4

The harness's own run (`bench.py run`, scope clock, the defaults: 5 startup
rounds, 30 s settle, a 300 s idle window, 240 switches at 4 Hz), **scoot and sway in
every run**, seven runs in the order `main`, this PR's build, the stack as merged, this PR's,
`main`, this PR's, `main` (A-B-C-B-A-B-A), 180 s of cool-down between them, so that
drift shows and each build's runs are neighbours of `main`'s. Raw runs:
[`m4-usage-*`](bench/README.md); every `compare` as it ran and the pooled
verdicts, [`m4-usage-compares.md`](bench/m4-usage-compares.md). Medians of each
run, a build's runs side by side in the order they ran:

| Row, scoot | `main` before M4 (3 runs) | the stack as merged | this PR (3 runs) | pooled verdict, this PR against `main` |
|---|---|---|---|---|
| Startup to first frame (ms) | 40.8 / 31.4 / 30.3 | 37.7 | 38.5 / 36.0 / 33.4 | same (margin 37.7) |
| Idle RSS (MiB) | 3.53 / 3.50 / 3.52 | 3.72 | 3.28 / 3.27 / 3.28 | better (margin 0.176) |
| Idle PSS (MiB) | 2.06 / 2.04 / 2.04 | 2.25 | 1.81 / 1.79 / 1.81 | better (margin 0.102) |
| Idle heap, `RssAnon` (MiB) | 0.44 / 0.42 / 0.42 | 0.44 | 0.44 / 0.42 / 0.44 | same (margin 0.0312) |
| Peak memory, `VmHWM` (MiB) | 3.53 / 3.50 / 3.52 | 3.72 | 3.28 / 3.27 / 3.28 | better (margin 0.176) |
| Idle wakeups per minute | 2 / 2 / 2 | 2 | 2 / 2 / 2 | same (margin 1) |
| Idle CPU, 300 s window (ms) | 0.91 / 0.93 / 0.95 | 0.95 | 0.96 / 0.88 / 0.96 | same (margin 0.118) |
| CPU, 240 workspace switches (ms) | 0.26 / 0.24 / 0.27 | 0.21 | 0.22 / 0.22 / 0.23 | same (margin 0.1) |
| Size: binary + non-glibc `ldd` closure (B) | 1,383,080 | 1,645,256 | 1,645,256 | REGRESSED (accepted, 2026-10-01) |

| Row, sway | `main` before M4 (3 runs) | the stack as merged | this PR (3 runs) | pooled verdict, this PR against `main` |
|---|---|---|---|---|
| Startup to first frame (ms) | 17.4 / 17.6 / 16.9 | 8.9 | 16.7 / 16.6 / 17.1 | same (margin 39.8) |
| Idle RSS (MiB) | 3.52 / 3.52 / 3.53 | 3.69 | 3.34 / 3.34 / 3.33 | same (margin 0.176) |
| Idle PSS (MiB) | 2.08 / 2.08 / 2.09 | 2.26 | 1.91 / 1.91 / 1.89 | better (margin 0.104) |
| Idle heap, `RssAnon` (MiB) | 0.42 / 0.42 / 0.44 | 0.42 | 0.44 / 0.44 / 0.42 | same (margin 0.0312) |
| Peak memory, `VmHWM` (MiB) | 3.52 / 3.52 / 3.53 | 3.69 | 3.34 / 3.34 / 3.33 | same (margin 0.176) |
| Idle wakeups per minute | 2 / 2 / 2 | 2 | 2 / 2 / 2 | same (margin 1) |
| Idle CPU, 300 s window (ms) | 0.85 / 0.67 / 0.65 | 0.82 | 0.80 / 0.65 / 0.73 | same (margin 0.355) |
| CPU, 240 workspace switches (ms) | 0.23 / 0.23 / 0.22 | 0.15 | 0.18 / 0.17 / 0.23 | same (margin 0.1) |
| Size: binary + non-glibc `ldd` closure (B) | 1,383,080 | 1,645,256 | 1,645,256 | REGRESSED (accepted, 2026-10-01) |

- **What moved**: idle RSS 3.50 to 3.53 against **3.27 to 3.28 MiB**, idle PSS 2.04 to
  2.06 against **1.79 to 1.81** (scoot; sway 2.08 to 2.09 against 1.89 to 1.91), peak
  memory with them. Pooled, scoot's idle RSS, PSS and peak are **better** than `main`'s,
  and sway's PSS is (RSS and peak there: the same). The stack as merged is what the
  ticket was about: 3.72 and 2.25 on scoot, regressed by `compare` against every
  `main` run (RSS, PSS and peak on scoot, PSS on sway).
- **What did not move**: idle heap (0.42 to 0.44 MiB, one page either way), idle wakeups (2
  a minute in every run, on both compositors), threads (1), startup, and the CPU of 240
  switches (0.22 against `main`'s 0.24 to 0.27 ms). **Idle CPU**: scoot 0.91, 0.93, 0.95
  for `main` and 0.96, 0.88, 0.96 for this PR, pooled **same** (margin 0.118); sway 0.85,
  0.67, 0.65 and 0.80, 0.65, 0.73, pooled **same** (margin 0.355).
- **`compare`, pairing by pairing** (nine of this PR's runs against `main`'s,
  [all in the compares file](bench/m4-usage-compares.md)): every one exits 1 on the
  **size row** alone, which was 1,383,080 B and is 1,645,256 B with this PR as with the
  stack (+19.0%, accepted by the maintainer on 2026-10-01, not waived by this PR);
  two of them, against `main`'s second and third runs, also flag **sway's idle CPU**
  (0.7 ms against 0.8), a row on which `main`'s own three runs span 0.2 ms and which the
  pooled verdict calls the same. Against the stack as merged, all three exit 0 with
  idle RSS, PSS and peak **better** on both compositors.

**The workspaces scope** (`--scope clock-workspaces`, scoot only, one run of each build
against `main`'s two): idle RSS 3.28, PSS 1.77 for this PR against `main`'s 3.52 to
3.53 and 2.02, and the stack's 3.72 and 2.22. The CPU of 240 workspace switches, which
at this scope redraws the workspaces module each time (about 27 ms, not the 0.2 of the clock
scope), is a finding the clock scope could not show: **the stack as merged costs 29.0 ms
against `main`'s 27.3 and 27.8 (+5%, margin 1.4: regressed), and this PR's build 27.6,
`main`'s.** Idle CPU there: 0.84 and 0.91 for `main`, 0.95 for this PR and the
stack; the one `compare` that flags it (this PR against `main`'s first run, 0.8 against 1.0 ms,
the 0.1 ms floor) does not against its rerun, and the pooled verdict is the same.

**Is the ticket's bar met?** The acceptance bar was no row worse than `main`'s by more
than the harness's noise margin on idle RSS, PSS, heap and idle CPU at scope clock, on scoot
and sway, a pooled `verdict()` over at least three runs for the CPU row, `main`
re-run beside it. **Met, with a margin the other way** (RSS and PSS better, the rest the same),
for the build in this PR; wakeups stay at 2 a minute; no other row of the ratchet
regresses. **The size row regressed against `main` before M4 and stays so**: the
maintainer accepted that on 2026-10-01 (lightest's Decisions) and the ticket's own
"Not in this ticket" says so; this PR neither moved nor waived it.

**What this does not settle.** The order file was profiled and measured on
aarch64 only; x86_64 builds link with it (CI builds and `check`s it there) and
should save memory by the same mechanism, but that is unmeasured. The workspaces
scope was measured on scoot only, one run of each build.
