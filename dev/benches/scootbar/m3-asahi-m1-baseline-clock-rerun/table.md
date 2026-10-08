**On scoot** (scoot 0.1.0 (ipc protocol 5))

| Row | scootbar |
|---|---|
| Size: stripped binary + non-glibc `ldd` closure (bytes) | 924,320 |
| Bare executable, stripped (bytes) *(not gated)* | 791,224 |
| Startup to first frame (ms) | 30.7 [27.4–39.4] |
| Idle RSS (MiB) | 2.9 |
| Idle PSS (MiB) | 1.5 |
| Idle heap (`RssAnon`) (MiB) | 0.3 |
| Peak memory (`VmHWM`) (MiB) | 2.9 |
| Idle wakeups per minute | 2 |
| Idle CPU in the window (ms) | 0.8 |
| CPU while switching workspaces (ms) | 0.2 |
| Wakeups while switching workspaces *(not gated)* | 2 |
| Threads *(not gated)* | 1 |

**On sway** (sway version 1.12)

| Row | scootbar |
|---|---|
| Size: stripped binary + non-glibc `ldd` closure (bytes) | 924,320 |
| Bare executable, stripped (bytes) *(not gated)* | 791,224 |
| Startup to first frame (ms) | 16.9 [7.5–18.1] |
| Idle RSS (MiB) | 2.9 |
| Idle PSS (MiB) | 1.5 |
| Idle heap (`RssAnon`) (MiB) | 0.3 |
| Peak memory (`VmHWM`) (MiB) | 2.9 |
| Idle wakeups per minute | 2 |
| Idle CPU in the window (ms) | 0.6 |
| CPU while switching workspaces (ms) | 0.2 |
| Wakeups while switching workspaces *(not gated)* | 2 |
| Threads *(not gated)* | 1 |

Machine, 24 readings around the runs: governor schedutil; cpufreq policy cap below the hardware maximum in 0; current frequency seen 600 to 3204 MHz; a mains supply offline in 0 of them; hwmon temperatures 22 to 26 C.

scootbar's code: 10,628 lines of Rust in `crates/scootbar/src`, 6,995 outside `tests.rs` files; 6 direct dependencies on Linux (ab_glyph, rustix, scootbg-mem, wayland-client, wayland-protocols, wayland-protocols-wlr).

Gate (no competitor beats scootbar): 0 loss(es).
