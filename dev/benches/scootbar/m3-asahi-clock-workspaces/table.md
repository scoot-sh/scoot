**On scoot** (scoot 0.1.0 (ipc protocol 5))

| Row | scootbar | yambar | waybar |
|---|---|---|---|
| Size: stripped binary + non-glibc `ldd` closure (bytes) | 1,383,080 | 20,950,384 | 73,973,448 |
| Bare executable, stripped (bytes) *(not gated)* | 1,249,984 | 396,008 | 2,954,024 |
| Startup to first frame (ms) | 37.3 [33.2–45.1] | cannot show this scope | 97.8 [82.4–110] |
| Idle RSS (MiB) | 3.5 | cannot show this scope | 51.4 |
| Idle PSS (MiB) | 2.1 | cannot show this scope | 45.5 |
| Idle heap (`RssAnon`) (MiB) | 0.4 | cannot show this scope | 10.7 |
| Peak memory (`VmHWM`) (MiB) | 3.5 | cannot show this scope | 51.4 |
| Idle wakeups per minute | 2 | cannot show this scope | 5 |
| Idle CPU in the window (ms) | 1.0 | cannot show this scope | 5.1 |
| CPU while switching workspaces (ms) | 26.3 | cannot show this scope | 464 |
| Wakeups while switching workspaces *(not gated)* | 482 | cannot show this scope | 965 |
| Threads *(not gated)* | 1 | cannot show this scope | 8 |

**On sway** (sway version 1.12)

| Row | scootbar | yambar | waybar |
|---|---|---|---|
| Size: stripped binary + non-glibc `ldd` closure (bytes) | 1,383,080 | 20,950,384 | 73,973,448 |
| Bare executable, stripped (bytes) *(not gated)* | 1,249,984 | 396,008 | 2,954,024 |
| Startup to first frame (ms) | 18.2 [9.2–21.4] | 28.4 [17.4–33.1] | 62.9 [59.7–70.7] |
| Idle RSS (MiB) | 3.5 | 13.4 | 51.4 |
| Idle PSS (MiB) | 2.1 | 8.0 | 43.2 |
| Idle heap (`RssAnon`) (MiB) | 0.4 | 3.1 | 10.8 |
| Peak memory (`VmHWM`) (MiB) | 3.5 | 13.4 | 51.4 |
| Idle wakeups per minute | 2 | 3 | 3 |
| Idle CPU in the window (ms) | 0.8 | 2.5 | 4.1 |
| CPU while switching workspaces (ms) | 22.2 | 177 | 1425 |
| Wakeups while switching workspaces *(not gated)* | 482 | 1205 | 2642 |
| Threads *(not gated)* | 1 | 4 | 9 |

- tie: Startup to first frame: yambar
- tie: Idle wakeups per minute: yambar
- tie: Idle wakeups per minute: waybar

Machine, 60 readings around the runs: governor schedutil; cpufreq policy cap below the hardware maximum in 0; current frequency seen 600 to 3204 MHz; a mains supply offline in 0 of them; hwmon temperatures 22 to 26 C.

scootbar's code: 25,347 lines of Rust in `crates/scootbar/src`, 16,312 outside `tests.rs` files; 10 direct dependencies on Linux (ab_glyph, png, rustix, scootbg-mem, serde, serde_json, toml, wayland-client, wayland-protocols, wayland-protocols-wlr).

Gate (no competitor beats scootbar): 0 loss(es).
Not compared, so not passed: 1 bar and compositor pair(s) cannot show the milestone's scope, so no row was measured or invented for them (the rule does not say what to do then: a maintainer call).
- NOT COMPARED on scoot: yambar: yambar 1.11.0 has no ext-workspace-v1 module, so it cannot show workspaces on scoot
