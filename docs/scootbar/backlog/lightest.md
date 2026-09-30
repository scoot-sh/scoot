---
title: "The resource ratchet: every milestone measured against the last one and the competitors"
status: "open"
area: "scootbar"
priority: "high"
blocked: null
milestone: "ongoing"
---

# The resource ratchet

Filed 2026-09-29; reframed the same day from a single release gate to a
ratchet, so the bar can ship in small steps. Serves the project's "low memory
and CPU is the name of the game". Modeled on scootbg's
[`lightest.md`](../../scootbg/backlog/lightest.md), but applied at every
milestone, not once at the end.

## The rule

A milestone is not done until its numbers are published and:

1. **No row regresses** beyond the noise margin against the previous
   milestone's published numbers, except where the new module adds a row that
   did not exist (then that row is measured against the competitors' equivalent).
2. **At the milestone's own scope, no competitor beats scootbar** beyond the margin
   on any row that applies to both. Clock and workspaces are compared with
   yambar and waybar showing exactly those modules; battery is compared when
   the battery module lands, and so on. A loss is a finding to fix, or a
   waiver the user writes down with the class it covers, as scootbg's was.
3. **The size of the codebase and its dependency count are rows too**: lightweight
   bars have died of maintainer burnout (yambar's own README says it is no longer
   developed), so keeping the bar small enough for one person is a measured
   property, not a hope.

## Rows

From [baselines-and-spikes](resolved/baselines-and-spikes-done.md), on the same machine and
outputs: idle RSS and PSS, idle wakeups per minute (target for the first
milestones: **two** with a clock placed, the clock's timer plus the compositor's
`wl_buffer.release` for each frame it draws, and **zero** with no module placed;
ratified, see [Decisions](#decisions)), CPU over a fixed window while switching
workspaces, peak memory, stripped binary size, installed closure size
([nix-package](resolved/nix-package-done.md) keeps fonts out of it), startup to first frame,
plus lines of code and direct dependency count.

Also **a multi-day soak**: RSS and fd count sampled over days with the bar
running through suspend/resume, DPMS wake and hotplug, since the bugs people
report in other bars are growth and CPU loops, not one-frame costs
([robustness-and-limits](robustness-and-limits.md)). Neither number is found
anywhere public for the competitors (no measured RSS or wakeup benchmarks turned
up in research), so the published comparison is itself a contribution.

Published in `docs/scootbar/README.md` with the method, by
`scripts/scootbar-bench` (reusing `scripts/scootbg-bench`'s runner), with each
milestone's table kept, not overwritten.

## Appearance looks (flush against floating)

[appearance](resolved/appearance-done.md) was done when the flush and floating
looks had measured costs published here. `scripts/scootbar-appearance-hw-test.sh`
measures them (four looks: flush-opaque, rounded-opaque,
rounded-translucent, floating), on real `--tty` hardware for the numbers that
count; [testing.md](../testing.md#the-appearance-hardware-test) has the method.

Measured 2026-09-30 on the Asahi M2 (NixOS aarch64, 8 CPUs), `--tty` on VT 2,
DejaVu Sans as the clock's font (`SCOOTBAR_HW_FONT`), release builds of scoot
and scootbar made on the box from `main` at `ee8948f` (the run's `environment.txt`
records the script's tree, `653e370`, not the binaries'; the script's only
change from `ee8948f` is the font option). One run, 14 PASS, 0 FAIL. **Two outputs
were live**, the panel (eDP-1, 2560x1600) and an external monitor (DP-1,
1920x1080), both at scale 1, and the default `outputs = "all"` put a bar on
each: RSS and the redraw costs below are for **two bars**, so they are not
comparable as they stand with the one-output clock-scope numbers above (rerun
with `outputs = ["eDP-1"]` for that). Idle is 20 s after settling; a redraw is
one of 300 whole-bar redraws (`scootbar msg reload`); the cursor number is
scoot's CPU per pointer move over the bar in an 8 s paced sweep.

| Look | scootbar RSS kB | idle wakeups / 20 s | idle jiffies | whole-bar redraw: scootbar / scoot ms | cursor move over the bar: scoot ms |
| --- | --- | --- | --- | --- | --- |
| flush-opaque | 3872 | 0 | 0 | 0.20 / 0.10 | 0.86 |
| rounded-opaque | 3920 | 0 | 0 | 0.23 / 0.13 | 0.81 |
| rounded-translucent | 4496 | 2 | 0 | 0.23 / 0.13 | 0.86 |
| floating | 3936 | 0 | 0 | 0.23 / 0.13 | 0.81 |

How to read it. Jiffies are 10 ms, so one jiffy over 300 redraws is 0.033 ms:
the +0.03 ms of every rounded or floating look against flush is **one jiffy,
not a measured cost**, and the cursor rows differ by less than the script's
own rule (trust a difference only past a jiffy per hundred moves). What the
run does show: a rounded, translucent or floating bar costs no more than a
flush one that this method can resolve, and all four idle at 0 jiffies. The
raw files (`summary.tsv`, `pixels.tsv`, `environment.txt`, screenshots,
protocol traces) are not committed.

**One bar, twice** (same box, same day, release builds of the tree at `f619928` (`main` at
`ec12fa7` plus the script's outputs option), `SCOOTBAR_HW_OUTPUTS=eDP-1` so the DP-1 monitor stayed live
but carried no bar; one `damage_buffer` request per run confirms one bar). These
are the numbers to compare with the one-output clock-scope baselines, and two
runs give the run-to-run noise:

| Look | scootbar RSS kB (A / B) | idle wakeups / 20 s (A / B) | idle jiffies | whole-bar redraw: scootbar / scoot ms (A; B) | cursor move: scoot ms (A / B) |
| --- | --- | --- | --- | --- | --- |
| flush-opaque | 3632 / 3936 | 0 / 0 | 0 / 0 | 0.17 / 0.07; 0.20 / 0.13 | 0.86 / 0.78 |
| rounded-opaque | 3680 / 3696 | 0 / 0 | 0 / 0 | 0.17 / 0.07; 0.17 / 0.10 | 0.84 / 0.81 |
| rounded-translucent | 3696 / 3680 | 0 / 0 | 0 / 0 | 0.17 / 0.10; 0.20 / 0.07 | 0.81 / 0.81 |
| floating | 3680 / 4016 | 0 / 2 | 0 / 0 | 0.17 / 0.10; 0.17 / 0.07 | 0.81 / 0.81 |

All eight look-windows idle at 0 jiffies. The run-to-run RSS swing on one look
is about 300 kB (flush 3632 against 3936, floating 3680 against 4016), so RSS
differences under roughly that are noise. The translucent look's RSS was 560 to
624 kB above the others in the two-bar run; in both one-bar runs it is level
with them, so that gap did not reproduce and is not a finding. The occasional
two context switches in a window (2 of 12 windows, against about 4 expected if a
minute tick lands in a 20 s window a third of the time) are consistent with the
clock's tick; no trace was taken to show it, and two switches per tick is assumed.

The rule applies as ever: none of these rows may regress the clock-scope
numbers above, and a look that costs real CPU or memory is off by default
(they all are: the default look is flush, square and opaque).

## Rules

- Release builds only (`lto = "fat"`, `panic = "abort"`).
- Compare after idle settles; say how it was detected.
- **Size is the binary plus what it links**, not the bare executable: a bar that
  links libwayland, pixman and a font library is not smaller for keeping them in
  shared objects. The bare-executable figure is published but not gated.
- After a milestone, every PR touching drawing, the event loop or a module re-runs
  the benchmark.

## Decisions

Ruled by the maintainer at the first milestone that measured them (#324,
`module-api-and-clock`). An agent does not waive a row or move a target; these
were the maintainer's calls.

- **Idle wakeups: two a minute is the target** (user, 2026-09-29: "I'm good with
  2"), for a bar with a clock placed; zero with no module placed. The second
  wakeup is the compositor's `wl_buffer.release` for the buffer each tick's commit
  replaced (about 1 ms later on scoot, 0.2 ms on sway; every `wl_shm` client gets
  it once per frame). The only protocol-legal way found to avoid it is a fresh
  buffer per frame, which costs more than one wakeup. Still below yambar's 2.6 to
  4 a minute. A later change that adds wakeups beyond these fails the ratchet as
  before.
- **Binary size is judged with what the binary links** (user, 2026-09-29,
  accepting a larger executable than yambar's). The bare executable is 849 KB
  against yambar's 407 KB, but yambar also links libwayland, pixman and fcft.
  The row is the binary plus its linked closure, which is how scootbg's own
  benchmark counts it. The M0 baselines recorded the installed closures (yambar
  771 MB, Waybar 1,037 MB, ironbar 1,232 MB, ashell 720 MB, nixpkgs default
  features); scootbar links only glibc, libm and libgcc_s. Its own closure,
  measured when `packages.scootbar` landed
  ([nix-package](resolved/nix-package-done.md#evidence)), is 49 MB with no
  font, published on the clock's table in
  [the README](../README.md#baselines).
