| Row | baseline | now | verdict |
|---|---|---|---|
| Idle RSS, 1× 1080p, image | 11.9 [11.9–11.9] | 12.1 [12.1–12.1] | same |
| Idle PSS, 1× 1080p, image | 6.3 [6.3–6.3] | 6.5 [6.5–6.5] | same |
| Idle floor (the buffers the compositor maps), 1× 1080p, image | 7.9 | 7.9 | same |
| Idle RSS above the floor, 1× 1080p, image | 4.0 [4.0–4.0] | 4.2 [4.2–4.2] | same |
| Idle PSS above the floor, 1× 1080p, image | 2.3 [2.3–2.4] | 2.5 [2.5–2.5] | **REGRESSED** |
| Idle total with the floor (PSS above it + the floor + compositor copies), 1× 1080p, image | 10.3 [10.3–10.3] | 10.4 [10.4–10.4] | same |
| Idle RSS, 2× 4K, image | 35.5 [35.5–35.5] | 35.7 [35.7–35.7] | same |
| Idle PSS, 2× 4K, image | 18.1 [18.1–18.1] | 18.2 [18.2–18.3] | same |
| Idle floor (the buffers the compositor maps), 2× 4K, image | 31.6 | 31.6 | same |
| Idle RSS above the floor, 2× 4K, image | 3.9 [3.9–3.9] | 4.1 [4.1–4.1] | same |
| Idle PSS above the floor, 2× 4K, image | 2.3 [2.2–2.3] | 2.4 [2.4–2.4] | **REGRESSED** |
| Idle total with the floor (PSS above it + the floor + compositor copies), 2× 4K, image | 33.9 [33.9–33.9] | 34.1 [34.0–34.1] | same |
| Idle RSS, 1× 1080p, color | 3.5 | 3.7 [3.7–3.7] | same |
| Idle PSS, 1× 1080p, color | 2.1 [2.1–2.1] | 2.2 [2.2–2.3] | **REGRESSED** |
| Idle floor (the buffers the compositor maps), 1× 1080p, color | 0.0 | 0.0 | same |
| Idle RSS above the floor, 1× 1080p, color | 3.5 | 3.7 [3.7–3.7] | same |
| Idle PSS above the floor, 1× 1080p, color | 2.1 [2.1–2.1] | 2.2 [2.2–2.3] | **REGRESSED** |
| Idle total with the floor (PSS above it + the floor + compositor copies), 1× 1080p, color | 2.1 [2.1–2.1] | 2.2 [2.2–2.3] | **REGRESSED** |
| Idle RSS, 2× 4K, color | 3.5 [3.5–3.5] | 3.7 [3.7–3.7] | same |
| Idle PSS, 2× 4K, color | 2.1 [2.1–2.1] | 2.2 [2.2–2.3] | **REGRESSED** |
| Idle floor (the buffers the compositor maps), 2× 4K, color | 0.0 | 0.0 | same |
| Idle RSS above the floor, 2× 4K, color | 3.5 [3.5–3.5] | 3.7 [3.7–3.7] | same |
| Idle PSS above the floor, 2× 4K, color | 2.1 [2.1–2.1] | 2.2 [2.2–2.3] | **REGRESSED** |
| Idle total with the floor (PSS above it + the floor + compositor copies), 2× 4K, color | 2.1 [2.1–2.1] | 2.2 [2.2–2.3] | **REGRESSED** |
| Idle wakeups in 60 s, 1× 1080p, image | 0 | 0 | same |
| Idle wakeups in 60 s, 2× 4K, image | 0 | 0 | same |
| Idle wakeups in 60 s, 1× 1080p, color | 0 | 0 | same |
| Idle wakeups in 60 s, 2× 4K, color | 0 | 0 | same |
| Idle CPU in 60 s, 1× 1080p, image | 0.0 | 0.0 | same |
| Idle CPU in 60 s, 2× 4K, image | 0.0 | 0.0 | same |
| Idle CPU in 60 s, 1× 1080p, color | 0.0 | 0.0 | same |
| Idle CPU in 60 s, 2× 4K, color | 0.0 | 0.0 | same |
| Peak memory (PSS), JPEG at start-up, 1× 4K | 85.3 [83.2–85.4] | 84.3 [83.3–84.9] | same |
| Peak memory (PSS), live change to the JPEG, 1× 4K | 100.5 [99.7–101.2] | 100.6 [99.2–101.2] | same |
| Peak memory (RSS), JPEG at start-up, 1× 4K | 88.1 [87.5–88.4] | 88.1 [87.7–88.7] | same |
| Peak memory (RSS), live change to the JPEG, 1× 4K | 120.0 [119.4–120.5] | 119.7 [119.6–120.1] | same |
| Set: latency to the JPEG | 290 [279–305] | 279 [278–298] | same |
| Set: CPU for the JPEG | 281 [274–297] | 274 [273–282] | same |
| Set: latency to a color | 15.4 [11.8–27.2] | 24.7 [20.2–32.2] | same |
| Set: CPU for a color | 1.7 [1.2–1.9] | 2.0 [1.8–2.2] | same |
| Startup: to a color on screen | 27.2 [22.9–34.2] | 25.3 [17.5–31.5] | same |
| Startup: CPU, color | 4.0 [3.3–4.5] | 2.7 [2.2–3.3] | same |
| Startup: to the JPEG on screen | 310 [300–322] | 304 [297–317] | same |
| Startup: CPU, JPEG | 283 [280–300] | 283 [281–295] | same |
| Restore: to the JPEG on screen | 324 [310–337] | 323 [314–324] | same |
| Restore: CPU, JPEG | 287 [281–295] | 289 [280–291] | same |
| Restore: to a color on screen | 29.2 [28.1–30.5] | 31.5 [27.9–36.8] | same |
| Restore: CPU, color | 0.5 [0.5–0.6] | 0.6 [0.5–0.6] | same |
| Size: stripped binaries + non-glibc `ldd` closure | 1,841,928 | 1,907,464 | same |
| Disk: installed with its non-glibc closure, plus what it writes | 34,858,673 | 34,989,745 | same |

8 regression(s) beyond the margin.
