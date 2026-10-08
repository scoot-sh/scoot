| Row | scootbg | awww |
|---|---|---|
| Size: stripped binaries + non-glibc `ldd` closure (bytes) | 1,841,928 | 7,421,368 |
| Disk: installed with its non-glibc closure, plus what it writes (bytes) | 1,841,928 | 35,132,088 |
| Restore: to the JPEG on screen (ms) | 323 [318–328] | 331 [316–336] |
| Restore: CPU, JPEG (ms) | 289 [280–291] | **264 [255–267]** (beats scootbg) |
| Restore: to a color on screen (ms) | 29.0 [27.8–37.9] | n/a |
| Restore: CPU, color (ms) | 0.6 [0.5–1.3] | n/a |

Gate: 1 loss(es), 2 win(s), 1 tie(s) for scootbg.
- LOSS: Restore: CPU, JPEG: awww 264.18 against scootbg 289.09 (margin 23.64)
- tie: Restore: to the JPEG on screen: awww
