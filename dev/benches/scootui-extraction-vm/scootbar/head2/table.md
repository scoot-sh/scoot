**On scoot** (scoot 0.7.3 (ipc protocol 13))

| Row | scootbar |
|---|---|
| Size: stripped binary + non-glibc `ldd` closure (bytes) | 3,283,824 |
| Bare executable, stripped (bytes) *(not gated)* | 3,150,728 |
| Startup to first frame (ms) | 40.4 [22.2–45.8] |
| Idle RSS (MiB) | 4.8 |
| Idle PSS (MiB) | 3.2 |
| Idle heap (`RssAnon`) (MiB) | 0.5 |
| Peak memory (`VmHWM`) (MiB) | 4.8 |
| Idle wakeups per minute | 2 |
| Idle CPU in the window (ms) | 3.1 |
| CPU while switching workspaces (ms) | 0.2 |
| Wakeups while switching workspaces *(not gated)* | 2 |
| Threads *(not gated)* | 1 |

Machine, 12 readings around the runs.

scootbar's code: 78,487 lines of Rust in `crates/scootbar/src`, 52,306 outside `tests.rs` files; 15 direct dependencies on Linux (ab_glyph, png, rustix, scoot-dbus, scoot-ipc, scoot-look, scoot-rt, scootbg-mem, scootui, serde, serde_json, toml, wayland-client, wayland-protocols, wayland-protocols-wlr).

Gate (no competitor beats scootbar): 0 loss(es).
