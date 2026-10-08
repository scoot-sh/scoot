| Row | baseline | now | verdict |
|---|---|---|---|
| Idle RSS, 1× 1080p, image | 11.9 [11.9–11.9] | 12.0 [12.0–12.1] | same |
| Idle PSS, 1× 1080p, image | 6.3 [6.3–6.3] | 6.4 [6.4–6.5] | same |
| Idle floor (the buffers the compositor maps), 1× 1080p, image | 7.9 | 7.9 | same |
| Idle RSS above the floor, 1× 1080p, image | 4.0 [4.0–4.0] | 4.1 [4.1–4.1] | same |
| Idle PSS above the floor, 1× 1080p, image | 2.3 [2.3–2.4] | 2.5 [2.5–2.5] | **REGRESSED** |
| Idle total with the floor (PSS above it + the floor + compositor copies), 1× 1080p, image | 10.3 [10.3–10.3] | 10.4 [10.4–10.4] | same |
| Idle RSS, 2× 4K, image | 35.5 [35.5–35.5] | 35.7 [35.6–35.7] | same |
| Idle PSS, 2× 4K, image | 18.1 [18.1–18.1] | 18.2 [18.2–18.2] | same |
| Idle floor (the buffers the compositor maps), 2× 4K, image | 31.6 | 31.6 | same |
| Idle RSS above the floor, 2× 4K, image | 3.9 [3.9–3.9] | 4.0 [4.0–4.0] | same |
| Idle PSS above the floor, 2× 4K, image | 2.3 [2.2–2.3] | 2.4 [2.4–2.4] | **REGRESSED** |
| Idle total with the floor (PSS above it + the floor + compositor copies), 2× 4K, image | 33.9 [33.9–33.9] | 34.0 [34.0–34.0] | same |
| Idle RSS, 1× 1080p, color | 3.5 | 3.6 [3.5–3.6] | same |
| Idle PSS, 1× 1080p, color | 2.1 [2.1–2.1] | 2.2 [2.1–2.2] | same |
| Idle floor (the buffers the compositor maps), 1× 1080p, color | 0.0 | 0.0 | same |
| Idle RSS above the floor, 1× 1080p, color | 3.5 | 3.6 [3.5–3.6] | same |
| Idle PSS above the floor, 1× 1080p, color | 2.1 [2.1–2.1] | 2.2 [2.1–2.2] | same |
| Idle total with the floor (PSS above it + the floor + compositor copies), 1× 1080p, color | 2.1 [2.1–2.1] | 2.2 [2.1–2.2] | same |
| Idle RSS, 2× 4K, color | 3.5 [3.5–3.5] | 3.6 [3.5–3.6] | same |
| Idle PSS, 2× 4K, color | 2.1 [2.1–2.1] | 2.2 [2.1–2.2] | same |
| Idle floor (the buffers the compositor maps), 2× 4K, color | 0.0 | 0.0 | same |
| Idle RSS above the floor, 2× 4K, color | 3.5 [3.5–3.5] | 3.6 [3.5–3.6] | same |
| Idle PSS above the floor, 2× 4K, color | 2.1 [2.1–2.1] | 2.2 [2.1–2.2] | same |
| Idle total with the floor (PSS above it + the floor + compositor copies), 2× 4K, color | 2.1 [2.1–2.1] | 2.2 [2.1–2.2] | same |
| Idle wakeups in 60 s, 1× 1080p, image | 0 | 0 | same |
| Idle wakeups in 60 s, 2× 4K, image | 0 | 0 | same |
| Idle wakeups in 60 s, 1× 1080p, color | 0 | 0 | same |
| Idle wakeups in 60 s, 2× 4K, color | 0 | 0 | same |
| Idle CPU in 60 s, 1× 1080p, image | 0.0 | 0.0 | same |
| Idle CPU in 60 s, 2× 4K, image | 0.0 | 0.0 | same |
| Idle CPU in 60 s, 1× 1080p, color | 0.0 | 0.0 | same |
| Idle CPU in 60 s, 2× 4K, color | 0.0 | 0.0 | same |
| Peak memory (PSS), JPEG at start-up, 1× 4K | 85.3 [83.2–85.4] | 84.9 [83.6–85.6] | same |
| Peak memory (PSS), live change to the JPEG, 1× 4K | 100.5 [99.7–101.2] | 101.5 [101.1–101.7] | same |
| Peak memory (RSS), JPEG at start-up, 1× 4K | 88.1 [87.5–88.4] | 88.0 [87.6–88.7] | same |
| Peak memory (RSS), live change to the JPEG, 1× 4K | 120.0 [119.4–120.5] | 120.3 [120.1–120.5] | same |
| Set: latency to the JPEG | 290 [279–305] | 299 [289–308] | same |
| Set: CPU for the JPEG | 281 [274–297] | 282 [282–286] | same |
| Set: latency to a color | 15.4 [11.8–27.2] | 34.4 [14.3–48.5] | same |
| Set: CPU for a color | 1.7 [1.2–1.9] | 1.8 [1.5–2.3] | same |
| Startup: to a color on screen | 27.2 [22.9–34.2] | 24.4 [21.7–35.3] | same |
| Startup: CPU, color | 4.0 [3.3–4.5] | 2.6 [2.3–3.7] | same |
| Startup: to the JPEG on screen | 310 [300–322] | 305 [301–333] | same |
| Startup: CPU, JPEG | 283 [280–300] | 285 [283–300] | same |
| Restore: to the JPEG on screen | 324 [310–337] | 324 [318–339] | same |
| Restore: CPU, JPEG | 287 [281–295] | 291 [280–297] | same |
| Restore: to a color on screen | 29.2 [28.1–30.5] | 31.7 [27.2–43.7] | same |
| Restore: CPU, color | 0.5 [0.5–0.6] | 0.7 [0.6–0.7] | same |
| Size: stripped binaries + non-glibc `ldd` closure | 1,841,928 | 1,907,464 | same |
| Disk: installed with its non-glibc closure, plus what it writes | 34,858,673 | 34,924,209 | same |

2 regression(s) beyond the margin.
