| Row | scootbg | scootbg-bilinear | awww | hyprpaper | swaybg | wbg | wpaperd |
|---|---|---|---|---|---|---|---|
| Size: stripped binaries + non-glibc `ldd` closure (bytes) | 1,866,680 | n/a | 9,080,536 | did not run | 13,459,384 | 13,303,784 | 15,287,664 |
| Disk: installed with its non-glibc closure, plus what it writes (bytes) | 12,172,256 | n/a | 33,294,797 | did not run | 94,586,520 | 57,149,648 | 35,294,647 |
| Peak memory (PSS), JPEG at start-up, 1× 4K (MiB) | 85.3 [84.2–85.4] | 84.6 [82.5–85.4] | 278.9 [278.4–279.1] | did not run | 160.1 [153.4–166.2] | 105.4 [104.1–105.6] | 418.7 [418.6–418.9] |
| Peak memory (PSS), live change to the JPEG, 1× 4K (MiB) | 116.7 [116.4–117.5] | 116.1 [114.6–116.3] | 310.7 [308.7–311.0] | did not run | n/a | n/a | 547.2 [546.6–548.2] |
| Peak memory (RSS), JPEG at start-up, 1× 4K (MiB) *(not gated)* | 89.0 [88.8–89.4] | 88.5 [87.9–89.1] | 281.8 [281.1–282.2] | did not run | 169.4 [169.3–169.5] | 107.4 [107.4–107.5] | 420.8 [420.6–420.9] |
| Peak memory (RSS), live change to the JPEG, 1× 4K (MiB) *(not gated)* | 121.0 [121.0–121.4] | 119.7 [119.4–120.9] | 313.8 [313.6–313.9] | did not run | n/a | n/a | 549.3 [548.6–550.2] |
| Set: latency to the JPEG (ms) | 471 [456–501] | 385 [365–392] | 632 [617–653] | did not run | n/a | n/a | 707 [692–750] |
| Set: CPU for the JPEG (ms) | 448 [444–477] | 367 [357–379] | 565 [562–580] | did not run | n/a | n/a | 878 [835–886] |
| Set: latency to a color (ms) | 17.8 [16.4–22.0] | n/a | 56.1 [47.5–76.7] | did not run | n/a | n/a | n/a |
| Set: CPU for a color (ms) | 3.5 [3.2–3.8] | n/a | 36.1 [35.5–46.7] | did not run | n/a | n/a | n/a |
| Startup: to a color on screen (ms) | 25.8 [18.1–39.0] | n/a | 72.2 [66.0–92.7] | did not run | 26.2 [20.5–33.4] | n/a | n/a |
| Startup: CPU, color (ms) | 4.9 [3.7–5.9] | n/a | 50.7 [48.9–52.0] | did not run | 5.7 [5.4–7.6] | n/a | n/a |
| Startup: to the JPEG on screen (ms) | 513 [492–542] | 396 [369–465] | 633 [608–671] | did not run | 548 [534–591] | 445 [440–480] | 983 [971–1110] |
| Startup: CPU, JPEG (ms) | 473 [462–506] | 363 [354–404] | 568 [549–594] | did not run | 529 [517–570] | 424 [422–464] | 1529 [1485–1662] |

Gate: 0 loss(es), 25 win(s), 7 tie(s) for scootbg.
- tie: Startup: to a color on screen: awww
- tie: Startup: to a color on screen: swaybg
- tie: Startup: CPU, color: swaybg
- tie: Startup: to the JPEG on screen: swaybg
- tie: Startup: to the JPEG on screen: wbg
- tie: Startup: CPU, JPEG: swaybg
- tie: Startup: CPU, JPEG: wbg
