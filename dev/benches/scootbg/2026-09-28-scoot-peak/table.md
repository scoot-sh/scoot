| Row | scootbg | scootbg-bilinear | awww | hyprpaper | swaybg | wbg | wpaperd |
|---|---|---|---|---|---|---|---|
| Size: stripped binaries + non-glibc `ldd` closure (bytes) | 1,866,680 | n/a | 9,080,536 | did not run | 13,459,384 | 13,303,784 | 15,287,664 |
| Disk: installed with its non-glibc closure, plus what it writes (bytes) | 12,172,256 | n/a | 33,294,797 | did not run | 94,586,520 | 57,149,648 | 35,294,647 |
| Peak memory (PSS), JPEG at start-up, 1× 4K (MiB) | 84.7 [84.2–85.6] | 83.2 [82.6–84.4] | 278.9 [275.8–279.4] | did not run | 159.4 [153.6–165.8] | 106.0 [105.0–106.1] | 419.0 [418.8–419.1] |
| Peak memory (PSS), live change to the JPEG, 1× 4K (MiB) | 101.3 [100.4–101.4] | 98.8 [98.5–101.4] | 294.9 [290.8–295.2] | did not run | n/a | n/a | 499.5 [499.4–499.6] |
| Peak memory (RSS), JPEG at start-up, 1× 4K (MiB) *(not gated)* | 89.1 [88.9–89.2] | 88.1 [87.9–89.1] | 281.9 [281.7–282.2] | did not run | 169.4 [169.2–169.5] | 107.4 [107.4–107.5] | 420.3 [420.1–420.4] |
| Peak memory (RSS), live change to the JPEG, 1× 4K (MiB) *(not gated)* | 120.8 [120.8–121.3] | 120.1 [119.6–120.7] | 313.8 [313.6–313.9] | did not run | n/a | n/a | 548.3 [548.2–548.4] |
| Set: latency to the JPEG (ms) | 479 [460–517] | 397 [374–416] | 608 [598–669] | did not run | n/a | n/a | 706 [678–744] |
| Set: CPU for the JPEG (ms) | 448 [446–496] | 369 [359–386] | 562 [554–624] | did not run | n/a | n/a | 839 [803–868] |
| Set: latency to a color (ms) | 18.0 [15.2–23.3] | n/a | 54.0 [44.1–60.7] | did not run | n/a | n/a | n/a |
| Set: CPU for a color (ms) | 3.8 [3.5–4.1] | n/a | 36.3 [34.7–40.1] | did not run | n/a | n/a | n/a |
| Startup: to a color on screen (ms) | 29.5 [19.2–30.9] | n/a | 73.2 [66.6–81.3] | did not run | 25.4 [20.6–32.1] | n/a | n/a |
| Startup: CPU, color (ms) | 5.0 [4.2–5.3] | n/a | 50.0 [48.8–57.3] | did not run | 5.5 [5.4–7.4] | n/a | n/a |
| Startup: to the JPEG on screen (ms) | 494 [477–531] | 382 [377–393] | 626 [604–645] | did not run | 554 [531–612] | 451 [444–469] | 988 [974–1022] |
| Startup: CPU, JPEG (ms) | 475 [450–504] | 363 [355–364] | 558 [549–578] | did not run | 522 [515–579] | 426 [418–445] | 1466 [1434–1515] |

Gate: 0 loss(es), 25 win(s), 7 tie(s) for scootbg.
- tie: Set: CPU for the JPEG: awww
- tie: Startup: to a color on screen: swaybg
- tie: Startup: CPU, color: swaybg
- tie: Startup: to the JPEG on screen: swaybg
- tie: Startup: to the JPEG on screen: wbg
- tie: Startup: CPU, JPEG: swaybg
- tie: Startup: CPU, JPEG: wbg
