| Row | scootbg | scootbg-bilinear | awww | hyprpaper | swaybg | wbg | wpaperd |
|---|---|---|---|---|---|---|---|
| Size: stripped binaries + non-glibc `ldd` closure (bytes) | 1,907,464 | n/a | 7,421,368 | 52,574,976 | 14,323,368 | 14,732,512 | 14,586,640 |
| Disk: installed with its non-glibc closure, plus what it writes (bytes) | 34,924,209 | n/a | 35,132,158 | 176,474,512 | 101,231,856 | 62,619,128 | 39,563,560 |
| Idle RSS, 1× 1080p, image (MiB) *(not gated)* | 12.0 [12.0–12.1] | n/a | 10.3 [10.3–10.3] | 86.6 [86.5–86.7] | 8.9 [8.8–8.9] | 14.6 [14.5–14.6] | 72.8 [72.6–72.9] |
| Idle PSS, 1× 1080p, image (MiB) *(not gated)* | 6.4 [6.4–6.5] | n/a | 4.9 [4.9–4.9] | 59.3 [59.3–59.4] | 4.1 [4.1–4.1] | 7.0 [7.0–7.0] | 49.2 [49.0–49.3] |
| Idle floor (the buffers the compositor maps), 1× 1080p, image (MiB) *(not gated)* | 7.9 | n/a | 7.9 | 0.0 | 7.9 | 7.9 | 0.0 |
| Idle RSS above the floor, 1× 1080p, image (MiB) | 4.1 [4.1–4.1] | n/a | **2.4 [2.3–2.4]** (beats scootbg) | 86.6 [86.5–86.7] | 8.9 [8.8–8.9] | 6.6 [6.6–6.6] | 72.8 [72.6–72.9] |
| Idle PSS above the floor, 1× 1080p, image (MiB) | 2.5 [2.5–2.5] | n/a | **0.9 [0.9–0.9]** (beats scootbg) | 59.3 [59.3–59.4] | 4.1 [4.1–4.1] | 3.1 [3.1–3.1] | 49.2 [49.0–49.3] |
| Idle total with the floor (PSS above it + the floor + compositor copies), 1× 1080p, image (MiB) | 10.4 [10.4–10.4] | n/a | **8.8 [8.8–8.8]** (beats scootbg) | 59.3 [59.3–59.4] | 12.0 [12.0–12.1] | 11.0 [11.0–11.0] | 49.2 [49.0–49.3] |
| Idle RSS, 2× 4K, image (MiB) *(not gated)* | 35.7 [35.6–35.7] | n/a | 65.7 | 86.5 [86.5–86.6] | 8.9 [8.9–8.9] | 69.9 [69.9–69.9] | 75.8 [75.8–75.8] |
| Idle PSS, 2× 4K, image (MiB) *(not gated)* | 18.2 [18.2–18.2] | n/a | 32.6 | 59.3 [59.3–59.3] | 4.1 [4.1–4.1] | 34.7 [34.7–34.7] | 52.1 [52.1–52.1] |
| Idle floor (the buffers the compositor maps), 2× 4K, image (MiB) *(not gated)* | 31.6 | n/a | 63.3 | 0.0 | 63.3 | 63.3 | 0.0 |
| Idle RSS above the floor, 2× 4K, image (MiB) | 4.0 [4.0–4.0] | n/a | **2.4** (beats scootbg) | 86.5 [86.5–86.6] | 8.9 [8.9–8.9] | 6.6 [6.6–6.6] | 75.8 [75.8–75.8] |
| Idle PSS above the floor, 2× 4K, image (MiB) | 2.4 [2.4–2.4] | n/a | **0.9** (beats scootbg) | 59.3 [59.3–59.3] | 4.1 [4.1–4.1] | 3.1 [3.1–3.1] | 52.1 [52.1–52.1] |
| Idle total with the floor (PSS above it + the floor + compositor copies), 2× 4K, image (MiB) | 34.0 [34.0–34.0] | n/a | 64.2 | 59.3 [59.3–59.3] | 67.4 [67.4–67.4] | 66.3 [66.3–66.4] | 52.1 [52.1–52.1] |
| Idle RSS, 1× 1080p, color (MiB) *(not gated)* | 3.6 [3.5–3.6] | n/a | 10.1 [10.1–10.1] | n/a | 6.8 [6.8–6.8] | n/a | n/a |
| Idle PSS, 1× 1080p, color (MiB) *(not gated)* | 2.2 [2.1–2.2] | n/a | 4.9 [4.9–4.9] | n/a | 4.2 [4.1–4.2] | n/a | n/a |
| Idle floor (the buffers the compositor maps), 1× 1080p, color (MiB) *(not gated)* | 0.0 | n/a | 7.9 | n/a | 0.0 | n/a | n/a |
| Idle RSS above the floor, 1× 1080p, color (MiB) | 3.6 [3.5–3.6] | n/a | **2.2 [2.2–2.2]** (beats scootbg) | n/a | 6.8 [6.8–6.8] | n/a | n/a |
| Idle PSS above the floor, 1× 1080p, color (MiB) | 2.2 [2.1–2.2] | n/a | **1.0 [1.0–1.0]** (beats scootbg) | n/a | 4.2 [4.1–4.2] | n/a | n/a |
| Idle total with the floor (PSS above it + the floor + compositor copies), 1× 1080p, color (MiB) | 2.2 [2.1–2.2] | n/a | 8.9 [8.9–8.9] | n/a | 4.2 [4.1–4.2] | n/a | n/a |
| Idle RSS, 2× 4K, color (MiB) *(not gated)* | 3.6 [3.5–3.6] | n/a | 65.5 | n/a | 6.8 [6.8–6.8] | n/a | n/a |
| Idle PSS, 2× 4K, color (MiB) *(not gated)* | 2.2 [2.1–2.2] | n/a | 32.6 | n/a | 4.2 [4.2–4.2] | n/a | n/a |
| Idle floor (the buffers the compositor maps), 2× 4K, color (MiB) *(not gated)* | 0.0 | n/a | 63.3 | n/a | 0.0 | n/a | n/a |
| Idle RSS above the floor, 2× 4K, color (MiB) | 3.6 [3.5–3.6] | n/a | **2.2** (beats scootbg) | n/a | 6.8 [6.8–6.8] | n/a | n/a |
| Idle PSS above the floor, 2× 4K, color (MiB) | 2.2 [2.1–2.2] | n/a | **1.0** (beats scootbg) | n/a | 4.2 [4.2–4.2] | n/a | n/a |
| Idle total with the floor (PSS above it + the floor + compositor copies), 2× 4K, color (MiB) | 2.2 [2.1–2.2] | n/a | 64.3 | n/a | 4.2 [4.2–4.2] | n/a | n/a |
| Idle wakeups in 60 s, 1× 1080p, image | 0 | n/a | 0 | 48 [47–48] | 0 | 0 | 122 |
| Idle wakeups in 60 s, 2× 4K, image | 0 | n/a | 0 | 49 [48–51] | 0 | 0 | 136 [132–137] |
| Idle wakeups in 60 s, 1× 1080p, color | 0 | n/a | 0 | n/a | 0 | n/a | n/a |
| Idle wakeups in 60 s, 2× 4K, color | 0 | n/a | 0 | n/a | 0 | n/a | n/a |
| Idle CPU in 60 s, 1× 1080p, image (ms) | 0.0 | n/a | 0.0 | 0.5 [0.5–0.9] | 0.0 | 0.0 | 0.5 [0.4–1.3] |
| Idle CPU in 60 s, 2× 4K, image (ms) | 0.0 | n/a | 0.0 | 0.6 [0.5–0.6] | 0.0 | 0.0 | 0.7 [0.6–0.7] |
| Idle CPU in 60 s, 1× 1080p, color (ms) | 0.0 | n/a | 0.0 | n/a | 0.0 | n/a | n/a |
| Idle CPU in 60 s, 2× 4K, color (ms) | 0.0 | n/a | 0.0 | n/a | 0.0 | n/a | n/a |
| Peak memory (PSS), JPEG at start-up, 1× 4K (MiB) | 84.9 [83.6–85.6] | 80.7 [80.5–84.0] | 278.5 [265.7–278.6] | 176.6 [175.6–182.0] | 156.4 [130.4–165.0] | 103.6 [103.5–105.0] | 194.1 [163.4–231.7] |
| Peak memory (PSS), live change to the JPEG, 1× 4K (MiB) | 101.5 [101.1–101.7] | 99.8 [96.4–101.3] | 291.1 [290.7–294.4] | n/a | n/a | n/a | 182.1 [163.7–213.6] |
| Peak memory (RSS), JPEG at start-up, 1× 4K (MiB) *(not gated)* | 88.0 [87.6–88.7] | 86.8 [86.2–86.9] | 280.5 [278.2–280.5] | 184.6 [184.2–186.0] | 168.2 [168.2–168.2] | 106.2 [106.1–106.7] | 230.5 [229.8–232.7] |
| Peak memory (RSS), live change to the JPEG, 1× 4K (MiB) *(not gated)* | 120.3 [120.1–120.5] | 118.3 [117.9–119.9] | 312.4 [312.4–312.4] | n/a | n/a | n/a | 233.3 [230.6–233.5] |
| Set: latency to the JPEG (ms) | 299 [289–308] | 233 [215–259] | 325 [318–342] | n/a | n/a | n/a | 299 [285–319] |
| Set: CPU for the JPEG (ms) | 282 [282–286] | 215 [209–224] | 276 [271–283] | n/a | n/a | n/a | **234 [220–241]** (beats scootbg) |
| Set: latency to a color (ms) | 34.4 [14.3–48.5] | n/a | 87.2 [67.9–96.2] | n/a | n/a | n/a | n/a |
| Set: CPU for a color (ms) | 1.8 [1.5–2.3] | n/a | 58.3 [50.0–62.4] | n/a | n/a | n/a | n/a |
| Startup: to a color on screen (ms) | 24.4 [21.7–35.3] | n/a | 57.6 [53.1–78.0] | n/a | 28.0 [22.5–32.7] | n/a | n/a |
| Startup: CPU, color (ms) | 2.6 [2.3–3.7] | n/a | 36.3 [34.8–45.2] | n/a | 4.6 [3.8–4.9] | n/a | n/a |
| Startup: to the JPEG on screen (ms) | 305 [301–333] | 247 [244–251] | 326 [311–341] | 437 [400–732] | 340 [322–353] | 287 [283–288] | 381 [358–407] |
| Startup: CPU, JPEG (ms) | 285 [283–300] | 228 [209–230] | 259 [257–273] | 256 [240–421] | 318 [309–322] | 265 [263–266] | 286 [267–299] |
| Restore: to the JPEG on screen (ms) | 324 [318–339] | n/a | 327 [314–336] | n/a | n/a | n/a | n/a |
| Restore: CPU, JPEG (ms) | 291 [280–297] | n/a | **259 [255–266]** (beats scootbg) | n/a | n/a | n/a | n/a |
| Restore: to a color on screen (ms) | 31.7 [27.2–43.7] | n/a | n/a | n/a | n/a | n/a | n/a |
| Restore: CPU, color (ms) | 0.7 [0.6–0.7] | n/a | n/a | n/a | n/a | n/a | n/a |

Gate: 11 loss(es), 59 win(s), 39 tie(s) for scootbg.
- LOSS: Idle RSS above the floor, 1× 1080p, image: awww 2.37 against scootbg 4.12 (margin 0.21)
- LOSS: Idle PSS above the floor, 1× 1080p, image: awww 0.91 against scootbg 2.49 (margin 0.12)
- LOSS: Idle total with the floor (PSS above it + the floor + compositor copies), 1× 1080p, image: awww 8.83 against scootbg 10.41 (margin 0.52)
- LOSS: Idle RSS above the floor, 2× 4K, image: awww 2.38 against scootbg 4.02 (margin 0.20)
- LOSS: Idle PSS above the floor, 2× 4K, image: awww 0.92 against scootbg 2.38 (margin 0.12)
- LOSS: Idle RSS above the floor, 1× 1080p, color: awww 2.17 against scootbg 3.56 (margin 0.18)
- LOSS: Idle PSS above the floor, 1× 1080p, color: awww 0.97 against scootbg 2.15 (margin 0.11)
- LOSS: Idle RSS above the floor, 2× 4K, color: awww 2.17 against scootbg 3.56 (margin 0.18)
- LOSS: Idle PSS above the floor, 2× 4K, color: awww 0.97 against scootbg 2.15 (margin 0.11)
- LOSS: Set: CPU for the JPEG: wpaperd 233.51 against scootbg 282.45 (margin 25.20)
- LOSS: Restore: CPU, JPEG: awww 259.45 against scootbg 291.10 (margin 27.87)
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
- tie: Idle CPU in 60 s, 1× 1080p, image: wpaperd
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
- tie: Set: latency to a color: awww
- tie: Startup: to a color on screen: awww
- tie: Startup: to a color on screen: swaybg
- tie: Startup: CPU, color: swaybg
- tie: Startup: to the JPEG on screen: awww
- tie: Startup: to the JPEG on screen: hyprpaper
- tie: Startup: to the JPEG on screen: swaybg
- tie: Startup: to the JPEG on screen: wbg
- tie: Startup: to the JPEG on screen: wpaperd
- tie: Startup: CPU, JPEG: awww
- tie: Startup: CPU, JPEG: hyprpaper
- tie: Startup: CPU, JPEG: wbg
- tie: Startup: CPU, JPEG: wpaperd
- tie: Restore: to the JPEG on screen: awww

8 failed run(s):
- awww idle image round 1: ['awww: a client failed (1)']
- awww idle color round 1: ['awww: a client failed (1)']
- awww idle color round 2: ['awww: a client failed (1)']
- awww idle image round 3: ['awww: a client failed (1)']
- awww idle image round 3: ['awww: a client failed (1)']
- awww idle color round 3: ['awww: a client failed (1)']
- awww idle color round 4: ['awww: a client failed (1)']
- awww idle color round 4: ['awww: a client failed (1)']
