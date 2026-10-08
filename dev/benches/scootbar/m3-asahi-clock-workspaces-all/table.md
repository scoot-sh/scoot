**On scoot** (scoot 0.1.0 (ipc protocol 5))

| Row | scootbar | yambar | waybar | ironbar (informational) | ashell (informational) |
|---|---|---|---|---|---|
| Size: stripped binary + non-glibc `ldd` closure (bytes) | 1,383,080 | 20,950,384 | 73,973,448 | 100,859,448 | 39,516,656 |
| Bare executable, stripped (bytes) *(not gated)* | 1,249,984 | 396,008 | 2,954,024 | 24,746,760 | 29,113,952 |
| Startup to first frame (ms) | 34.4 [33.6–45.4] | cannot show this scope | 91.7 [85.9–101] | cannot show this scope | 28.6 [27.3–38.1] |
| Idle RSS (MiB) | 3.5 | cannot show this scope | 51.3 | cannot show this scope | 28.9 |
| Idle PSS (MiB) | 2.1 | cannot show this scope | 45.6 | cannot show this scope | 26.4 |
| Idle heap (`RssAnon`) (MiB) | 0.4 | cannot show this scope | 10.7 | cannot show this scope | 6.7 |
| Peak memory (`VmHWM`) (MiB) | 3.5 | cannot show this scope | 51.3 | cannot show this scope | 28.9 |
| Idle wakeups per minute | 2 | cannot show this scope | 5 | cannot show this scope | 276 |
| Idle CPU in the window (ms) | 1.1 | cannot show this scope | 4.3 | cannot show this scope | 95.1 |
| CPU while switching workspaces (ms) | 27.5 | cannot show this scope | 482 | cannot show this scope | 267 |
| Wakeups while switching workspaces *(not gated)* | 482 | cannot show this scope | 965 | cannot show this scope | 3208 |
| Threads *(not gated)* | 1 | cannot show this scope | 8 | cannot show this scope | 12 |

**On sway** (sway version 1.12)

| Row | scootbar | yambar | waybar | ironbar (informational) | ashell (informational) |
|---|---|---|---|---|---|
| Size: stripped binary + non-glibc `ldd` closure (bytes) | 1,383,080 | 20,950,384 | 73,973,448 | 100,859,448 | 39,516,656 |
| Bare executable, stripped (bytes) *(not gated)* | 1,249,984 | 396,008 | 2,954,024 | 24,746,760 | 29,113,952 |
| Startup to first frame (ms) | 17.1 [7.8–17.7] | 27.2 [18.8–36.3] | 69.5 [54.4–86.0] | 61.0 [51.2–66.2] | 30.5 [21.1–35.4] |
| Idle RSS (MiB) | 3.5 | 13.4 | 51.4 | 56.2 | 28.1 |
| Idle PSS (MiB) | 2.1 | 8.0 | 43.2 | 47.9 | 25.6 |
| Idle heap (`RssAnon`) (MiB) | 0.4 | 3.1 | 10.8 | 14.8 | 6.2 |
| Peak memory (`VmHWM`) (MiB) | 3.5 | 13.4 | 51.4 | 56.2 | 28.1 |
| Idle wakeups per minute | 2 | 3 | 3 | 313 | 194 |
| Idle CPU in the window (ms) | 0.6 | 2.3 | 5.5 | 56.0 | 81.4 |
| CPU while switching workspaces (ms) | 22.3 | 187 | 1455 | 127 | 234 |
| Wakeups while switching workspaces *(not gated)* | 481 | 1202 | 2298 | 1942 | 2912 |
| Threads *(not gated)* | 1 | 4 | 9 | 19 | 12 |

- tie: Startup to first frame: yambar
- tie: Idle wakeups per minute: yambar
- tie: Idle wakeups per minute: waybar

Machine, 96 readings around the runs: governor schedutil; cpufreq policy cap below the hardware maximum in 0; current frequency seen 600 to 3204 MHz; a mains supply offline in 0 of them; hwmon temperatures 22 to 31 C.

scootbar's code: 25,347 lines of Rust in `crates/scootbar/src`, 16,312 outside `tests.rs` files; 10 direct dependencies on Linux (ab_glyph, png, rustix, scootbg-mem, serde, serde_json, toml, wayland-client, wayland-protocols, wayland-protocols-wlr).

Gate (no competitor beats scootbar): 0 loss(es).
Not compared, so not passed: 1 bar and compositor pair(s) cannot show the milestone's scope, so no row was measured or invented for them (the rule does not say what to do then: a maintainer call).
- NOT COMPARED on scoot: yambar: yambar 1.11.0 has no ext-workspace-v1 module, so it cannot show workspaces on scoot

Informational, not gated (ironbar, ashell): the ratified competitors are yambar and Waybar; these columns are context, and promoting them into the rule is the maintainer's call.
0 gated row(s) on which one of them beats scootbar (a finding, counted nowhere).
- not shown on scoot: ironbar: ironbar 0.19.0 has no ext-workspace-v1 support (it speaks compositor IPCs), so it cannot show workspaces on scoot
