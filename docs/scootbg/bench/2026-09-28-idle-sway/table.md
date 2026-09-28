| Row | scootbg | scootbg-bilinear | awww | hyprpaper | swaybg | wbg | wpaperd |
|---|---|---|---|---|---|---|---|
| Size: stripped binaries + non-glibc `ldd` closure (bytes) | 1,866,680 | n/a | 9,080,536 | did not run | 13,459,384 | 13,303,784 | 15,287,664 |
| Disk: installed with its non-glibc closure, plus what it writes (bytes) | 12,176,264 | n/a | 33,294,752 | did not run | 94,586,520 | 57,149,648 | 35,294,624 |
| Idle RSS, 1× 1080p, image (MiB) *(not gated)* | 12.2 [12.2–12.2] | n/a | 10.8 [10.7–10.8] | did not run | 9.6 [9.4–9.8] | 15.1 [15.1–15.2] | 270.5 [270.0–352.4] |
| Idle PSS, 1× 1080p, image (MiB) *(not gated)* | 10.2 [10.2–10.3] | n/a | 8.9 [8.8–8.9] | did not run | 3.0 [3.0–3.1] | 11.0 [10.9–11.0] | 266.2 [265.7–348.2] |
| Idle floor (the buffers the compositor maps), 1× 1080p, image (MiB) *(not gated)* | 7.9 | n/a | 7.9 | did not run | 7.9 | 7.9 | 23.7 |
| Idle RSS above the floor, 1× 1080p, image (MiB) | 4.3 [4.3–4.3] | n/a | **2.9 [2.8–2.9]** (beats scootbg) | did not run | 9.6 [9.4–9.8] | 7.2 [7.2–7.2] | 246.7 [246.3–328.7] |
| Idle PSS above the floor, 1× 1080p, image (MiB) | 2.3 [2.3–2.4] | n/a | **1.0 [0.9–1.0]** (beats scootbg) | did not run | 3.0 [3.0–3.1] | 3.1 [3.0–3.1] | 242.5 [242.0–324.5] |
| Idle total with the floor (PSS above it + the floor + compositor copies), 1× 1080p, image (MiB) | 10.2 [10.2–10.3] | n/a | **8.9 [8.8–8.9]** (beats scootbg) | did not run | 11.0 [10.9–11.0] | 11.0 [10.9–11.0] | 266.2 [265.7–348.2] |
| Idle RSS, 2× 4K, image (MiB) *(not gated)* | 36.0 [36.0–36.1] | n/a | 66.2 [66.1–66.2] | did not run | 9.6 [9.5–9.7] | 70.5 [70.5–70.6] | 627.0 [626.8–709.3] |
| Idle PSS, 2× 4K, image (MiB) *(not gated)* | 34.0 [34.0–34.1] | n/a | 64.2 [64.2–64.2] | did not run | 3.0 [3.0–3.0] | 66.4 [66.3–66.4] | 622.7 [622.5–705.1] |
| Idle floor (the buffers the compositor maps), 2× 4K, image (MiB) *(not gated)* | 31.6 | n/a | 63.3 | did not run | 63.3 | 63.3 | 189.8 |
| Idle RSS above the floor, 2× 4K, image (MiB) | 4.4 [4.3–4.4] | n/a | **2.9 [2.8–2.9]** (beats scootbg) | did not run | 9.6 [9.5–9.7] | 7.2 [7.2–7.3] | 437.2 [437.0–519.5] |
| Idle PSS above the floor, 2× 4K, image (MiB) | 2.4 [2.4–2.4] | n/a | **1.0 [0.9–1.0]** (beats scootbg) | did not run | 3.0 [3.0–3.0] | 3.1 [3.0–3.1] | 432.9 [432.7–515.2] |
| Idle total with the floor (PSS above it + the floor + compositor copies), 2× 4K, image (MiB) | 34.0 [34.0–34.1] | n/a | 64.2 [64.2–64.3] | did not run | 66.3 [66.3–66.3] | 66.4 [66.3–66.4] | 622.7 [622.5–705.1] |
| Idle RSS, 1× 1080p, color (MiB) *(not gated)* | 3.8 [3.7–3.8] | n/a | 10.6 [10.6–10.6] | did not run | 7.5 [7.4–7.6] | n/a | n/a |
| Idle PSS, 1× 1080p, color (MiB) *(not gated)* | 2.0 [2.0–2.0] | n/a | 8.9 [8.9–9.0] | did not run | 2.5 [2.5–2.5] | n/a | n/a |
| Idle floor (the buffers the compositor maps), 1× 1080p, color (MiB) *(not gated)* | 0.0 | n/a | 7.9 | did not run | 0.0 | n/a | n/a |
| Idle RSS above the floor, 1× 1080p, color (MiB) | 3.8 [3.7–3.8] | n/a | **2.7 [2.6–2.7]** (beats scootbg) | did not run | 7.5 [7.4–7.6] | n/a | n/a |
| Idle PSS above the floor, 1× 1080p, color (MiB) | 2.0 [2.0–2.0] | n/a | **1.0 [1.0–1.0]** (beats scootbg) | did not run | 2.5 [2.5–2.5] | n/a | n/a |
| Idle total with the floor (PSS above it + the floor + compositor copies), 1× 1080p, color (MiB) | 2.0 [2.0–2.0] | n/a | 8.9 [8.9–9.0] | did not run | 2.5 [2.5–2.5] | n/a | n/a |
| Idle RSS, 2× 4K, color (MiB) *(not gated)* | 3.8 [3.7–3.9] | n/a | 66.0 [65.9–66.0] | did not run | 7.5 [7.3–7.5] | n/a | n/a |
| Idle PSS, 2× 4K, color (MiB) *(not gated)* | 2.0 [2.0–2.1] | n/a | 64.3 [64.3–64.3] | did not run | 2.5 [2.4–2.5] | n/a | n/a |
| Idle floor (the buffers the compositor maps), 2× 4K, color (MiB) *(not gated)* | 0.0 | n/a | 63.3 | did not run | 0.0 | n/a | n/a |
| Idle RSS above the floor, 2× 4K, color (MiB) | 3.8 [3.7–3.9] | n/a | **2.7 [2.6–2.7]** (beats scootbg) | did not run | 7.5 [7.3–7.5] | n/a | n/a |
| Idle PSS above the floor, 2× 4K, color (MiB) | 2.0 [2.0–2.1] | n/a | **1.0 [1.0–1.1]** (beats scootbg) | did not run | 2.5 [2.4–2.5] | n/a | n/a |
| Idle total with the floor (PSS above it + the floor + compositor copies), 2× 4K, color (MiB) | 2.0 [2.0–2.1] | n/a | 64.3 [64.3–64.3] | did not run | 2.5 [2.4–2.5] | n/a | n/a |
| Idle wakeups in 60 s, 1× 1080p, image | 0 | n/a | 0 | did not run | 0 | 0 | 127 |
| Idle wakeups in 60 s, 2× 4K, image | 0 | n/a | 0 | did not run | 0 | 0 | 166 [165–166] |
| Idle wakeups in 60 s, 1× 1080p, color | 0 | n/a | 0 | did not run | 0 | n/a | n/a |
| Idle wakeups in 60 s, 2× 4K, color | 0 | n/a | 0 | did not run | 0 | n/a | n/a |
| Idle CPU in 60 s, 1× 1080p, image (ms) | 0.0 | n/a | 0.0 | did not run | 0.0 | 0.0 | 5.8 [5.6–6.3] |
| Idle CPU in 60 s, 2× 4K, image (ms) | 0.0 | n/a | 0.0 | did not run | 0.0 | 0.0 | 7.7 [7.3–7.9] |
| Idle CPU in 60 s, 1× 1080p, color (ms) | 0.0 | n/a | 0.0 | did not run | 0.0 | n/a | n/a |
| Idle CPU in 60 s, 2× 4K, color (ms) | 0.0 | n/a | 0.0 | did not run | 0.0 | n/a | n/a |

Gate: 9 loss(es), 39 win(s), 20 tie(s) for scootbg.
- LOSS: Idle RSS above the floor, 1× 1080p, image: awww 2.89 against scootbg 4.30 (margin 0.22)
- LOSS: Idle PSS above the floor, 1× 1080p, image: awww 0.96 against scootbg 2.31 (margin 0.17)
- LOSS: Idle total with the floor (PSS above it + the floor + compositor copies), 1× 1080p, image: awww 8.88 against scootbg 10.23 (margin 0.51)
- LOSS: Idle RSS above the floor, 2× 4K, image: awww 2.90 against scootbg 4.37 (margin 0.22)
- LOSS: Idle PSS above the floor, 2× 4K, image: awww 0.95 against scootbg 2.39 (margin 0.12)
- LOSS: Idle RSS above the floor, 1× 1080p, color: awww 2.67 against scootbg 3.77 (margin 0.19)
- LOSS: Idle PSS above the floor, 1× 1080p, color: awww 1.02 against scootbg 2.00 (margin 0.10)
- LOSS: Idle RSS above the floor, 2× 4K, color: awww 2.69 against scootbg 3.79 (margin 0.20)
- LOSS: Idle PSS above the floor, 2× 4K, color: awww 1.04 against scootbg 2.03 (margin 0.16)
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
