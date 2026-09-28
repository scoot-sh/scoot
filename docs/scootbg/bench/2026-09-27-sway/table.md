| Row | scootbg | scootbg-bilinear | awww | hyprpaper | swaybg | wbg | wpaperd |
|---|---|---|---|---|---|---|---|
| Size: stripped binaries + non-glibc `ldd` closure (bytes) | 1,866,680 | n/a | 9,080,536 | did not run | 13,459,384 | 13,303,784 | 15,287,664 |
| Disk: installed with its non-glibc closure, plus what it writes (bytes) | 12,172,256 | n/a | 33,294,797 | did not run | 94,586,520 | 57,149,648 | 35,294,647 |
| Idle RSS, 1× 1080p, image (MiB) *(not gated)* | 12.3 [12.2–12.3] | n/a | 10.8 [10.8–10.8] | did not run | 9.6 [9.5–9.7] | 15.1 [15.0–15.1] | 270.2 [270.0–270.4] |
| Idle PSS, 1× 1080p, image (MiB) *(not gated)* | 10.2 [10.2–10.3] | n/a | 8.9 [8.9–8.9] | did not run | 3.0 [3.0–3.1] | 11.0 [10.9–11.0] | 266.0 [265.7–266.1] |
| Idle floor (the buffers the compositor maps), 1× 1080p, image (MiB) *(not gated)* | 7.9 | n/a | 7.9 | did not run | 7.9 | 7.9 | 23.7 |
| Idle RSS above the floor, 1× 1080p, image (MiB) | 4.4 [4.3–4.4] | n/a | **2.9 [2.9–2.9]** (beats scootbg) | did not run | 9.6 [9.5–9.7] | 7.2 [7.1–7.2] | 246.5 [246.2–246.7] |
| Idle PSS above the floor, 1× 1080p, image (MiB) | 2.3 [2.3–2.4] | n/a | **1.0 [0.9–1.0]** (beats scootbg) | did not run | 3.0 [3.0–3.1] | 3.0 [3.0–3.1] | 242.3 [242.0–242.4] |
| Idle total with the floor (PSS above it + the floor + compositor copies), 1× 1080p, image (MiB) | 10.3 [10.2–10.3] | n/a | **8.9 [8.9–8.9]** (beats scootbg) | did not run | 10.9 [10.9–11.0] | 11.0 [10.9–11.0] | 266.0 [265.7–266.1] |
| Idle RSS, 2× 4K, image (MiB) *(not gated)* | 36.1 [36.0–36.1] | n/a | 66.1 [66.1–66.2] | did not run | 9.5 [9.4–9.7] | 70.5 [70.4–70.5] | 627.3 [627.1–627.5] |
| Idle PSS, 2× 4K, image (MiB) *(not gated)* | 34.0 [34.0–34.1] | n/a | 64.2 [64.2–64.3] | did not run | 3.0 [2.9–3.1] | 66.3 [66.3–66.4] | 623.0 [622.8–623.2] |
| Idle floor (the buffers the compositor maps), 2× 4K, image (MiB) *(not gated)* | 31.6 | n/a | 63.3 | did not run | 63.3 | 63.3 | 189.8 |
| Idle RSS above the floor, 2× 4K, image (MiB) | 4.4 [4.4–4.5] | n/a | **2.8 [2.8–2.9]** (beats scootbg) | did not run | 9.5 [9.4–9.7] | 7.2 [7.2–7.2] | 437.5 [437.2–437.7] |
| Idle PSS above the floor, 2× 4K, image (MiB) | 2.4 [2.4–2.5] | n/a | **0.9 [0.9–1.0]** (beats scootbg) | did not run | 3.0 [2.9–3.1] | 3.0 [3.0–3.1] | 433.1 [432.9–433.4] |
| Idle total with the floor (PSS above it + the floor + compositor copies), 2× 4K, image (MiB) | 34.0 [34.0–34.1] | n/a | 64.2 [64.2–64.3] | did not run | 66.3 [66.2–66.4] | 66.3 [66.3–66.4] | 623.0 [622.8–623.2] |
| Idle RSS, 1× 1080p, color (MiB) *(not gated)* | 3.9 [3.8–3.9] | n/a | 10.6 [10.5–10.6] | did not run | 7.5 [7.4–7.6] | n/a | n/a |
| Idle PSS, 1× 1080p, color (MiB) *(not gated)* | 2.1 [2.0–2.1] | n/a | 8.9 [8.9–9.0] | did not run | 2.5 [2.5–2.6] | n/a | n/a |
| Idle floor (the buffers the compositor maps), 1× 1080p, color (MiB) *(not gated)* | 0.0 | n/a | 7.9 | did not run | 0.0 | n/a | n/a |
| Idle RSS above the floor, 1× 1080p, color (MiB) | 3.9 [3.8–3.9] | n/a | **2.7 [2.6–2.7]** (beats scootbg) | did not run | 7.5 [7.4–7.6] | n/a | n/a |
| Idle PSS above the floor, 1× 1080p, color (MiB) | 2.1 [2.0–2.1] | n/a | **1.0 [1.0–1.1]** (beats scootbg) | did not run | 2.5 [2.5–2.6] | n/a | n/a |
| Idle total with the floor (PSS above it + the floor + compositor copies), 1× 1080p, color (MiB) | 2.1 [2.1–2.1] | n/a | 8.9 [8.9–9.0] | did not run | 2.5 [2.5–2.6] | n/a | n/a |
| Idle RSS, 2× 4K, color (MiB) *(not gated)* | 3.9 [3.8–3.9] | n/a | 66.0 [65.9–66.0] | did not run | 7.4 [7.3–7.6] | n/a | n/a |
| Idle PSS, 2× 4K, color (MiB) *(not gated)* | 2.1 [2.0–2.1] | n/a | 64.3 [64.3–64.4] | did not run | 2.5 [2.4–2.6] | n/a | n/a |
| Idle floor (the buffers the compositor maps), 2× 4K, color (MiB) *(not gated)* | 0.0 | n/a | 63.3 | did not run | 0.0 | n/a | n/a |
| Idle RSS above the floor, 2× 4K, color (MiB) | 3.9 [3.8–3.9] | n/a | **2.7 [2.6–2.7]** (beats scootbg) | did not run | 7.4 [7.3–7.6] | n/a | n/a |
| Idle PSS above the floor, 2× 4K, color (MiB) | 2.1 [2.0–2.1] | n/a | **1.0 [1.0–1.1]** (beats scootbg) | did not run | 2.5 [2.4–2.6] | n/a | n/a |
| Idle total with the floor (PSS above it + the floor + compositor copies), 2× 4K, color (MiB) | 2.1 [2.0–2.1] | n/a | 64.3 [64.3–64.4] | did not run | 2.5 [2.5–2.6] | n/a | n/a |
| Idle wakeups in 60 s, 1× 1080p, image | 0 | n/a | 0 | did not run | 0 | 0 | 127 [126–127] |
| Idle wakeups in 60 s, 2× 4K, image | 0 | n/a | 0 | did not run | 0 | 0 | 167 [165–168] |
| Idle wakeups in 60 s, 1× 1080p, color | 0 | n/a | 0 | did not run | 0 | n/a | n/a |
| Idle wakeups in 60 s, 2× 4K, color | 0 | n/a | 0 | did not run | 0 | n/a | n/a |
| Idle CPU in 60 s, 1× 1080p, image (ms) | 0.0 | n/a | 0.0 | did not run | 0.0 | 0.0 | 6.1 [5.6–6.3] |
| Idle CPU in 60 s, 2× 4K, image (ms) | 0.0 | n/a | 0.0 | did not run | 0.0 | 0.0 | 7.6 [7.5–7.9] |
| Idle CPU in 60 s, 1× 1080p, color (ms) | 0.0 | n/a | 0.0 | did not run | 0.0 | n/a | n/a |
| Idle CPU in 60 s, 2× 4K, color (ms) | 0.0 | n/a | 0.0 | did not run | 0.0 | n/a | n/a |
| Peak memory, JPEG at start-up, 1× 4K (MiB) | 89.1 [88.8–89.5] | 88.8 [87.5–89.0] | 282.0 [281.9–282.2] | did not run | 169.4 [169.3–169.5] | 107.4 [107.2–107.5] | 420.8 [420.8–420.9] |
| Peak memory, live change to the JPEG, 1× 4K (MiB) | 121.0 [121.0–121.2] | 120.6 [120.2–121.1] | 313.8 [313.6–314.0] | did not run | n/a | n/a | 549.2 [548.7–550.5] |
| Set: latency to the JPEG (ms) | 464 [452–493] | 388 [359–419] | 624 [611–633] | did not run | n/a | n/a | 697 [669–721] |
| Set: CPU for the JPEG (ms) | 452 [442–484] | 376 [353–392] | 572 [565–587] | did not run | n/a | n/a | 862 [849–898] |
| Set: latency to a color (ms) | 17.2 [14.3–27.1] | n/a | 62.1 [59.2–67.4] | did not run | n/a | n/a | n/a |
| Set: CPU for a color (ms) | 4.0 [3.8–4.3] | n/a | 36.4 [35.4–45.8] | did not run | n/a | n/a | n/a |
| Startup: to a color on screen (ms) | 24.2 [17.5–35.2] | n/a | 73.6 [64.2–89.6] | did not run | 32.1 [16.4–32.8] | n/a | n/a |
| Startup: CPU, color (ms) | 5.3 [4.6–5.5] | n/a | 50.7 [47.1–55.8] | did not run | 5.8 [5.5–6.4] | n/a | n/a |
| Startup: to the JPEG on screen (ms) | 490 [454–524] | 405 [381–429] | 631 [606–680] | did not run | 565 [541–614] | 475 [446–516] | 984 [975–992] |
| Startup: CPU, JPEG (ms) | 459 [435–501] | 370 [360–398] | 565 [548–612] | did not run | 533 [517–582] | 449 [410–473] | 1514 [1502–1552] |
| Restore: to the JPEG on screen (ms) | 489 [458–516] | n/a | 615 [602–667] | did not run | n/a | n/a | n/a |
| Restore: CPU, JPEG (ms) | 476 [440–491] | n/a | 566 [556–602] | did not run | n/a | n/a | n/a |
| Restore: to a color on screen (ms) | 15.8 [11.8–25.0] | n/a | n/a | did not run | n/a | n/a | n/a |
| Restore: CPU, color (ms) | 2.3 [2.2–2.4] | n/a | n/a | did not run | n/a | n/a | n/a |

Gate: 9 loss(es), 56 win(s), 29 tie(s) for scootbg.
- LOSS: Idle RSS above the floor, 1× 1080p, image: awww 2.90 against scootbg 4.38 (margin 0.22)
- LOSS: Idle PSS above the floor, 1× 1080p, image: awww 0.96 against scootbg 2.34 (margin 0.15)
- LOSS: Idle total with the floor (PSS above it + the floor + compositor copies), 1× 1080p, image: awww 8.88 against scootbg 10.26 (margin 0.51)
- LOSS: Idle RSS above the floor, 2× 4K, image: awww 2.84 against scootbg 4.45 (margin 0.24)
- LOSS: Idle PSS above the floor, 2× 4K, image: awww 0.90 against scootbg 2.39 (margin 0.20)
- LOSS: Idle RSS above the floor, 1× 1080p, color: awww 2.66 against scootbg 3.88 (margin 0.19)
- LOSS: Idle PSS above the floor, 1× 1080p, color: awww 1.00 against scootbg 2.06 (margin 0.10)
- LOSS: Idle RSS above the floor, 2× 4K, color: awww 2.70 against scootbg 3.87 (margin 0.19)
- LOSS: Idle PSS above the floor, 2× 4K, color: awww 1.04 against scootbg 2.08 (margin 0.17)
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
- tie: Startup: to a color on screen: swaybg
- tie: Startup: CPU, color: swaybg
- tie: Startup: to the JPEG on screen: awww
- tie: Startup: to the JPEG on screen: swaybg
- tie: Startup: to the JPEG on screen: wbg
- tie: Startup: CPU, JPEG: awww
- tie: Startup: CPU, JPEG: swaybg
- tie: Startup: CPU, JPEG: wbg
- tie: Restore: CPU, JPEG: awww
