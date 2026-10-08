| Row | scootbg | scootbg-bilinear | awww | hyprpaper | swaybg | wbg | wpaperd |
|---|---|---|---|---|---|---|---|
| Size: stripped binaries + non-glibc `ldd` closure (bytes) | 1,841,928 | n/a | 7,421,368 | 52,574,976 | 14,323,368 | 14,732,512 | 14,586,640 |
| Disk: installed with its non-glibc closure, plus what it writes (bytes) | 34,858,673 | n/a | 35,132,158 | 176,474,512 | 101,231,856 | 62,619,128 | 39,563,560 |
| Idle RSS, 1× 1080p, image (MiB) *(not gated)* | 11.9 [11.9–11.9] | n/a | 10.3 [10.3–10.3] | 86.5 [86.5–86.7] | 8.9 [8.9–8.9] | 14.6 [14.5–14.6] | 72.8 [72.7–72.9] |
| Idle PSS, 1× 1080p, image (MiB) *(not gated)* | 6.3 [6.3–6.3] | n/a | 4.9 [4.9–4.9] | 59.3 [59.3–59.4] | 4.2 [4.1–4.2] | 7.0 [7.0–7.0] | 49.1 [49.1–49.3] |
| Idle floor (the buffers the compositor maps), 1× 1080p, image (MiB) *(not gated)* | 7.9 | n/a | 7.9 | 0.0 | 7.9 | 7.9 | 0.0 |
| Idle RSS above the floor, 1× 1080p, image (MiB) | 4.0 [4.0–4.0] | n/a | **2.4 [2.3–2.4]** (beats scootbg) | 86.5 [86.5–86.7] | 8.9 [8.9–8.9] | 6.6 [6.6–6.6] | 72.8 [72.7–72.9] |
| Idle PSS above the floor, 1× 1080p, image (MiB) | 2.3 [2.3–2.4] | n/a | **0.9 [0.9–0.9]** (beats scootbg) | 59.3 [59.3–59.4] | 4.2 [4.1–4.2] | 3.1 [3.0–3.1] | 49.1 [49.1–49.3] |
| Idle total with the floor (PSS above it + the floor + compositor copies), 1× 1080p, image (MiB) | 10.3 [10.3–10.3] | n/a | **8.8 [8.8–8.8]** (beats scootbg) | 59.3 [59.3–59.4] | 12.1 [12.1–12.1] | 11.0 [11.0–11.0] | 49.1 [49.1–49.3] |
| Idle RSS, 2× 4K, image (MiB) *(not gated)* | 35.5 [35.5–35.5] | n/a | 65.6 [65.6–65.6] | 86.5 [86.5–86.7] | 8.9 [8.9–8.9] | 69.9 | 75.8 [75.7–75.8] |
| Idle PSS, 2× 4K, image (MiB) *(not gated)* | 18.1 [18.1–18.1] | n/a | 32.5 [32.5–32.5] | 59.3 [59.3–59.5] | 4.2 [4.1–4.2] | 34.7 [34.7–34.7] | 52.1 [52.0–52.1] |
| Idle floor (the buffers the compositor maps), 2× 4K, image (MiB) *(not gated)* | 31.6 | n/a | 63.3 | 0.0 | 63.3 | 63.3 | 0.0 |
| Idle RSS above the floor, 2× 4K, image (MiB) | 3.9 [3.9–3.9] | n/a | **2.4 [2.3–2.4]** (beats scootbg) | 86.5 [86.5–86.7] | 8.9 [8.9–8.9] | 6.6 | 75.8 [75.7–75.8] |
| Idle PSS above the floor, 2× 4K, image (MiB) | 2.3 [2.2–2.3] | n/a | **0.9 [0.9–0.9]** (beats scootbg) | 59.3 [59.3–59.5] | 4.2 [4.1–4.2] | 3.1 [3.1–3.1] | 52.1 [52.0–52.1] |
| Idle total with the floor (PSS above it + the floor + compositor copies), 2× 4K, image (MiB) | 33.9 [33.9–33.9] | n/a | 64.2 [64.2–64.2] | 59.3 [59.3–59.5] | 67.4 [67.4–67.5] | 66.3 [66.3–66.3] | 52.1 [52.0–52.1] |
| Idle RSS, 1× 1080p, color (MiB) *(not gated)* | 3.5 | n/a | 10.1 [10.1–10.1] | n/a | 6.8 [6.8–6.8] | n/a | n/a |
| Idle PSS, 1× 1080p, color (MiB) *(not gated)* | 2.1 [2.1–2.1] | n/a | 4.9 [4.9–4.9] | n/a | 4.2 [4.2–4.2] | n/a | n/a |
| Idle floor (the buffers the compositor maps), 1× 1080p, color (MiB) *(not gated)* | 0.0 | n/a | 7.9 | n/a | 0.0 | n/a | n/a |
| Idle RSS above the floor, 1× 1080p, color (MiB) | 3.5 | n/a | **2.1 [2.1–2.2]** (beats scootbg) | n/a | 6.8 [6.8–6.8] | n/a | n/a |
| Idle PSS above the floor, 1× 1080p, color (MiB) | 2.1 [2.1–2.1] | n/a | **1.0 [0.9–1.0]** (beats scootbg) | n/a | 4.2 [4.2–4.2] | n/a | n/a |
| Idle total with the floor (PSS above it + the floor + compositor copies), 1× 1080p, color (MiB) | 2.1 [2.1–2.1] | n/a | 8.9 [8.9–8.9] | n/a | 4.2 [4.2–4.2] | n/a | n/a |
| Idle RSS, 2× 4K, color (MiB) *(not gated)* | 3.5 [3.5–3.5] | n/a | 65.4 [65.4–65.4] | n/a | 6.8 [6.8–6.8] | n/a | n/a |
| Idle PSS, 2× 4K, color (MiB) *(not gated)* | 2.1 [2.1–2.1] | n/a | 32.6 [32.6–32.6] | n/a | 4.2 [4.2–4.2] | n/a | n/a |
| Idle floor (the buffers the compositor maps), 2× 4K, color (MiB) *(not gated)* | 0.0 | n/a | 63.3 | n/a | 0.0 | n/a | n/a |
| Idle RSS above the floor, 2× 4K, color (MiB) | 3.5 [3.5–3.5] | n/a | **2.1 [2.1–2.2]** (beats scootbg) | n/a | 6.8 [6.8–6.8] | n/a | n/a |
| Idle PSS above the floor, 2× 4K, color (MiB) | 2.1 [2.1–2.1] | n/a | **1.0 [1.0–1.0]** (beats scootbg) | n/a | 4.2 [4.2–4.2] | n/a | n/a |
| Idle total with the floor (PSS above it + the floor + compositor copies), 2× 4K, color (MiB) | 2.1 [2.1–2.1] | n/a | 64.2 [64.2–64.2] | n/a | 4.2 [4.2–4.2] | n/a | n/a |
| Idle wakeups in 60 s, 1× 1080p, image | 0 | n/a | 0 | 48 | 0 | 0 | 122 [122–123] |
| Idle wakeups in 60 s, 2× 4K, image | 0 | n/a | 0 | 48 [48–52] | 0 | 0 | 136 [131–138] |
| Idle wakeups in 60 s, 1× 1080p, color | 0 | n/a | 0 | n/a | 0 | n/a | n/a |
| Idle wakeups in 60 s, 2× 4K, color | 0 | n/a | 0 | n/a | 0 | n/a | n/a |
| Idle CPU in 60 s, 1× 1080p, image (ms) | 0.0 | n/a | 0.0 | 0.5 [0.5–0.5] | 0.0 | 0.0 | 0.5 [0.4–0.5] |
| Idle CPU in 60 s, 2× 4K, image (ms) | 0.0 | n/a | 0.0 | 0.5 [0.5–0.6] | 0.0 | 0.0 | 0.6 [0.6–0.7] |
| Idle CPU in 60 s, 1× 1080p, color (ms) | 0.0 | n/a | 0.0 | n/a | 0.0 | n/a | n/a |
| Idle CPU in 60 s, 2× 4K, color (ms) | 0.0 | n/a | 0.0 | n/a | 0.0 | n/a | n/a |
| Peak memory (PSS), JPEG at start-up, 1× 4K (MiB) | 85.3 [83.2–85.4] | 82.2 [81.7–82.3] | 268.9 [266.1–275.2] | 176.2 [175.5–182.0] | 151.8 [130.4–167.0] | 104.4 [103.2–104.9] | 163.4 [163.4–213.0] |
| Peak memory (PSS), live change to the JPEG, 1× 4K (MiB) | 100.5 [99.7–101.2] | 96.7 [96.0–97.1] | 292.1 [284.0–293.1] | n/a | n/a | n/a | 232.3 [163.7–232.5] |
| Peak memory (RSS), JPEG at start-up, 1× 4K (MiB) *(not gated)* | 88.1 [87.5–88.4] | 87.8 [85.0–88.3] | 280.2 [278.2–280.6] | 185.3 [184.8–185.6] | 168.2 [168.1–168.2] | 106.0 [105.7–106.5] | 232.6 [229.4–232.6] |
| Peak memory (RSS), live change to the JPEG, 1× 4K (MiB) *(not gated)* | 120.0 [119.4–120.5] | 118.4 [117.7–119.0] | 307.9 [306.7–311.3] | n/a | n/a | n/a | 233.3 [231.2–233.5] |
| Set: latency to the JPEG (ms) | 290 [279–305] | 220 [212–222] | 332 [321–344] | n/a | n/a | n/a | 305 [275–318] |
| Set: CPU for the JPEG (ms) | 281 [274–297] | 214 [206–215] | 283 [280–304] | n/a | n/a | n/a | **238 [236–239]** (beats scootbg) |
| Set: latency to a color (ms) | 15.4 [11.8–27.2] | n/a | 81.3 [51.5–94.7] | n/a | n/a | n/a | n/a |
| Set: CPU for a color (ms) | 1.7 [1.2–1.9] | n/a | 56.5 [30.5–61.1] | n/a | n/a | n/a | n/a |
| Startup: to a color on screen (ms) | 27.2 [22.9–34.2] | n/a | 57.5 [53.9–68.1] | n/a | 28.6 [19.9–38.7] | n/a | n/a |
| Startup: CPU, color (ms) | 4.0 [3.3–4.5] | n/a | 37.9 [35.0–39.3] | n/a | 4.4 [3.6–5.4] | n/a | n/a |
| Startup: to the JPEG on screen (ms) | 310 [300–322] | 255 [230–255] | 328 [308–330] | 417 [381–440] | 337 [332–353] | 282 [247–286] | 378 [360–402] |
| Startup: CPU, JPEG (ms) | 283 [280–300] | 233 [208–233] | 271 [259–272] | 257 [219–272] | 314 [313–320] | 263 [234–267] | 281 [249–302] |
| Restore: to the JPEG on screen (ms) | 324 [310–337] | n/a | 334 [323–337] | n/a | n/a | n/a | n/a |
| Restore: CPU, JPEG (ms) | 287 [281–295] | n/a | 268 [259–272] | n/a | n/a | n/a | n/a |
| Restore: to a color on screen (ms) | 29.2 [28.1–30.5] | n/a | n/a | n/a | n/a | n/a | n/a |
| Restore: CPU, color (ms) | 0.5 [0.5–0.6] | n/a | n/a | n/a | n/a | n/a | n/a |

Gate: 10 loss(es), 64 win(s), 35 tie(s) for scootbg.
- LOSS: Idle RSS above the floor, 1× 1080p, image: awww 2.36 against scootbg 3.98 (margin 0.20)
- LOSS: Idle PSS above the floor, 1× 1080p, image: awww 0.90 against scootbg 2.35 (margin 0.12)
- LOSS: Idle total with the floor (PSS above it + the floor + compositor copies), 1× 1080p, image: awww 8.82 against scootbg 10.27 (margin 0.51)
- LOSS: Idle RSS above the floor, 2× 4K, image: awww 2.35 against scootbg 3.89 (margin 0.19)
- LOSS: Idle PSS above the floor, 2× 4K, image: awww 0.90 against scootbg 2.25 (margin 0.11)
- LOSS: Idle RSS above the floor, 1× 1080p, color: awww 2.15 against scootbg 3.50 (margin 0.18)
- LOSS: Idle PSS above the floor, 1× 1080p, color: awww 0.96 against scootbg 2.09 (margin 0.10)
- LOSS: Idle RSS above the floor, 2× 4K, color: awww 2.14 against scootbg 3.50 (margin 0.18)
- LOSS: Idle PSS above the floor, 2× 4K, color: awww 0.96 against scootbg 2.09 (margin 0.10)
- LOSS: Set: CPU for the JPEG: wpaperd 237.50 against scootbg 280.81 (margin 25.41)
- tie: Disk: installed with its non-glibc closure, plus what it writes: awww
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
- tie: Set: latency to the JPEG: awww
- tie: Set: latency to the JPEG: wpaperd
- tie: Set: CPU for the JPEG: awww
- tie: Startup: to a color on screen: swaybg
- tie: Startup: CPU, color: swaybg
- tie: Startup: to the JPEG on screen: awww
- tie: Startup: to the JPEG on screen: swaybg
- tie: Startup: to the JPEG on screen: wbg
- tie: Startup: CPU, JPEG: awww
- tie: Startup: CPU, JPEG: hyprpaper
- tie: Startup: CPU, JPEG: wbg
- tie: Startup: CPU, JPEG: wpaperd
- tie: Restore: to the JPEG on screen: awww
- tie: Restore: CPU, JPEG: awww

3 failed run(s):
- awww idle image round 3: ['awww: a client failed (1)']
- awww idle color round 5: ['awww: a client failed (1)']
- awww idle color round 5: ['awww: a client failed (1)']
