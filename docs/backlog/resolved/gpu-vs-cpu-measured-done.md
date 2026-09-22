---
title: "CPU vs GPU rendering, measured on a real GPU (RESOLVED 2026-09-21)"
status: "resolved"
area: "rendering"
priority: "high"
blocked: null
---

# CPU vs GPU rendering, measured on a real GPU

Filed 2026-09-19 because milestone 6 built a GPU renderer and a GPU scanout
tier and **every performance number attached to either was a software
rasteriser's**. Answered 2026-09-21 on the user's Apple M2 (`apple,t8112`)
under Asahi Linux, `eDP-1` at `2560x1600@60`, `scale = 1.5`, on battery.

Runbook: [`Asahi.md`](../../../Asahi.md)'s Test 4, now runnable as one
command (`scripts/asahi-test4.sh`, which writes its own report). Full
evidence with method and caveats:
[`docs/roadmap/06-gpu-pipeline.md`](../../roadmap/06-gpu-pipeline.md),
"Evidence (Apple M2 / AGX under Asahi Linux, 2026-09-21)". Two runs -- four
alternating rounds, then two more closing the gaps the first left -- both
measuring byte-identical `nix build`s of `499083b`, which is what makes them
comparable with each other.

## What it said, and what the answer was

| the VM measured (llvmpipe) | the M2 measured |
| --- | --- |
| offscreen GLES, frame read back: **17-32x slower** than pixman | **parity**: 55.0 vs 56.4µs (empty), 57.3 vs 56.0µs (8 windows) |
| GPU scanout: **~1.5x slower** than the dumb tier | **4.2-5.1x faster**: 0.455 -> 0.090 j/ev motion, 3.358 -> 0.796 j/ev relayout |

The entry's own reasoning was that the read-back, not the rasterising, was
the dominant cost, so scanout should win where rasterising is the GPU's job.
That held, and the margin is larger than the VM's shape suggested.

Against the four metrics this entry named, in its order:

1. **Idle CPU** -- 0 jiffies over 10s, both tiers, every round. Neither
   wakes when nothing moves, so the "fast but busy is worse on a laptop"
   concern does not apply. Idle power indistinguishable: medians 5.510 W
   (dumb) and 5.505 W (gpu), per-round means spanning 5.411-5.533.
2. **Frame cost under damage** -- the table above; 20.8% of one core down to
   4.3% under large-damage motion, 52.0% down to 14.0% under relayout.
3. **RSS** -- +7 to +16 MB for the GBM swapchain, medians (the baseline moves
   between runs, so it is a range, not a constant). The one metric the dumb
   tier wins.
4. **Power** -- called "genuinely unknown and the most interesting number
   available" when this was filed. Scanout draws **less**: 0.22 W lower
   under motion, 0.25 W lower under relayout, whole-system, ~3.5% of draw.
   It does not trade CPU wakeups for GPU draw.

## Methodology, which this entry insisted on and which changed

Alternating rounds, medians with spread, never a best-of -- kept. Two things
had to change for this hardware, both found by rehearsing the harness rather
than by the run:

- **The VM's "300 unpaced pointer moves" measures nothing here.** An IPC
  round trip costs ~0.8 ms on this machine against the VM's ~11 ms, so an
  unpaced burst arrives ~20x faster than the panel refreshes and the
  compositor correctly coalesces it away: 3000 moves produced 16 jiffies.
  Damage is now driven at a fixed rate *below* the refresh rate for a fixed
  wall-clock window, and normalised per event -- the tiers get through
  different event counts in the same window (156 vs 176 relayouts), so a
  per-round total would compare different amounts of work.
- **"Confirm which tier is live before trusting a number" was necessary and
  nearly failed.** The compositor coloured its log unconditionally, so
  `scanout="gpu"` in a redirected log is really
  `scanout\x1b[0m\x1b[2m=\x1b[0m"gpu"`; the harness reported the tier as
  absent on a run where it had come up. Fixed at the source (`225719e`,
  gated on `IsTerminal`) -- every `tee` capture `Asahi.md` recommends was
  affected.

## Run 1's raw rows, recovered

Run 1's `summary.tsv` was destroyed on 2026-09-21 by pointing the entry-point
script at its own output directory (see `Asahi.md` Test 4; the script now
refuses that). These rows are a **transcription** from the review session's
captured output of that file, taken before it was lost — not the file.

They are not merely a second party's word for it. The `.power` sample files
survived untouched (mtimes 22:05–22:10, before the 23:10 overwrite), and
`idle_uW_mean` was computed from them, so that column re-derives from disk
today: **8 of 8 match exactly** under the script's own `power_mean`
arithmetic. One full column of the table below is independently checkable
against surviving artefacts.

16 columns, not 18: run 1 predates the two power-under-damage columns. This
is also why `connector` reads `?` and `scanout` reads `none` — run 1's
harness predated the ANSI stripping, so it could not read those fields out
of its own colourised log. Both facts are part of the record.

```
round	tier	came_up	paused	connector	scanout	idle_jiffies	idle_secs	idle_uW_mean	move_jiffies	move_events	move_ms	width_jiffies	width_events	width_ms	rss_kB
1	dumb	yes	no	?	none	0	10	5527300	312	694	15004	525	156	10056	77664
1	gpu	yes	no	?	none	0	10	5523800	64	711	15005	140	176	10052	93200
2	dumb	yes	no	?	none	0	10	5411000	310	680	15019	527	158	10037	80224
2	gpu	yes	no	?	none	0	10	5504900	63	710	15002	144	177	10015	95984
3	dumb	yes	no	?	none	0	10	5500000	317	690	15021	514	155	10040	72896
3	gpu	yes	no	?	none	0	10	5480700	62	711	15007	139	176	10006	93280
4	dumb	yes	no	?	none	0	10	5415800	311	684	15012	516	154	10006	78496
4	gpu	yes	no	?	none	0	10	5486500	66	709	15011	141	177	10052	95984
```

Its `environment.txt`, from the same capture, quoted as it read -- the host
string really has no separators (`tr -d '\0'` concatenates the device-tree
entries) and run 1's file labelled the SHA `git:`, the `harness tree:`
relabel having come later:
`date: 2026-09-21T22:05:23-04:00`;
`host: Linux 7.1.5 aarch64  apple,j413apple,t8112apple,arm-platform`;
`git: 650a1872f221a750ca37b6f16010beaad01cd60a`; binaries
`/nix/store/mf5nm9vmjrhq049w23dq3v3brymhsldb-scoot-0.1.0` and
`/nix/store/b1kkl6s3h5vwcz9g0bgx4avkggch4jj2-scoot-gpu-0.1.0`;
`gbm linkage: dumb=0 gpu=1`; `backend: --tty`; `vt: 2  session: 5`; seat
holder pid 1748 (the live noctalia session); `card2-eDP-1: connected`;
`ac_online=0 battery=Discharging`; `cpus: 8`.

Everything else from run 1 is still on disk: all eight screenshots, all eight
`.power` files, the `.outputs`/`.windows` dumps, and the **r3–r4** logs (r1
and r2's logs were overwritten with seat-failure output at 23:10).

## What stays open

- The motion scene is *large-bbox* damage (the injected path jumps across
  ~900x600 logical pixels). A small cursor-rect move is a different
  measurement and was not made.
- One panel, one resolution, one machine.
- Cursor and overlay planes: phase 2 of
  [gpu-scanout-planes](../rendering/gpu-scanout-planes.md), which phase 1
  of this run unblocks.
