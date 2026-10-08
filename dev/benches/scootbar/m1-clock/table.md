**On scoot** (scoot 0.1.0 (ipc protocol 4))

| Row | scootbar | yambar | waybar |
|---|---|---|---|
| Size: stripped binary + non-glibc `ldd` closure (bytes) | 1,030,976 | 19,717,088 | 71,422,752 |
| Bare executable, stripped (bytes) *(not gated)* | 848,624 | 359,224 | 3,253,144 |
| Startup to first frame (ms) | 21.4 [18.0–37.2] | 44.3 [41.3–81.1] | 178 [165–267] |
| Idle RSS (MiB) | 3.9 | 14.1 | 52.5 |
| Idle PSS (MiB) | 2.2 | 8.6 | 45.8 |
| Idle heap (`RssAnon`) (MiB) | 0.9 | 1.8 | 8.1 |
| Peak memory (`VmHWM`) (MiB) | 3.9 | 14.1 | 52.5 |
| Idle wakeups per minute | 2 | 4 | 5 |
| Idle CPU in the window (ms) | 1.6 | 3.5 | 8.2 |
| CPU while switching workspaces (ms) | 0.1 | 0.7 | 1.6 |
| Wakeups while switching workspaces *(not gated)* | 2 | 4 | 5 |
| Threads *(not gated)* | 1 | 3 | 8 |

- tie: Startup to first frame: yambar

**On sway** (sway version 1.12)

| Row | scootbar | yambar | waybar |
|---|---|---|---|
| Size: stripped binary + non-glibc `ldd` closure (bytes) | 1,030,976 | 19,717,088 | 71,422,752 |
| Bare executable, stripped (bytes) *(not gated)* | 848,624 | 359,224 | 3,253,144 |
| Startup to first frame (ms) | 24.2 [12.7–27.5] | 49.8 [31.2–77.0] | 165 [158–216] |
| Idle RSS (MiB) | 3.9 | 13.8 | 52.4 |
| Idle PSS (MiB) | 2.2 | 7.8 | 43.2 |
| Idle heap (`RssAnon`) (MiB) | 0.9 | 1.8 | 8.3 |
| Peak memory (`VmHWM`) (MiB) | 3.9 | 13.8 | 52.4 |
| Idle wakeups per minute | 1 | 3 | 3 |
| Idle CPU in the window (ms) | 1.6 | 3.4 | 7.3 |
| CPU while switching workspaces (ms) | 0.2 | 0.5 | 1.3 |
| Wakeups while switching workspaces *(not gated)* | 1 | 3 | 3 |
| Threads *(not gated)* | 1 | 3 | 8 |

- tie: Startup to first frame: yambar

scootbar's code: 10,628 lines of Rust in `crates/scootbar/src`, 6,995 outside `tests.rs` files; 6 direct dependencies on Linux (ab_glyph, rustix, scootbg-mem, wayland-client, wayland-protocols, wayland-protocols-wlr).

Gate (no competitor beats scootbar): 0 loss(es).
