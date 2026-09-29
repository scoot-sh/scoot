# scootbar

A status bar for scoot, and for any compositor with `wlr-layer-shell-v1`:
the lightest bar that is still beautiful and configurable. It starts as a
clock, gains workspaces, and grows by modules, one small daemon of a
composable shell.

> **Status: not built yet.** M0, the measuring milestone, is done: the
> competitor baselines below, and the two choices the first milestone
> needs (the font rasterizer and the clock), recorded with their numbers in
> [the dependency record](backlog/resolved/dependencies-done.md). M1, a
> clock you can `nix run`, is next. The plan, milestone by milestone, is
> the [backlog](backlog/README.md).

## What it is for

- **The lowest resource use of any bar at the same scope**, checked at
  every milestone against the last one and against the bars below, in the
  [resource ratchet](backlog/lightest.md). A loss is a finding to fix.
- **Event-driven, no polling**: one `poll(2)` loop, and idle wakeups are a
  measured row. A minute clock should wake once a minute; M0's spike clock
  does ([measured](backlog/resolved/dependencies-done.md#2a-idle-one-wakeup-per-minute)).
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
noisy and memory as close to exact. scootbar's own column arrives with M1.

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
- One idle and one switching run per bar and compositor; startup is the
  median of 5.

Not measured: **i3status-rust** (a status-line generator that needs
`swaybar`, so sway only and not a like-for-like bar; see the record), any
real hardware, a third compositor, and the competitors' lines of code and
dependency counts ([lightest](backlog/lightest.md) wants those rows for
scootbar itself). yambar, Waybar and ironbar are
MIT; ashell and i3status-rust are GPL, and were only run as binaries.
