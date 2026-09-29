# scootbar

A status bar for scoot, and for any compositor with `wlr-layer-shell-v1`:
the lightest bar that is still beautiful and configurable. It starts as a
clock, gains workspaces, and grows by modules, one small daemon of a
composable shell.

> **Status: early.** `scootbar daemon` puts a bar with a clock on every
> output, reserving its space, across outputs coming and going; the flags
> are in [cli.md](cli.md). That is M1's
> [skeleton](backlog/resolved/skeleton-layer-surface-done.md) and its
> [module API and clock](backlog/resolved/module-api-and-clock-done.md),
> and its [Nix package](backlog/resolved/nix-package-done.md), which
> ships M1's `nix run`: a clock you can run in one command. M1 closes
> with its testing and CI ticket. M0, the
> measuring milestone, is done: the competitor baselines below, and the
> font rasterizer and clock choices, recorded in
> [the dependency record](backlog/resolved/dependencies-done.md). The
> plan, milestone by milestone, is the [backlog](backlog/README.md).

```sh
scootbar daemon --font /path/to/DejaVuSans.ttf &            # 3:07 pm, centered
scootbar daemon --clock-format '%a %d %b %H:%M' --right clock &
nix run github:scoot-sh/scoot#scootbar-demo                 # with a font, from Nix
```

From Nix, `packages.<system>.scootbar` is the bar with no font in its
closure (give it `--font`), and `scootbar-demo` the same bar with DejaVu
Sans as its default font ([docs/nix.md](../nix.md#the-status-bar-scootbar)).

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
