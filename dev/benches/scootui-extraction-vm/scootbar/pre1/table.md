**On scoot** (scoot 0.7.3 (ipc protocol 13))

| Row | scootbar |
|---|---|
| Size: stripped binary + non-glibc `ldd` closure (bytes) | 3,152,720 |
| Bare executable, stripped (bytes) *(not gated)* | 3,019,624 |
| Startup to first frame (ms) | 28.8 [21.6–37.2] |
| Idle RSS (MiB) | 4.7 |
| Idle PSS (MiB) | 3.1 |
| Idle heap (`RssAnon`) (MiB) | 0.5 |
| Peak memory (`VmHWM`) (MiB) | 4.7 |
| Idle wakeups per minute | 2 |
| Idle CPU in the window (ms) | 4.7 |
| CPU while switching workspaces (ms) | 0.9 |
| Wakeups while switching workspaces *(not gated)* | 2 |
| Threads *(not gated)* | 1 |

Machine, 12 readings around the runs.

scootbar's code: 94,775 lines of Rust in `crates/scootbar/src`, 63,422 outside `tests.rs` files; 12 direct dependencies on Linux (ab_glyph, png, rustix, scoot-ipc, scoot-look, scootbg-mem, serde, serde_json, toml, wayland-client, wayland-protocols, wayland-protocols-wlr).

Gate (no competitor beats scootbar): 0 loss(es).
