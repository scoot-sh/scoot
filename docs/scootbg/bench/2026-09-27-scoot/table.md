| Row | scootbg | scootbg-bilinear | awww | hyprpaper | swaybg | wbg | wpaperd |
|---|---|---|---|---|---|---|---|
| Size: stripped binaries + non-glibc `ldd` closure (bytes) | 1,866,680 | n/a | 9,080,536 | did not run | 13,459,384 | 13,303,784 | 15,287,664 |
| Disk: installed with its non-glibc closure, plus what it writes (bytes) | 12,172,256 | n/a | 33,294,797 | did not run | 94,586,520 | 57,149,648 | 35,294,647 |
| Idle RSS, 1× 1080p, image (MiB) *(not gated)* | 12.3 [12.2–12.3] | n/a | 10.8 [10.7–10.8] | did not run | 9.6 [9.5–9.8] | 15.1 [15.1–15.2] | 269.8 [269.5–269.9] |
| Idle PSS, 1× 1080p, image (MiB) *(not gated)* | 6.3 [6.2–6.3] | n/a | 4.9 [4.9–4.9] | did not run | 5.2 [5.1–5.3] | 7.3 [7.2–7.3] | 254.5 [254.3–254.6] |
| Idle floor (the buffers the compositor maps), 1× 1080p, image (MiB) *(not gated)* | 7.9 | n/a | 7.9 | did not run | 7.9 | 7.9 | 23.7 |
| Idle RSS above the floor, 1× 1080p, image (MiB) | 4.4 [4.3–4.4] | n/a | **2.9 [2.8–2.9]** (beats scootbg) | did not run | 9.6 [9.5–9.8] | 7.2 [7.2–7.3] | 246.1 [245.8–246.1] |
| Idle PSS above the floor, 1× 1080p, image (MiB) | 2.4 [2.3–2.4] | n/a | **1.0 [0.9–1.0]** (beats scootbg) | did not run | 5.2 [5.1–5.3] | 3.3 [3.2–3.4] | 242.7 [242.4–242.7] |
| Idle total with the floor (PSS above it + the floor + compositor copies), 1× 1080p, image (MiB) | 10.3 [10.2–10.3] | n/a | **8.9 [8.8–8.9]** (beats scootbg) | did not run | 13.1 [13.0–13.2] | 11.2 [11.1–11.3] | 266.4 [266.1–266.5] |
| Idle RSS, 2× 4K, image (MiB) *(not gated)* | 36.1 [36.1–36.2] | n/a | 66.2 [66.2–66.2] | did not run | 9.6 [9.5–9.6] | 70.4 [70.4–70.5] | 626.4 [625.7–626.6] |
| Idle PSS, 2× 4K, image (MiB) *(not gated)* | 18.3 [18.2–18.3] | n/a | 32.6 [32.6–32.7] | did not run | 5.2 [5.1–5.2] | 34.9 [34.9–35.0] | 528.1 [527.3–528.2] |
| Idle floor (the buffers the compositor maps), 2× 4K, image (MiB) *(not gated)* | 31.6 | n/a | 63.3 | did not run | 63.3 | 63.3 | 189.8 |
| Idle RSS above the floor, 2× 4K, image (MiB) | 4.5 [4.4–4.5] | n/a | **2.9 [2.9–2.9]** (beats scootbg) | did not run | 9.6 [9.5–9.6] | 7.2 [7.1–7.2] | 436.5 [435.8–436.7] |
| Idle PSS above the floor, 2× 4K, image (MiB) | 2.5 [2.4–2.5] | n/a | **1.0 [0.9–1.0]** (beats scootbg) | did not run | 5.2 [5.1–5.2] | 3.3 [3.2–3.3] | 433.2 [432.4–433.3] |
| Idle total with the floor (PSS above it + the floor + compositor copies), 2× 4K, image (MiB) | 34.1 [34.0–34.1] | n/a | 64.3 [64.2–64.3] | did not run | 68.5 [68.4–68.5] | 66.6 [66.5–66.6] | 623.0 [622.3–623.2] |
| Idle RSS, 1× 1080p, color (MiB) *(not gated)* | 3.8 [3.8–3.9] | n/a | 10.6 [10.5–10.6] | did not run | 7.4 [7.4–7.6] | n/a | n/a |
| Idle PSS, 1× 1080p, color (MiB) *(not gated)* | 2.1 [2.0–2.1] | n/a | 5.0 [4.9–5.0] | did not run | 4.3 [4.3–4.4] | n/a | n/a |
| Idle floor (the buffers the compositor maps), 1× 1080p, color (MiB) *(not gated)* | 0.0 | n/a | 7.9 | did not run | 0.0 | n/a | n/a |
| Idle RSS above the floor, 1× 1080p, color (MiB) | 3.8 [3.8–3.9] | n/a | **2.7 [2.6–2.7]** (beats scootbg) | did not run | 7.4 [7.4–7.6] | n/a | n/a |
| Idle PSS above the floor, 1× 1080p, color (MiB) | 2.1 [2.0–2.1] | n/a | **1.1 [1.0–1.1]** (beats scootbg) | did not run | 4.3 [4.3–4.4] | n/a | n/a |
| Idle total with the floor (PSS above it + the floor + compositor copies), 1× 1080p, color (MiB) | 2.1 [2.0–2.1] | n/a | 9.0 [8.9–9.0] | did not run | 4.3 [4.3–4.4] | n/a | n/a |
| Idle RSS, 2× 4K, color (MiB) *(not gated)* | 3.9 [3.8–3.9] | n/a | 66.0 [66.0–66.0] | did not run | 7.5 [7.4–7.6] | n/a | n/a |
| Idle PSS, 2× 4K, color (MiB) *(not gated)* | 2.1 [2.0–2.1] | n/a | 32.7 [32.7–32.7] | did not run | 4.4 [4.3–4.4] | n/a | n/a |
| Idle floor (the buffers the compositor maps), 2× 4K, color (MiB) *(not gated)* | 0.0 | n/a | 63.3 | did not run | 0.0 | n/a | n/a |
| Idle RSS above the floor, 2× 4K, color (MiB) | 3.9 [3.8–3.9] | n/a | **2.7 [2.7–2.7]** (beats scootbg) | did not run | 7.5 [7.4–7.6] | n/a | n/a |
| Idle PSS above the floor, 2× 4K, color (MiB) | 2.1 [2.0–2.1] | n/a | **1.0 [1.0–1.1]** (beats scootbg) | did not run | 4.4 [4.3–4.4] | n/a | n/a |
| Idle total with the floor (PSS above it + the floor + compositor copies), 2× 4K, color (MiB) | 2.1 [2.0–2.1] | n/a | 64.3 [64.3–64.3] | did not run | 4.4 [4.3–4.4] | n/a | n/a |
| Idle wakeups in 60 s, 1× 1080p, image | 0 | n/a | 0 | did not run | 0 | 0 | 125 [125–126] |
| Idle wakeups in 60 s, 2× 4K, image | 0 | n/a | 0 | did not run | 0 | 0 | 160 [160–161] |
| Idle wakeups in 60 s, 1× 1080p, color | 0 | n/a | 0 | did not run | 0 | n/a | n/a |
| Idle wakeups in 60 s, 2× 4K, color | 0 | n/a | 0 | did not run | 0 | n/a | n/a |
| Idle CPU in 60 s, 1× 1080p, image (ms) | 0.0 | n/a | 0.0 | did not run | 0.0 | 0.0 | 6.2 [5.8–6.6] |
| Idle CPU in 60 s, 2× 4K, image (ms) | 0.0 | n/a | 0.0 | did not run | 0.0 | 0.0 | 7.4 [7.1–7.9] |
| Idle CPU in 60 s, 1× 1080p, color (ms) | 0.0 | n/a | 0.0 | did not run | 0.0 | n/a | n/a |
| Idle CPU in 60 s, 2× 4K, color (ms) | 0.0 | n/a | 0.0 | did not run | 0.0 | n/a | n/a |
| Peak memory (RSS), JPEG at start-up, 1× 4K (MiB) *(not gated)* | 88.9 [88.8–89.2] | 88.5 [87.8–88.6] | 281.7 [281.7–282.1] | did not run | 169.3 [169.3–169.4] | 107.4 [107.3–107.5] | 420.1 [420.0–420.4] |
| Peak memory (RSS), live change to the JPEG, 1× 4K (MiB) *(not gated)* | 121.1 [120.9–121.4] | 120.3 [119.7–120.9] | 313.9 [313.5–314.0] | did not run | n/a | n/a | 548.6 [548.2–549.6] |
| Set: latency to the JPEG (ms) | 472 [447–702] | 382 [373–443] | 637 [610–700] | did not run | n/a | n/a | 702 [668–757] |
| Set: CPU for the JPEG (ms) | 458 [437–683] | 369 [362–421] | 595 [564–643] | did not run | n/a | n/a | 844 [809–913] |
| Set: latency to a color (ms) | 16.7 [13.9–25.4] | n/a | 51.9 [47.8–55.7] | did not run | n/a | n/a | n/a |
| Set: CPU for a color (ms) | 4.3 [3.9–4.4] | n/a | 35.6 [34.1–36.4] | did not run | n/a | n/a | n/a |
| Startup: to a color on screen (ms) | 22.2 [20.4–30.0] | n/a | 76.5 [70.9–86.3] | did not run | 25.0 [21.1–30.0] | n/a | n/a |
| Startup: CPU, color (ms) | 5.8 [4.8–7.6] | n/a | 51.3 [48.5–65.9] | did not run | 6.2 [5.9–8.3] | n/a | n/a |
| Startup: to the JPEG on screen (ms) | 469 [465–498] | 394 [380–405] | 657 [623–675] | did not run | 542 [533–575] | 447 [429–460] | 944 [914–1027] |
| Startup: CPU, JPEG (ms) | 444 [443–462] | 367 [362–384] | 566 [560–602] | did not run | 523 [513–557] | 413 [402–436] | 1451 [1412–1550] |
| Restore: to the JPEG on screen (ms) | 482 [449–522] | n/a | 630 [620–665] | did not run | n/a | n/a | n/a |
| Restore: CPU, JPEG (ms) | 463 [432–484] | n/a | 584 [567–621] | did not run | n/a | n/a | n/a |
| Restore: to a color on screen (ms) | 20.0 [11.9–23.8] | n/a | n/a | did not run | n/a | n/a | n/a |
| Restore: CPU, color (ms) | 2.3 [2.3–3.4] | n/a | n/a | did not run | n/a | n/a | n/a |

Gate: 9 loss(es), 51 win(s), 28 tie(s) for scootbg.
- LOSS: Idle RSS above the floor, 1× 1080p, image: awww 2.86 against scootbg 4.38 (margin 0.22)
- LOSS: Idle PSS above the floor, 1× 1080p, image: awww 0.96 against scootbg 2.35 (margin 0.16)
- LOSS: Idle total with the floor (PSS above it + the floor + compositor copies), 1× 1080p, image: awww 8.87 against scootbg 10.26 (margin 0.51)
- LOSS: Idle RSS above the floor, 2× 4K, image: awww 2.89 against scootbg 4.49 (margin 0.22)
- LOSS: Idle PSS above the floor, 2× 4K, image: awww 1.00 against scootbg 2.47 (margin 0.19)
- LOSS: Idle RSS above the floor, 1× 1080p, color: awww 2.72 against scootbg 3.85 (margin 0.20)
- LOSS: Idle PSS above the floor, 1× 1080p, color: awww 1.05 against scootbg 2.07 (margin 0.16)
- LOSS: Idle RSS above the floor, 2× 4K, color: awww 2.72 against scootbg 3.87 (margin 0.19)
- LOSS: Idle PSS above the floor, 2× 4K, color: awww 1.05 against scootbg 2.07 (margin 0.10)
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
- tie: Startup: to the JPEG on screen: swaybg
- tie: Startup: to the JPEG on screen: wbg
- tie: Startup: CPU, JPEG: wbg
