| Row | scootbg | scootbg-bilinear | awww | hyprpaper | swaybg | wbg | wpaperd |
|---|---|---|---|---|---|---|---|
| Size: stripped binaries + non-glibc `ldd` closure (bytes) | 1,866,680 | n/a | 9,080,536 | did not run | 13,459,384 | 13,303,784 | 15,287,664 |
| Disk: installed with its non-glibc closure, plus what it writes (bytes) | 12,176,264 | n/a | 33,294,752 | did not run | 94,586,520 | 57,149,648 | 35,294,624 |
| Idle RSS, 1× 1080p, image (MiB) *(not gated)* | 12.2 [12.1–12.2] | n/a | 10.8 [10.7–10.8] | did not run | 9.5 [9.4–9.5] | 15.1 [15.0–15.1] | 269.7 [269.6–269.9] |
| Idle PSS, 1× 1080p, image (MiB) *(not gated)* | 6.3 [6.2–6.3] | n/a | 4.9 [4.9–5.0] | did not run | 5.2 [5.1–5.2] | 7.2 [7.2–7.3] | 254.4 [254.3–254.6] |
| Idle floor (the buffers the compositor maps), 1× 1080p, image (MiB) *(not gated)* | 7.9 | n/a | 7.9 | did not run | 7.9 | 7.9 | 23.7 |
| Idle RSS above the floor, 1× 1080p, image (MiB) | 4.3 [4.2–4.3] | n/a | **2.8 [2.8–2.9]** (beats scootbg) | did not run | 9.5 [9.4–9.5] | 7.2 [7.1–7.2] | 245.9 [245.9–246.1] |
| Idle PSS above the floor, 1× 1080p, image (MiB) | 2.3 [2.3–2.4] | n/a | **0.9 [0.9–1.0]** (beats scootbg) | did not run | 5.2 [5.1–5.2] | 3.3 [3.2–3.3] | 242.6 [242.5–242.8] |
| Idle total with the floor (PSS above it + the floor + compositor copies), 1× 1080p, image (MiB) | 10.3 [10.2–10.3] | n/a | **8.8 [8.8–8.9]** (beats scootbg) | did not run | 13.1 [13.0–13.1] | 11.2 [11.2–11.3] | 266.3 [266.2–266.5] |
| Idle RSS, 2× 4K, image (MiB) *(not gated)* | 36.0 [36.0–36.1] | n/a | 66.2 [66.1–66.2] | did not run | 9.6 [9.4–9.7] | 70.4 [70.4–70.5] | 626.6 [626.3–626.7] |
| Idle PSS, 2× 4K, image (MiB) *(not gated)* | 18.2 [18.2–18.3] | n/a | 32.6 [32.6–32.6] | did not run | 5.2 [5.1–5.3] | 34.9 [34.9–35.0] | 528.3 [527.9–528.4] |
| Idle floor (the buffers the compositor maps), 2× 4K, image (MiB) *(not gated)* | 31.6 | n/a | 63.3 | did not run | 63.3 | 63.3 | 189.8 |
| Idle RSS above the floor, 2× 4K, image (MiB) | 4.4 [4.3–4.5] | n/a | **2.9 [2.9–2.9]** (beats scootbg) | did not run | 9.6 [9.4–9.7] | 7.1 [7.1–7.2] | 436.7 [436.4–436.9] |
| Idle PSS above the floor, 2× 4K, image (MiB) | 2.4 [2.4–2.5] | n/a | **1.0 [1.0–1.0]** (beats scootbg) | did not run | 5.2 [5.1–5.3] | 3.3 [3.2–3.3] | 433.4 [433.0–433.5] |
| Idle total with the floor (PSS above it + the floor + compositor copies), 2× 4K, image (MiB) | 34.0 [34.0–34.1] | n/a | 64.3 [64.2–64.3] | did not run | 68.5 [68.3–68.6] | 66.6 [66.5–66.6] | 623.2 [622.9–623.3] |
| Idle RSS, 1× 1080p, color (MiB) *(not gated)* | 3.8 [3.8–3.9] | n/a | 10.6 [10.6–10.7] | did not run | 7.5 [7.4–7.6] | n/a | n/a |
| Idle PSS, 1× 1080p, color (MiB) *(not gated)* | 2.0 [2.0–2.1] | n/a | 5.0 [4.9–5.0] | did not run | 4.4 [4.3–4.5] | n/a | n/a |
| Idle floor (the buffers the compositor maps), 1× 1080p, color (MiB) *(not gated)* | 0.0 | n/a | 7.9 | did not run | 0.0 | n/a | n/a |
| Idle RSS above the floor, 1× 1080p, color (MiB) | 3.8 [3.8–3.9] | n/a | **2.7 [2.7–2.7]** (beats scootbg) | did not run | 7.5 [7.4–7.6] | n/a | n/a |
| Idle PSS above the floor, 1× 1080p, color (MiB) | 2.0 [2.0–2.1] | n/a | **1.0 [1.0–1.1]** (beats scootbg) | did not run | 4.4 [4.3–4.5] | n/a | n/a |
| Idle total with the floor (PSS above it + the floor + compositor copies), 1× 1080p, color (MiB) | 2.0 [2.0–2.1] | n/a | 8.9 [8.9–9.0] | did not run | 4.4 [4.3–4.5] | n/a | n/a |
| Idle RSS, 2× 4K, color (MiB) *(not gated)* | 3.8 [3.7–3.8] | n/a | 66.0 [65.9–66.0] | did not run | 7.5 [7.4–7.6] | n/a | n/a |
| Idle PSS, 2× 4K, color (MiB) *(not gated)* | 2.0 [2.0–2.1] | n/a | 32.7 [32.6–32.7] | did not run | 4.3 [4.3–4.5] | n/a | n/a |
| Idle floor (the buffers the compositor maps), 2× 4K, color (MiB) *(not gated)* | 0.0 | n/a | 63.3 | did not run | 0.0 | n/a | n/a |
| Idle RSS above the floor, 2× 4K, color (MiB) | 3.8 [3.7–3.8] | n/a | **2.7 [2.7–2.7]** (beats scootbg) | did not run | 7.5 [7.4–7.6] | n/a | n/a |
| Idle PSS above the floor, 2× 4K, color (MiB) | 2.0 [2.0–2.1] | n/a | **1.0 [1.0–1.1]** (beats scootbg) | did not run | 4.3 [4.3–4.5] | n/a | n/a |
| Idle total with the floor (PSS above it + the floor + compositor copies), 2× 4K, color (MiB) | 2.1 [2.0–2.1] | n/a | 64.3 [64.3–64.4] | did not run | 4.3 [4.3–4.5] | n/a | n/a |
| Idle wakeups in 60 s, 1× 1080p, image | 0 | n/a | 0 | did not run | 0 | 0 | 125 |
| Idle wakeups in 60 s, 2× 4K, image | 0 | n/a | 0 | did not run | 0 | 0 | 160 [159–160] |
| Idle wakeups in 60 s, 1× 1080p, color | 0 | n/a | 0 | did not run | 0 | n/a | n/a |
| Idle wakeups in 60 s, 2× 4K, color | 0 | n/a | 0 | did not run | 0 | n/a | n/a |
| Idle CPU in 60 s, 1× 1080p, image (ms) | 0.0 | n/a | 0.0 | did not run | 0.0 | 0.0 | 6.1 [5.7–6.2] |
| Idle CPU in 60 s, 2× 4K, image (ms) | 0.0 | n/a | 0.0 | did not run | 0.0 | 0.0 | 7.2 [7.2–7.4] |
| Idle CPU in 60 s, 1× 1080p, color (ms) | 0.0 | n/a | 0.0 | did not run | 0.0 | n/a | n/a |
| Idle CPU in 60 s, 2× 4K, color (ms) | 0.0 | n/a | 0.0 | did not run | 0.0 | n/a | n/a |

Gate: 9 loss(es), 39 win(s), 20 tie(s) for scootbg.
- LOSS: Idle RSS above the floor, 1× 1080p, image: awww 2.84 against scootbg 4.29 (margin 0.21)
- LOSS: Idle PSS above the floor, 1× 1080p, image: awww 0.93 against scootbg 2.34 (margin 0.16)
- LOSS: Idle total with the floor (PSS above it + the floor + compositor copies), 1× 1080p, image: awww 8.84 against scootbg 10.25 (margin 0.51)
- LOSS: Idle RSS above the floor, 2× 4K, image: awww 2.88 against scootbg 4.39 (margin 0.22)
- LOSS: Idle PSS above the floor, 2× 4K, image: awww 0.99 against scootbg 2.40 (margin 0.14)
- LOSS: Idle RSS above the floor, 1× 1080p, color: awww 2.71 against scootbg 3.77 (margin 0.19)
- LOSS: Idle PSS above the floor, 1× 1080p, color: awww 1.04 against scootbg 2.02 (margin 0.13)
- LOSS: Idle RSS above the floor, 2× 4K, color: awww 2.69 against scootbg 3.79 (margin 0.19)
- LOSS: Idle PSS above the floor, 2× 4K, color: awww 1.04 against scootbg 2.05 (margin 0.12)
- tie: Idle wakeups in 60 s, 1× 1080p, image: awww
- tie: Idle wakeups in 60 s, 1× 1080p, image: swaybg
- tie: Idle wakeups in 60 s, 1× 1080p, image: wbg
- tie: Idle wakeups in 60 s, 2× 4K, image: awww
- tie: Idle wakeups in 60 s, 2× 4K, image: swaybg
- tie: Idle wakeups in 60 s, 2× 4K, image: wbg
- tie: Idle wakeups in 60 s, 1× 1080p, color: awww
- tie: Idle wakeups in 60 s, 1× 1080p, color: swaybg
- tie: Idle wakeups in 60 s, 2× 4K, color: awww
- tie: Idle wakeups in 60 s, 2× 4K, color: swaybg
- tie: Idle CPU in 60 s, 1× 1080p, image: awww
- tie: Idle CPU in 60 s, 1× 1080p, image: swaybg
- tie: Idle CPU in 60 s, 1× 1080p, image: wbg
- tie: Idle CPU in 60 s, 2× 4K, image: awww
- tie: Idle CPU in 60 s, 2× 4K, image: swaybg
- tie: Idle CPU in 60 s, 2× 4K, image: wbg
- tie: Idle CPU in 60 s, 1× 1080p, color: awww
- tie: Idle CPU in 60 s, 1× 1080p, color: swaybg
- tie: Idle CPU in 60 s, 2× 4K, color: awww
- tie: Idle CPU in 60 s, 2× 4K, color: swaybg
