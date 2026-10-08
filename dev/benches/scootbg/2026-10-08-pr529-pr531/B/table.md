| Row | scootbg | scootbg-bilinear | awww | hyprpaper | swaybg | wbg | wpaperd |
|---|---|---|---|---|---|---|---|
| Size: stripped binaries + non-glibc `ldd` closure (bytes) | 1,907,464 | n/a | 7,421,368 | 52,574,976 | 14,323,368 | 14,732,512 | 14,586,640 |
| Disk: installed with its non-glibc closure, plus what it writes (bytes) | 34,989,745 | n/a | 35,132,158 | 176,474,512 | 101,231,856 | 62,619,128 | 39,563,560 |
| Idle RSS, 1× 1080p, image (MiB) *(not gated)* | 12.1 [12.1–12.1] | n/a | 10.3 [10.3–10.3] | 86.6 [86.5–86.6] | 8.9 [8.8–8.9] | 14.6 [14.6–14.6] | 72.8 [72.8–72.8] |
| Idle PSS, 1× 1080p, image (MiB) *(not gated)* | 6.5 [6.5–6.5] | n/a | 4.9 [4.9–4.9] | 59.3 [59.3–59.4] | 4.1 [4.1–4.1] | 7.0 [7.0–7.0] | 49.2 [49.1–49.2] |
| Idle floor (the buffers the compositor maps), 1× 1080p, image (MiB) *(not gated)* | 7.9 | n/a | 7.9 | 0.0 | 7.9 | 7.9 | 0.0 |
| Idle RSS above the floor, 1× 1080p, image (MiB) | 4.2 [4.2–4.2] | n/a | **2.4 [2.4–2.4]** (beats scootbg) | 86.6 [86.5–86.6] | 8.9 [8.8–8.9] | 6.6 [6.6–6.7] | 72.8 [72.8–72.8] |
| Idle PSS above the floor, 1× 1080p, image (MiB) | 2.5 [2.5–2.5] | n/a | **0.9 [0.9–0.9]** (beats scootbg) | 59.3 [59.3–59.4] | 4.1 [4.1–4.1] | 3.1 [3.1–3.1] | 49.2 [49.1–49.2] |
| Idle total with the floor (PSS above it + the floor + compositor copies), 1× 1080p, image (MiB) | 10.4 [10.4–10.4] | n/a | **8.8 [8.8–8.8]** (beats scootbg) | 59.3 [59.3–59.4] | 12.0 [12.0–12.0] | 11.0 [11.0–11.0] | 49.2 [49.1–49.2] |
| Idle RSS, 2× 4K, image (MiB) *(not gated)* | 35.7 [35.7–35.7] | n/a | 65.7 [65.6–65.7] | 86.6 [86.5–86.9] | 8.9 [8.8–8.9] | 69.9 [69.9–69.9] | 75.8 [75.8–75.8] |
| Idle PSS, 2× 4K, image (MiB) *(not gated)* | 18.2 [18.2–18.3] | n/a | 32.6 [32.6–32.6] | 59.4 [59.3–59.6] | 4.1 [4.1–4.2] | 34.7 [34.7–34.7] | 52.1 [52.1–52.1] |
| Idle floor (the buffers the compositor maps), 2× 4K, image (MiB) *(not gated)* | 31.6 | n/a | 63.3 | 0.0 | 63.3 | 63.3 | 0.0 |
| Idle RSS above the floor, 2× 4K, image (MiB) | 4.1 [4.1–4.1] | n/a | **2.4 [2.4–2.4]** (beats scootbg) | 86.6 [86.5–86.9] | 8.9 [8.8–8.9] | 6.6 [6.6–6.6] | 75.8 [75.8–75.8] |
| Idle PSS above the floor, 2× 4K, image (MiB) | 2.4 [2.4–2.4] | n/a | **0.9 [0.9–0.9]** (beats scootbg) | 59.4 [59.3–59.6] | 4.1 [4.1–4.2] | 3.1 [3.0–3.1] | 52.1 [52.1–52.1] |
| Idle total with the floor (PSS above it + the floor + compositor copies), 2× 4K, image (MiB) | 34.1 [34.0–34.1] | n/a | 64.2 [64.2–64.2] | 59.4 [59.3–59.6] | 67.4 [67.4–67.4] | 66.3 [66.3–66.3] | 52.1 [52.1–52.1] |
| Idle RSS, 1× 1080p, color (MiB) *(not gated)* | 3.7 [3.7–3.7] | n/a | 10.1 [10.1–10.1] | n/a | 6.8 [6.7–6.8] | n/a | n/a |
| Idle PSS, 1× 1080p, color (MiB) *(not gated)* | 2.2 [2.2–2.3] | n/a | 4.9 [4.9–4.9] | n/a | 4.2 [4.1–4.2] | n/a | n/a |
| Idle floor (the buffers the compositor maps), 1× 1080p, color (MiB) *(not gated)* | 0.0 | n/a | 7.9 | n/a | 0.0 | n/a | n/a |
| Idle RSS above the floor, 1× 1080p, color (MiB) | 3.7 [3.7–3.7] | n/a | **2.2 [2.2–2.2]** (beats scootbg) | n/a | 6.8 [6.7–6.8] | n/a | n/a |
| Idle PSS above the floor, 1× 1080p, color (MiB) | 2.2 [2.2–2.3] | n/a | **1.0 [1.0–1.0]** (beats scootbg) | n/a | 4.2 [4.1–4.2] | n/a | n/a |
| Idle total with the floor (PSS above it + the floor + compositor copies), 1× 1080p, color (MiB) | 2.2 [2.2–2.3] | n/a | 8.9 [8.9–8.9] | n/a | 4.2 [4.1–4.2] | n/a | n/a |
| Idle RSS, 2× 4K, color (MiB) *(not gated)* | 3.7 [3.7–3.7] | n/a | 65.5 | n/a | 6.8 [6.8–6.8] | n/a | n/a |
| Idle PSS, 2× 4K, color (MiB) *(not gated)* | 2.2 [2.2–2.3] | n/a | 32.6 [32.6–32.6] | n/a | 4.2 [4.2–4.2] | n/a | n/a |
| Idle floor (the buffers the compositor maps), 2× 4K, color (MiB) *(not gated)* | 0.0 | n/a | 63.3 | n/a | 0.0 | n/a | n/a |
| Idle RSS above the floor, 2× 4K, color (MiB) | 3.7 [3.7–3.7] | n/a | **2.2** (beats scootbg) | n/a | 6.8 [6.8–6.8] | n/a | n/a |
| Idle PSS above the floor, 2× 4K, color (MiB) | 2.2 [2.2–2.3] | n/a | **1.0 [1.0–1.0]** (beats scootbg) | n/a | 4.2 [4.2–4.2] | n/a | n/a |
| Idle total with the floor (PSS above it + the floor + compositor copies), 2× 4K, color (MiB) | 2.2 [2.2–2.3] | n/a | 64.3 [64.3–64.3] | n/a | 4.2 [4.2–4.2] | n/a | n/a |
| Idle wakeups in 60 s, 1× 1080p, image | 0 | n/a | 0 | 48 | 0 | 0 | 122 [122–123] |
| Idle wakeups in 60 s, 2× 4K, image | 0 | n/a | 0 | 48 [48–51] | 0 | 0 | 137 [134–137] |
| Idle wakeups in 60 s, 1× 1080p, color | 0 | n/a | 0 | n/a | 0 | n/a | n/a |
| Idle wakeups in 60 s, 2× 4K, color | 0 | n/a | 0 | n/a | 0 | n/a | n/a |
| Idle CPU in 60 s, 1× 1080p, image (ms) | 0.0 | n/a | 0.0 | 0.5 [0.5–0.6] | 0.0 | 0.0 | 0.5 [0.5–0.6] |
| Idle CPU in 60 s, 2× 4K, image (ms) | 0.0 | n/a | 0.0 | 0.5 [0.5–0.6] | 0.0 | 0.0 | 0.7 [0.6–0.7] |
| Idle CPU in 60 s, 1× 1080p, color (ms) | 0.0 | n/a | 0.0 | n/a | 0.0 | n/a | n/a |
| Idle CPU in 60 s, 2× 4K, color (ms) | 0.0 | n/a | 0.0 | n/a | 0.0 | n/a | n/a |
| Peak memory (PSS), JPEG at start-up, 1× 4K (MiB) | 84.3 [83.3–84.9] | 81.2 [79.1–85.2] | 278.5 [275.3–278.6] | 178.1 [175.6–182.1] | 130.4 [130.4–167.0] | 105.6 [103.8–105.6] | 163.4 [163.4–231.6] |
| Peak memory (PSS), live change to the JPEG, 1× 4K (MiB) | 100.6 [99.2–101.2] | 96.8 [96.1–97.3] | 291.7 [283.7–294.4] | n/a | n/a | n/a | 165.2 [163.8–232.5] |
| Peak memory (RSS), JPEG at start-up, 1× 4K (MiB) *(not gated)* | 88.1 [87.7–88.7] | 87.4 [86.3–88.5] | 279.4 [278.2–280.6] | 185.4 [184.5–186.1] | 168.2 [167.7–168.2] | 106.6 [106.1–106.7] | 232.6 [229.9–232.6] |
| Peak memory (RSS), live change to the JPEG, 1× 4K (MiB) *(not gated)* | 119.7 [119.6–120.1] | 118.9 [118.0–119.1] | 310.0 [306.8–312.4] | n/a | n/a | n/a | 233.4 [230.8–233.5] |
| Set: latency to the JPEG (ms) | 279 [278–298] | 212 [210–221] | 324 [319–333] | n/a | n/a | n/a | 310 [282–311] |
| Set: CPU for the JPEG (ms) | 274 [273–282] | 206 [206–213] | 279 [277–284] | n/a | n/a | n/a | **241 [240–243]** (beats scootbg) |
| Set: latency to a color (ms) | 24.7 [20.2–32.2] | n/a | 63.5 [60.2–85.3] | n/a | n/a | n/a | n/a |
| Set: CPU for a color (ms) | 2.0 [1.8–2.2] | n/a | 48.0 [42.7–54.0] | n/a | n/a | n/a | n/a |
| Startup: to a color on screen (ms) | 25.3 [17.5–31.5] | n/a | 59.7 [54.5–61.6] | n/a | 25.5 [20.1–31.7] | n/a | n/a |
| Startup: CPU, color (ms) | 2.7 [2.2–3.3] | n/a | 38.8 [33.9–39.8] | n/a | 4.4 [2.7–5.4] | n/a | n/a |
| Startup: to the JPEG on screen (ms) | 304 [297–317] | 238 [226–253] | 322 [313–324] | 411 [404–451] | 339 [330–349] | 290 [283–292] | 390 [372–412] |
| Startup: CPU, JPEG (ms) | 283 [281–295] | 217 [215–233] | 266 [259–270] | 254 [242–270] | 316 [312–317] | 267 [265–269] | 297 [273–306] |
| Restore: to the JPEG on screen (ms) | 323 [314–324] | n/a | 326 [314–332] | n/a | n/a | n/a | n/a |
| Restore: CPU, JPEG (ms) | 289 [280–291] | n/a | **261 [257–267]** (beats scootbg) | n/a | n/a | n/a | n/a |
| Restore: to a color on screen (ms) | 31.5 [27.9–36.8] | n/a | n/a | n/a | n/a | n/a | n/a |
| Restore: CPU, color (ms) | 0.6 [0.5–0.6] | n/a | n/a | n/a | n/a | n/a | n/a |

Gate: 11 loss(es), 64 win(s), 34 tie(s) for scootbg.
- LOSS: Idle RSS above the floor, 1× 1080p, image: awww 2.38 against scootbg 4.16 (margin 0.21)
- LOSS: Idle PSS above the floor, 1× 1080p, image: awww 0.92 against scootbg 2.52 (margin 0.13)
- LOSS: Idle total with the floor (PSS above it + the floor + compositor copies), 1× 1080p, image: awww 8.84 against scootbg 10.44 (margin 0.52)
- LOSS: Idle RSS above the floor, 2× 4K, image: awww 2.38 against scootbg 4.06 (margin 0.20)
- LOSS: Idle PSS above the floor, 2× 4K, image: awww 0.92 against scootbg 2.41 (margin 0.12)
- LOSS: Idle RSS above the floor, 1× 1080p, color: awww 2.16 against scootbg 3.67 (margin 0.18)
- LOSS: Idle PSS above the floor, 1× 1080p, color: awww 0.97 against scootbg 2.25 (margin 0.11)
- LOSS: Idle RSS above the floor, 2× 4K, color: awww 2.17 against scootbg 3.67 (margin 0.18)
- LOSS: Idle PSS above the floor, 2× 4K, color: awww 0.97 against scootbg 2.25 (margin 0.11)
- LOSS: Set: CPU for the JPEG: wpaperd 241.44 against scootbg 273.56 (margin 13.68)
- LOSS: Restore: CPU, JPEG: awww 260.71 against scootbg 289.46 (margin 21.02)
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
- tie: Peak memory (PSS), live change to the JPEG, 1× 4K: wpaperd
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

5 failed run(s):
- awww idle color round 1: ['awww: a client failed (1)']
- awww idle color round 3: ['awww: a client failed (1)']
- awww idle color round 4: ['awww: a client failed (1)']
- awww idle color round 4: ['awww: a client failed (1)']
- awww idle color round 5: ['awww: a client failed (1)']
