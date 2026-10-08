**On scoot** (scoot 0.1.0 (ipc protocol 5))

| Row | scootbar |
|---|---|
| Size: stripped binary + non-glibc `ldd` closure (bytes) | 1,645,256 |
| Bare executable, stripped (bytes) *(not gated)* | 1,512,160 |
| Startup to first frame (ms) | 38.1 [29.5–48.1] |
| Idle RSS (MiB) | 3.7 |
| Idle PSS (MiB) | 2.2 |
| Idle heap (`RssAnon`) (MiB) | 0.5 |
| Peak memory (`VmHWM`) (MiB) | 3.7 |
| Idle wakeups per minute | 2 |
| Idle CPU in the window (ms) | 1.1 |
| CPU while switching workspaces (ms) | 0.2 |
| Wakeups while switching workspaces *(not gated)* | 2 |
| Threads *(not gated)* | 1 |

**On sway** (sway version 1.12)

| Row | scootbar |
|---|---|
| Size: stripped binary + non-glibc `ldd` closure (bytes) | 1,645,256 |
| Bare executable, stripped (bytes) *(not gated)* | 1,512,160 |
| Startup to first frame (ms) | 17.6 [9.3–18.0] |
| Idle RSS (MiB) | 3.7 |
| Idle PSS (MiB) | 2.2 |
| Idle heap (`RssAnon`) (MiB) | 0.4 |
| Peak memory (`VmHWM`) (MiB) | 3.7 |
| Idle wakeups per minute | 2 |
| Idle CPU in the window (ms) | 0.8 |
| CPU while switching workspaces (ms) | 0.2 |
| Wakeups while switching workspaces *(not gated)* | 2 |
| Threads *(not gated)* | 1 |

Machine, 24 readings around the runs: governor schedutil; cpufreq policy cap below the hardware maximum in 0; current frequency seen 600 to 3204 MHz; a mains supply offline in 0 of them; hwmon temperatures 22 to 26 C.

scootbar's code: 34,909 lines of Rust in `crates/scootbar/src`, 22,779 outside `tests.rs` files; 10 direct dependencies on Linux (ab_glyph, png, rustix, scootbg-mem, serde, serde_json, toml, wayland-client, wayland-protocols, wayland-protocols-wlr).

Gate (no competitor beats scootbar): 0 loss(es).
