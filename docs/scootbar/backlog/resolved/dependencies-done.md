---
title: "Choosing dependencies (and checking licences)"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M0"
resolved: "2026-09-29"
---

# Choosing dependencies (and checking licences) — RESOLVED

scootbar's dependency record, in the manner of scootbg's
[dependency choices](../../../scootbg/backlog/resolved/dependencies-done.md).
[M0](baselines-and-spikes-done.md) started it on 2026-09-29, before any
scootbar code exists, with the two choices the clock milestone needs (the
font rasterizer; the clock timer and time zone) and the competitor
baselines. **Every later entry that picks a dependency adds its own section
here** (the config parser in
[config-cli-and-reload](../config-cli-and-reload.md), D-Bus in
[dbus-client](../dbus-client.md), icons in
[icons-and-fonts](../icons-and-fonts.md)), with its alternatives, numbers
and licence check.

The spike code is kept, not deleted, so each number can be re-derived: it
is in [`docs/scootbar/spikes/m0/`](../../spikes/m0/README.md), outside the
Cargo workspace and outside CI, with the raw output, configs and
screenshots under its `results/`. Paths below written `bench/…`,
`clock/…` or `results/…` are relative to that directory. The published baselines table is in
[`docs/scootbar/README.md`](../../README.md#baselines); the method and every
raw run are in §4 here.

## Decisions

| Area | Choice | Runner-up | Deciding numbers |
|---|---|---|---|
| **Font rasterizer** | **`ab_glyph` 0.2** (`FontRef` over the font's bytes) | `swash` 0.2 (runner-up, for hinting); `fontdue` 0.9 rejected | Binary +115 KB against swash's +885 KB and fontdue's +82 KB. Idle after filling 190 glyphs (DejaVu Sans, 1x and 1.5x): heap 228 KB, RSS 2.8 MB, against swash's 260 KB / 3.9 MB and fontdue's **19.3 MB / 22.3 MB** (fontdue outlines every mapped glyph at load: 42–63 ms, against ab_glyph's 0.05 ms) |
| **Font file** | **mapped** (`mmap`, private, read-only); **refined in M1 (§8): mapped only on a read-only mount, read into the heap elsewhere** | read into the heap | DejaVu Sans (742 KiB): heap 228 KB mapped against 976 KB read; load 0.05 ms against 0.6 ms; the pages are page cache, shared and reclaimable. One hazard, stated in §1d, closed in §8 |
| **Clock timer** | **absolute `CLOCK_REALTIME` timerfd** on the next local minute boundary, `TFD_TIMER_CANCEL_ON_SET`, re-armed after every wake with a post-arm check | a relative or interval timer | **10 wakeups in 600.0 s**, 0 involuntary switches, 0 CPU ticks; each tick 107–201 µs after the boundary; a clock step wakes it at once (§2) |
| **Time zone** | **a hand-rolled TZif v1–v4 reader with the POSIX TZ footer** (one file, ~450 lines, no dependencies) | `tz-rs` 0.7 (also correct; +16 KB more) | +8,192 B over a UTC-only build, against tz-rs +24,576, chrono +53,344, jiff +77,824. **0 mismatches against `zdump` over 414,100 instants in 598 zones, fat and slim files, years 1800–2200**; tz-rs also 0 (§3) |
| Zone change | `statx` the zone path on every wake; reload when device, inode, mtime or size change | inotify on `/etc` | no extra fd and no extra wakeup; a swapped `/etc/localtime` shows at the next tick (§3c) |
| Local-offset path of `time`, and libc `localtime_r` | **rejected** (not built) | — | `time`'s local offset is glibc's `localtime_r` underneath (`time/src/sys/local_offset_at/unix.rs` at `time-rs/time` `b623066`); `localtime_r` is C, reads `TZ` and `/etc/localtime` itself and, in glibc, keeps the zone it first loaded until `tzset`. Seeing a zone change through `time` takes `time::util::refresh_tz`, which refuses in a multi-threaded process (`sys/refresh_tz/unix.rs:47`) |

Every crate in the chosen trees is MIT-compatible (§5). Nothing was taken
from yambar, Waybar, ironbar, i3status-rust or ashell; none of their code
was read. Their licences are listed in §5 because i3status-rust and ashell
are GPL: they were only run, as binaries, for the baselines.

## What this changes in the plan

- **[module-api-and-clock](module-api-and-clock-done.md):**
  - Text through `ab_glyph`: `FontRef` over a mapping of the font file,
    with `PxScale` converted from the em size (ab_glyph's `PxScale` is the
    ascent-to-descent height, not the em: `em × height_unscaled ÷
    units_per_em`, as the spike does). Grayscale coverage, no hinting.
  - The clock: the timer as §2 describes it, including the check after
    arming (a step between reading the clock and arming is not reported by
    `CANCEL_ON_SET`), and `ECANCELED` treated as "redraw now, then re-arm".
  - The time zone: port the spike's `tzif.rs`, with its bounds (a 64 KiB
    file cap enforced while reading, not after: at most `MAX_FILE + 1` bytes from
    a regular file checked with `fstat`, opened `O_NONBLOCK`, so `TZ=:/dev/zero`
    or a FIFO cannot hang or balloon the loop; every count and index checked, an unusable footer falling back
    to the last transition, no file at all falling back to UTC), and make
    `check-zones.py` a test: a fixed set of zones and `zdump` output
    checked in as fixtures, so CI needs no tzdata. Clamp the instant before
    the footer arithmetic (the spike wraps silently for `t` near `i64::MAX`).
  - Honour `TZ` as glibc does (`:path`, a zone name under `TZDIR` or
    `/usr/share/zoneinfo`, or a POSIX string, which the footer parser already
    reads) before `/etc/localtime`. Not in the spike.
  - "Idle is exactly one wakeup per minute" is now a measured target, not
    a hope: the spike's loop does it (§2a).
- **[icons-and-fonts](../icons-and-fonts.md):** the fallback chain and the
  bounded cache are built on `ab_glyph` (`ttf-parser` underneath reads CFF
  and variable fonts: both rendered in §1b). **Revisit hinting there**:
  swash's hinted output is visibly crisper at 15 px (§1c). If users want
  that at 1x, swash is the measured alternative, at +770 KB of binary and
  ~1 MB of RSS over ab_glyph.
- **[robustness-and-limits](../robustness-and-limits.md):** a mapped font
  file truncated in place under the bar is a `SIGBUS` (§1d). Record the
  decision there with the other bounds.
- **[lightest](../lightest.md):** the first baselines are published, and
  §4 is the method later runs repeat. The spike harness is a seed for
  `scripts/scootbar-bench`, not that script.

## Setup

- Machine: a Claude Code web container: Intel Xeon @ 2.80 GHz, 4 vCPUs,
  15 GiB, Linux 6.18.44 (a Firecracker VM), Ubuntu 24.04 userland.
  rustc 1.97.1, glibc 2.42 (the devenv shell's). **Timings are from a
  shared VM and noisy**; memory and sizes are close to exact, and times
  within ~10% of each other are ties.
- The repository at `4abf67c` (`main` when this started).
- Spike profile: a copy of the workspace `[profile.release]` (`lto = "fat"`,
  `codegen-units = 1`, `panic = "abort"`, `strip = true`, `opt-level = 3`).
  **Each spike has its own `Cargo.lock`**, not the workspace's: it resolved
  `rustix` 1.1.5 where the workspace has 1.1.4. Every size is a delta over a
  base built the same way, so this does not move a delta.
- Stripped sizes come in 4 KiB steps.
- Fonts from the pinned nixpkgs (`8ce4ef6c`): DejaVu Sans 2.37
  (TrueType, 759,676 B), Source Sans 3.052 (CFF, `SourceSans3-Regular.otf`,
  334,924 B), Inter 4.1 (variable, `InterVariable.ttf`, 879,708 B).
- Zones: tzdata 2026c from the same nixpkgs (fat files, as Ubuntu's and
  NixOS's are), and the same `tzdata.zi` compiled `zic -b slim` (which
  leaves every future transition to the footer; zic's default since 2020).
- Memory in-process from `/proc/self/status` (`VmRSS`, `RssAnon`,
  `RssFile`, `VmHWM`) and `/proc/self/smaps_rollup` (`Pss`).

## 1. Font rasterizer

`sb-font-spike` loads one font, fills a `HashMap` glyph cache with the 95
printable ASCII characters at 15 px (1x) and 22.5 px (1.5x), 190 glyphs in
all, and draws `Tue 29 Sep 14:07` from the cache at both sizes. One engine
per build; the base build (no engine) reads the file and draws nothing.

### 1a. Size, memory and time

Binary, stripped:

| Build | Bytes | Over the base |
|---|---|---|
| base | 414,432 | — |
| `fontdue` 0.9.4 | 496,352 | +81,920 |
| **`ab_glyph` 0.2.32** | 529,136 | **+114,704** |
| `swash` 0.2.10 | 1,299,192 | +884,760 |

(The `mmap` feature builds the same size as reading, in every case.)

DejaVu Sans, 5 runs each. Load is the font parse, fill is the cache fill
for that scale, in µs; memory in KB after the fill and the draw (identical
across runs, ±16 KB):

| Engine, file | Load µs | Fill 1x µs | Fill 1.5x µs | RSS | PSS | Heap (`RssAnon`) |
|---|---|---|---|---|---|---|
| fontdue, read | 55119, 49311, 81363, 48453, 44655 | 535, 276, 353, 277, 247 | 338, 164, 285, 174, 163 | 22,388 | 22,381 | 20,036 |
| fontdue, mapped | 47035, 44040, 53423, 44407, 61650 | 271, 254, 253, 267, 282 | 169, 185, 211, 170, 158 | 22,332 | 22,325 | 19,292 |
| ab_glyph, read | 708, 589, 717, 671, 550 | 538, 710, 613, 571, 409 | 588, 342, 511, 553, 333 | 3,364 | 3,357 | 976 |
| **ab_glyph, mapped** | **47, 69, 46, 48, 40** | 379, 374, 765, 531, 409 | 363, 338, 344, 340, 371 | **2,856** | **2,849** | **228** |
| swash, read | 725, 606, 670, 579, 596 | 764, 436, 483, 466, 446 | 749, 473, 481, 452, 553 | 4,456 | 4,449 | 1,000 |
| swash, mapped | 40, 25, 45, 33, 35 | 473, 475, 925, 451, 464 | 586, 448, 898, 448, 731 | 3,908 | 3,901 | 260 |
| swash, mapped, hinted | 28, 30, 26, 25, 36 | 711, 704, 696, 671, 721 | 660, 727, 689, 929, 705 | 3,992 | 3,985 | 260 |
| *base (reads the file)* | — | — | — | *3,212* | *3,205* | *936* |

- **fontdue outlines every glyph the font's character map reaches, at
  load** (`Font::from_bytes`, `font.rs:285-310` in 0.9.4): +19 MB of heap
  and 42–63 ms for DejaVu Sans, whatever the bar will draw. It then
  rasterizes fastest, which a cache makes irrelevant. Rejected.
- ab_glyph and swash parse lazily (both borrow the bytes), so load is a
  header read and memory follows what is drawn.
- swash's extra ~1 MB of RSS is its own code: `RssFile` 3,648 against
  2,628 KB, the binary's pages. Its heap is ab_glyph's plus 32 KB.
- Rasterizing is a one-time cost per glyph and size: under 1 ms for all 95.
  Drawing the 16-character line from the cache took 4–11 µs in every
  engine, dominated by the naive blit.

### 1b. Other font formats

The same, 5 runs, memory in KB after the fill (mapped; read in brackets):

| Font | Engine | Load µs | Fill 1x µs | RSS | Heap |
|---|---|---|---|---|---|
| Source Sans 3 (CFF) | fontdue | 18331, 19100, 24887, 15312, 14761 | 197, 331, 259, 396, 212 | 9,072 | 6,508 (read: 6,840) |
| | **ab_glyph** | 43, 67, 41, 71, 48 | 456, 737, 407, 722, 409 | **2,896** | **220** (read: 552) |
| | swash | 20, 28, 18, 27, 18 | 714, 1302, 684, 810, 748 | 3,816 | 252 (read: 580) |
| Inter (variable) | fontdue | 22992, 21271, 23728, 21543, 21605 | 266, 233, 301, 261, 255 | 12,388 | 9,744 (read: 10,612) |
| | **ab_glyph** | 46, 39, 41, 47, 46 | 533, 508, 514, 539, 497 | **3,336** | **228** (read: 1,088) |
| | swash | 24, 25, 18, 19, 18 | 559, 630, 527, 609, 575 | 4,048 | 256 (read: 1,116) |

Every engine rasterized all 95 glyphs of both (bitmap bytes at 1.5x:
19,646 / 19,646 / 19,609 for Source Sans and 24,512 / 24,512 / 24,500 for
Inter, fontdue / ab_glyph / swash); these two were not inspected by eye.
The variable font renders its default instance;
choosing an instance (weight) is an `icons-and-fonts` question. Raw lines:
`results/font-runs.txt`.

### 1c. Output

All three produce the same glyph bitmaps within rounding (coverage sums at
1x: fontdue 661,618, ab_glyph 657,665, swash unhinted 662,855; bitmap
bytes 8,168, 8,168 and 8,164). The images are `results/shots/text-15px.png`
and `text-22.5px.png`, top to bottom fontdue, ab_glyph, swash, hinted swash. Looked at by eye, 4x nearest-neighbour: unhinted
fontdue, ab_glyph and swash are indistinguishable; **hinted swash has
visibly sharper vertical stems at 15 px**, and at 22.5 px the difference is
small. That is the case for swash, and why it is the runner-up rather than
rejected: hinting at 1x is the one quality lever the choice gives up, for
+770 KB of binary and ~1 MB of RSS.

### 1d. Mapped or read

**Mapped.** With a lazy parser the difference is the whole file in the heap
(976 against 228 KB for DejaVu Sans, 1,088 against 228 KB for Inter), while
a mapping keeps only the touched pages, in the page cache, shared with every
other process that maps the same font and reclaimable under pressure. Load is
0.05 ms against 0.6 ms.

The hazard: **a file truncated in place while mapped makes the next access
to a lost page a `SIGBUS`**, which kills the bar. A private mapping does not
prevent it. Replacing a font the usual way (write a new file, `rename` it
over the old one, which is what package managers and Nix do) is safe, since
the mapping keeps the old inode. A user truncating the exact file under a
running bar is the case left. Either document it or read the file into the
heap for paths outside `/nix/store`; that is for
[robustness-and-limits](../robustness-and-limits.md). fontdue owns its
parsed copy, so it could drop the bytes after load, but its parsed copy is
the 19 MB.

## 2. The clock timer

`sb-clock-spike run ZONEFILE` is the clock: one `timerfd_create(CLOCK_REALTIME)`,
armed `TFD_TIMER_ABSTIME | TFD_TIMER_CANCEL_ON_SET` at the next local
minute boundary, and a `poll` loop. On a wake it reads the timerfd (8 bytes:
a tick; `ECANCELED`: the clock was set), `statx`es the zone file,
formats the time into a fixed buffer, prints a line and re-arms. Re-arming
reads the clock again after `timerfd_settime` and re-arms if the deadline
is no longer within the next 60 s: `CANCEL_ON_SET` only reports a step made
after the arm, so without that check a step landing between the read and
the arm would leave the timer an hour out.

### 2a. Idle: one wakeup per minute

A 600-second window, 5 s after start, sampling `/proc/PID/status` and
`/proc/PID/stat` (one thread):

```
pid=7431 window_s=600.009467646 threads=1 start(vol nonvol ticks)=1 0 0 end=11 0 0 rss_kb=2328 pss_kb=718
```

**10 voluntary context switches in 600.0 s, 0 involuntary, 0 CPU ticks.**
Each wake, from the log (`late_us` is the time after the boundary):

```
tick mono=3362.927291 real=1790651340.000192075 late_us=192 ticks=1 ... show="Tue 29 Sep 2026 03:09 UTC"
tick mono=3422.927230 real=1790651400.000130628 late_us=131 ticks=2 ...
...
tick mono=3902.927228 real=1790651880.000123628 late_us=124 ticks=10 ... show="Tue 29 Sep 2026 03:18 UTC"
```

(`late_us` across the ten: 192, 131, 144, 107, 155, 119, 136, 169, 201, 124.)
`strace -tt` over two minutes shows each wake is `ppoll` returning, `read`
(8 bytes), `statx` of the zone file, the log's `write`, `timerfd_settime`
and `ppoll` again, all within ~0.5 ms, with nothing between wakes.

### 2b. Clock steps, DST and suspend

`clock/step-test.py` sets `CLOCK_REALTIME` under running clocks and puts it
back from `CLOCK_MONOTONIC` at the end. See §2c for the results.

**Suspend and resume** could not be exercised: this machine is a VM with no
suspend path to test safely, and the dev VM was not reachable from this
environment. What covers it instead is the kernel, read at Linux v6.18:
`timekeeping_resume()` calls `timerfd_resume()`
(`kernel/time/timekeeping.c:1993-1994`, "Notify timerfd as resume is
equivalent to clock_was_set()"), which schedules `timerfd_clock_was_set()`
(`fs/timerfd.c:95-130`). That cancels every `CANCEL_ON_SET` timer whose
monotonic-to-realtime offset changed, and suspend changes it (realtime
advances across suspend, monotonic does not). So a resume is delivered
exactly as a clock step is, the path §2c exercises. **Still to confirm on
hardware** in [module-api-and-clock](module-api-and-clock-done.md)'s tests.

### 2c. Step results

Five clocks ran at once (`America/New_York`, fat and slim;
`Australia/Lord_Howe`, fat, whose DST shift is 30 minutes;
`Australia/Sydney`, slim; `/etc/localtime`, UTC), and `step-test.py` set
the clock as its docstring says. The steps, as `CLOCK_MONOTONIC` seconds
and the time set:

```
forward_1h mono=7051.169075 set_to=1790658628.242
back_to_now mono=7054.169503 set_to=1790655031.242
lord_howe_dst_start mono=7057.173933 set_to=1791041390.000
sydney_dst_start mono=7072.174408 set_to=1791043190.000
new_york_dst_end mono=7087.174937 set_to=1793512790.000
restore mono=7102.175312 set_to=1790655079.248
```

What the clocks showed (from each log; `clock-set` is a wake by
`ECANCELED`, `tick` a wake by the timer):

| Event | New York (fat and slim alike) | Lord Howe | Sydney | UTC |
|---|---|---|---|---|
| start | Tue 29 Sep 00:10 EDT | 14:40 +1030 | 14:10 AEST | 04:10 UTC |
| +1 h (`clock-set`) | 01:10 EDT | 15:40 +1030 | 15:10 AEST | 05:10 UTC |
| −1 h (`clock-set`) | 00:10 EDT | 14:40 +1030 | 14:10 AEST | 04:10 UTC |
| to 2026-10-03 15:29:50Z | 11:29 EDT | Sun 04 Oct 01:59 +1030 | 01:29 AEST | 15:29 UTC |
| next `tick`, 15:30Z | 11:30 EDT | **02:30 +11** | 01:30 AEST | 15:30 UTC |
| to 15:59:50Z | 11:59 EDT | 02:59 +11 | 01:59 AEST | 15:59 UTC |
| next `tick`, 16:00Z | 12:00 EDT | 03:00 +11 | **03:00 AEDT** | 16:00 UTC |
| to 2026-11-01 05:59:50Z | Sun 01 Nov 01:59 EDT | 16:59 +11 | 16:59 AEDT | 05:59 UTC |
| next `tick`, 06:00Z | **01:00 EST** | 17:00 +11 | 17:00 AEDT | 06:00 UTC |
| restore (`clock-set`) | Tue 29 Sep 00:11 EDT | 14:41 +1030 | 14:11 AEST | 04:11 UTC |

- **Every step woke every clock at once**: the `clock-set` wakes came
  0.12–0.55 ms of monotonic time after the step's `clock_settime` began
  (for the first step, 7051.169253 to 7051.169363 against 7051.169075). Each counted exactly one
  cancel per step (`cancels=1` … `6`) and no extra tick.
- **Every DST boundary showed on the first tick after it**: Lord Howe's
  half-hour change (01:59 +1030 → 02:30 +11), Sydney's spring-forward
  (01:59 AEST → 03:00 AEDT) and New York's fall-back (01:59 EDT → 01:00
  EST), the slim New York file (footer rule) identical to the fat one. The
  ticks landed 61–230 µs after the boundary.
- The system clock was put back from `CLOCK_MONOTONIC`: `date -u` read
  `04:10:25.2` before the test and `04:11:21.3` after, 56 s of wall time for
  a 56 s test.

## 3. Time zone

### 3a. The candidates

Each built into the same clock (the `run` loop and a `bench` mode), with
one backend per build; `utc` has no zone code, and `bare` leaves out the
spike's `dump` and `mutate` test modes:

| Backend | Stripped | Over UTC-only | Dependencies | Notes |
|---|---|---|---|---|
| UTC only (base) | 398,032 | — | — | |
| **hand-rolled TZif + footer** (`bare`) | 406,224 | **+8,192** | none | this spike's `src/tzif.rs`, 448 lines |
| `tz-rs` 0.7.3 (`tzrs,bare`) | 422,608 | +24,576 | none | `#![forbid(unsafe_code)]`, MIT OR Apache-2.0, 5,566 lines |
| `chrono` 0.4.45 (`clock`, `std`) | 451,376 | +53,344 | `iana-time-zone`, `num-traits` | `Local` only; its own TZif reader inside |
| `jiff` 0.2.37 (`std`, `tz-system`, `tzdb-zoneinfo`) | 475,856 | +77,824 | `jiff-core` | |
| libc `localtime_r` | 393,936 | −4,096 | `libc` | the work is in glibc, in C |

Per conversion (zone lookup, civil date, and formatting
`Mon 21 Sep 2026 10:13 EDT` into a fixed buffer), America/New_York, 10⁶
instants spread over a year, 5 runs, ns:

| Backend | ns per format | Zone load, µs |
|---|---|---|
| UTC only | 179.7, 170.0, 170.0, 174.1, 172.3 | — |
| hand-rolled | 215.2, 193.0, 207.3, 206.2, 256.1 | 25.7, 24.9, 25.2, 33.5, 24.0 |
| tz-rs | 239.1, 205.3, 197.8, 196.4, 203.7 | 30.5, 26.2, 32.8, 27.9, 59.0 |
| jiff | 208.9, 210.8, 211.3, 203.1, 209.5 | 79.1, 85.1, 46.0, 48.5, 45.8 |
| chrono | 220.6, 228.7, 233.6, 235.7, 225.0 | (lazy) |
| libc | 272.2, 267.2, 265.1, 262.0, 272.8 | (lazy) |

All are ~0.2 µs, most of it `core::fmt`: once a minute, time is not a
criterion. RSS after the bench was 2,340–2,440 KB for all of them.

### 3b. Correctness

`check-zones.py` takes every zone in tzdata 2026c (598, skipping `posix/`
and `right/`), asks `zdump -v -c 1800,2200` (tzcode's own `localtime`) for
every transition in those years, which it prints as the second before and
the second of each, and compares offset, DST flag and abbreviation with the
spike's `dump` mode, for the fat file and the slim one:

```
hand-rolled: zones=598 instants_checked=414100 mismatches=0
tz-rs:       zones=598 instants_checked=414100 mismatches=0
```

A negative control (a slim `Europe/London` in place of `America/New_York`)
gave 1,120 mismatches, so the check can fail. The slim files are the harder
case: they end their transitions where the present rules begin and leave
every later one, through 2200, to the POSIX footer, which the reader
evaluates. The 94 distinct footers in tzdata 2026c exercise `Mm.w.d` rules,
`<-02>`-style quoted names, a negative rule time (`M3.5.0/-1`) and rule
times past 24 h (`/26`, `/50`). **The `Jn` and zero-based `n` date forms
appear in no footer, so they are untested** here; M1's tests need
hand-written cases for them.

Malformed input: `mutate` truncates a zone file at every length and applies
200,000 random 1–4 byte corruptions, parsing and querying each at six
instants, in the release profile (a panic aborts). All ten runs
(New_York, Lord_Howe, Dublin, Casablanca, Tehran; fat and slim) finished,
for example `mutate …/America/New_York: parsed=61214 rejected=142338`.
That is a smoke test, not a fuzz campaign: the M1 entry should fuzz it.

### 3c. A zone change

The spike `statx`es the zone path at every wake. With it pointed at a
symlink to `America/New_York`, the symlink was replaced atomically (a new
symlink `rename`d over it) with one to `Europe/London` at 03:17:47.34 UTC:

```
start mono=3887.267577 show="Mon 28 Sep 2026 23:17 EDT"
tick mono=3902.927324 real=1790651880.000171419 late_us=171 ticks=1 cancels=0 reloaded=true show="Tue 29 Sep 2026 04:18 BST"
```

It costs one `statx` a minute and nothing between. inotify would show the
change at once, but needs a watch on the directory (`/etc`, since the file
is a symlink that is replaced), an fd, and a wake for every unrelated
change there.

### 3d. The choice

**Hand-rolled.** It and tz-rs are both correct on every instant checked;
the hand-rolled reader is 16 KB smaller and adds no dependency, which
[lightest](../lightest.md) counts as a row. Its cost is ~450 lines to own,
and the `zdump` check is what keeps that honest, so it becomes a test.
**tz-rs is the fallback** if the reader ever needs more than it has: a
drop-in at +16 KB with no dependencies of its own.

## 4. Baselines

The published table is in [`docs/scootbar/README.md`](../../README.md#baselines);
this is the method and the raw runs (`results/bars-run.txt`).

### 4a. What ran

| | Version (pinned nixpkgs `8ce4ef6c`) | Licence | Shows on scoot | Shows on sway |
|---|---|---|---|---|
| yambar | 1.11.0 | MIT | clock only: 1.11 has no `ext-workspace` module | clock and workspaces (its `i3` module) |
| Waybar | 0.15.0 | MIT | clock and workspaces (`ext/workspaces`) | clock and workspaces (`ext/workspaces`) |
| ironbar | 0.19.0 | MIT | clock only: its workspaces module failed to start (`failed to create module Workspaces`; it speaks compositor IPCs, not `ext-workspace`) | clock and workspaces |
| ashell | 0.10.0 (iced) | GPL-3.0-or-later | clock, workspaces and the focused window's title | the same |
| i3status-rust | 0.36.1 | GPL-3.0-only | **not measured** | **not measured** |

i3status-rust is a status-line generator, not a bar: it needs `swaybar` (so
sway, never scoot) and draws no workspaces itself, so a fair row would be
swaybar and i3status-rust summed, on one compositor only. ironbar and ashell
covered the "if cheap" slot instead. The screenshots of every bar on both
compositors are `results/shots/bars-on-scoot.png` and `bars-on-sway.png`.

### 4b. Method

- **Compositors**, both running on the machine at once, each with one
  1920×1080 output: `scoot --headless --outputs 1 --width 1920 --height 1080`
  (a debug build of `4abf67c`, pixman), and sway 1.12 with
  `WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1`
  (`results/bar-configs/sway.config`). The one not under test sat idle.
- **Two workspaces with a window each** on both (foot 1.28.0), so there is
  something to switch between. scoot also lists its trailing empty
  workspace, so its bars show `1 2 3`.
- **Environment** (`results/bar-configs/env.sh`): a private session bus
  (`dbus-daemon` 1.16.2, no services, so GTK's accessibility bus and the
  portal are absent, as the logs say), fontconfig limited to DejaVu
  (`fonts.conf`), `LANG=C.UTF-8`, the zone UTC.
- **Configs** (`results/bar-configs/`): each shows `%a %d %b %H:%M`, with
  minute updates where the bar has the option (Waybar `interval: 60`,
  yambar's granularity follows its format); ironbar and ashell use their
  default update behaviour.
- **Harness**: `bench/bar-bench.py`, driven by `results/bar-configs/run-all.sh`,
  one bar at a time:
  - *startup to first frame*, 5 runs: the bar under `WAYLAND_DEBUG=1`, each
    stderr line stamped as it arrives; first frame is the first `commit` of
    a surface given a non-nil `attach` after the layer surface's
    `ack_configure`, timed from just before `exec`. That is the first buffer,
    whatever is in it: a bar that commits a blank frame first is flattered.
  - *idle*: a fresh start without debug output, **a fixed 30 s settle (idle
    was not detected, only waited for)**, then a 300 s window. **Wakeups
    are voluntary context switches** summed over every thread
    (`/proc/PID/task/*/status`), with involuntary ones reported beside them;
    CPU is on-CPU time summed from `/proc/PID/task/*/schedstat` (ns; a
    thread that exits inside the window would be missed), cross-checked by
    `utime+stime` ticks. Memory at the window's end: `VmRSS`, `Pss`
    (`smaps_rollup`), `RssAnon`, and `VmHWM` for peak (the whole run,
    startup included). Every bar was a single process.
  - *switching*: straight after the idle window, 240 workspace switches in
    60 s (4 a second, alternating `scootctl action focus-workspace-index 0/1`
    or `swaymsg workspace number 1/2`), the same counters. A bar that does
    not show workspaces there still gets the row: it says what the
    compositor's churn costs a bystander.
- **Binary** is the ELF that runs (nixpkgs wraps Waybar and ironbar, so
  `.waybar-wrapped`, `.ironbar-wrapped`), stripped by nixpkgs. **Closure** is
  `nix path-info -S`: nixpkgs' default features decide it (yambar and ashell
  pull PipeWire, Waybar SDL and GStreamer), so it measures the package as
  much as the bar.
- **One idle run and one switching run per bar and compositor**, not
  repeated; startup has 5.

### 4c. Raw

Startup, ms, the 5 runs:

| | scoot | sway |
|---|---|---|
| yambar | 28.5, 31.3, 22.1, 36.1, 22.3 | 63.1, 20.7, 25.8, 19.6, 20.0 |
| Waybar | 135.4, 152.3, 142.5, 145.6, 137.4 | 233.1, 125.1, 132.4, 127.5, 164.2 |
| ironbar | 108.9, 136.5, 107.2, 124.4, 191.5 | 249.2, 106.9, 111.7, 131.4, 111.5 |
| ashell | 77.5, 35.5, 26.8, 29.8, 30.1 | 86.8, 27.2, 23.0, 25.3, 20.3 |

Idle, 300.0 s windows, and switching, 60.0 s (as printed by the harness):

```
scoot yambar  threads=3  idle vol=20 nonvol=0 cpu_ms=3.59 ticks=0     rss=14252 pss=7402 anon=1816 hwm=14252   switch vol=4 cpu_ms=0.62 ticks=1
scoot waybar  threads=8  idle vol=25 nonvol=0 cpu_ms=9.10 ticks=1     rss=53856 pss=43238 anon=8288 hwm=53856  switch vol=965 cpu_ms=420.39 ticks=43
scoot ironbar threads=15 idle vol=1550 nonvol=0 cpu_ms=170.62 ticks=17 rss=59620 pss=48898 anon=11660 hwm=59620 switch vol=1112 cpu_ms=83.13 ticks=8
scoot ashell  threads=9  idle vol=1139 nonvol=0 cpu_ms=124.71 ticks=16 rss=30480 pss=26964 anon=4972 hwm=31140  switch vol=3150 cpu_ms=312.08 ticks=32
sway  yambar  threads=4  idle vol=13 nonvol=2 cpu_ms=3.22 ticks=0     rss=14192 pss=7352 anon=1856 hwm=14192   switch vol=1276 cpu_ms=153.54 ticks=16
sway  waybar  threads=8  idle vol=15 nonvol=0 cpu_ms=8.08 ticks=1     rss=53968 pss=43373 anon=8540 hwm=53968  switch vol=719 cpu_ms=399.02 ticks=40
sway  ironbar threads=15 idle vol=1539 nonvol=0 cpu_ms=174.91 ticks=17 rss=58988 pss=48434 anon=11700 hwm=58988 switch vol=1982 cpu_ms=268.69 ticks=28
sway  ashell  threads=9  idle vol=963 nonvol=15 cpu_ms=112.69 ticks=15 rss=29664 pss=26423 anon=4592 hwm=30120  switch vol=2937 cpu_ms=311.28 ticks=32
```

Sizes, bytes:

| | Binary | Closure | Store paths |
|---|---|---|---|
| yambar | 407,296 | 770,775,760 | 240 |
| Waybar | 3,885,680 | 1,036,757,760 | 322 |
| ironbar | 37,176,976 | 1,231,996,320 | 348 |
| ashell | 41,597,120 | 719,861,008 | 206 |

### 4d. What the numbers say

- **yambar is the bar to beat on every row but closure size** (ashell's is smaller): 14 MiB RSS, 7 MiB PSS, under
  2 MiB of heap, 3–4 wakeups a minute, 3–4 ms of CPU in 5 minutes, a 400 KB
  binary and a ~20–30 ms first frame. Waybar matches it on idle wakeups
  (3–5 a minute) at almost 4× the memory. ironbar (~5 wakeups a second)
  and ashell (3–4 a second) wake constantly while idle. Where those
  wakeups come from was not investigated.
- **Nobody reaches one wakeup a minute**, which the spike clock does (§2a):
  that is scootbar's M1 target, at the spike's 2.3 MB RSS and 0.7 MB PSS
  before any Wayland code.
- **Switching workspaces costs the bars that show them 0.27–0.42 s of CPU
  a minute at 4 switches a second**, Waybar the most; yambar showing sway's
  workspaces through the i3 IPC costs 0.15 s.
- ashell, the one iced bar, sits between the two groups on memory (30 MB)
  and has by far the largest binary (41.6 MB).
- The two compositors agree within a few per cent on every memory row,
  which is expected: the bar's own costs dominate.


## 5. Licences

The chosen trees, from `cargo tree -e normal --features … -f "{p} | {l}"`
in each spike:

| Crate | Licence | For |
|---|---|---|
| ab_glyph 0.2.32, ab_glyph_rasterizer 0.1.10, owned_ttf_parser 0.25.1 | Apache-2.0 | text |
| ttf-parser 0.25.1 | MIT OR Apache-2.0 | text |
| rustix 1.1.x, linux-raw-sys 0.12.1 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | timerfd, poll, statx, mmap |
| bitflags 2.13.2 | MIT OR Apache-2.0 | (rustix) |

**All MIT-compatible.** ab_glyph's three crates are Apache-2.0 only, which is
permissive; a binary distribution carries their notice, as MIT requires for
scootbar's own. The rejected ones were checked too: fontdue 0.9.4 (MIT OR
Apache-2.0 OR Zlib; hashbrown, foldhash (Zlib), libm, core_maths, ttf-parser),
swash 0.2.10 (Apache-2.0 OR MIT; skrifa, read-fonts, font-types, yazi, zeno,
bytemuck (Zlib OR Apache-2.0 OR MIT)), tz-rs 0.7.3 (MIT OR Apache-2.0),
jiff 0.2.37 and jiff-core (Unlicense OR MIT), chrono 0.4.45, iana-time-zone,
num-traits and libc (MIT OR Apache-2.0). No GPL, LGPL or MPL in any of them.

New to the workspace lock with the choice: `ab_glyph`,
`ab_glyph_rasterizer`, `owned_ttf_parser`, `ttf-parser`. `rustix` and
`bitflags` are already there.

**The competitors, run as binaries only** (licences from their nixpkgs
`meta.license`): yambar MIT, Waybar MIT, ironbar MIT, ashell
**GPL-3.0-or-later**, i3status-rust **GPL-3.0-only**, sway MIT. No code from
any of them was read or copied; the two GPL ones must stay that way.

## 6. `unsafe` and fuzzing

Grep-based, as scootbg's record did (`unsafe {`, `unsafe fn`,
`unsafe impl` in `src/`), with upstream checked by a blob-less clone of
each repository's head for fuzz targets:

| Crate | ≈ `unsafe` sites | What for | Upstream fuzzing |
|---|---|---|---|
| **ttf-parser 0.25.1** | **0** (`#![forbid(unsafe_code)]`) | — | yes: `testing-tools/ttf-fuzz` (`harfbuzz/ttf-parser` at `0c72912`, 2026-08-06) |
| ab_glyph 0.2.32 | 0 | — | none (`alexheretic/ab-glyph` at `3eb21a5`) |
| ab_glyph_rasterizer 0.1.10 | 5 | runtime AVX2/SSE4.2 dispatch of `draw_line` | none |
| owned_ttf_parser 0.25.1 | 5 | the self-referential owned face (`FontVec`), which scootbar's `FontRef` path does not use but compiles | none |
| *rejected:* fontdue 0.9.4 | 31 | | none (`mooman219/fontdue` at `2924772`) |
| *rejected:* swash 0.2.10 | 54 (skrifa and read-fonts: 0, `forbid`) | | swash: none; fontations: `fuzz/` (`googlefonts/fontations` at `5650b42`) |
| *fallback:* tz-rs 0.7.3 | 0 (`#![forbid(unsafe_code)]`) | | not checked |

The parser that reads the untrusted bytes (a font file from a user path) is
ttf-parser: no `unsafe` and fuzzed upstream. The rasterizer only sees
outlines ttf-parser produced.

The hand-rolled TZif reader has no `unsafe`, and the spike's mapping of the
font file is the only `unsafe` the choices need (one `mmap`, one
`slice::from_raw_parts`).

## 7. The Wayland client (the skeleton)

Added by [skeleton-layer-surface](skeleton-layer-surface-done.md). The
skeleton picks no new dependency: every crate it uses is already in the
workspace for scootbg, chosen there by measurement (plain `wayland-client`
over `smithay-client-toolkit`,
[scootbg's record](../../../scootbg/backlog/resolved/dependencies-done.md)
§1), with its licences checked there (§8):

| Crate | For | Licence |
|---|---|---|
| `wayland-client` 0.31, its pure-Rust backend (no libwayland) | the connection | MIT |
| `wayland-protocols` 0.32 (`client`, `staging`) | `wp_viewporter`, `wp_fractional_scale_v1` | MIT |
| `wayland-protocols-wlr` 0.3 (`client`) | `zwlr_layer_shell_v1` | MIT |
| `rustix` 1 (`std`, `event`) | `poll(2)`, with no `libc` crate | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| `scootbg-mem` (in the tree) | the sealed-memfd `wl_shm` buffer, so scootbar itself is `#![forbid(unsafe_code)]` | MIT |

`cargo tree -p scootbar -e normal --prefix none` lists 32 lines and no
`libc` crate, and the release binary links only glibc and libgcc_s (`ldd`;
CI asserts both). The stripped release binary is **602,832 bytes** at
`d7384ad` and `62181e6`, against the M0 spike clock's 398,032 with no Wayland code at
all, so the Wayland client, the three protocol crates and the draw path
are about 200 KB. yambar's 407,296 is smaller, but it links
`libwayland-client`, `pixman` and `fcft` dynamically (its closure row
carries them); scootbar links none, so its binary is the whole of it. The
font rasterizer (+115 KB, §1) and the TZif reader (+8 KB, §3) join with the
clock.

## 8. The font file, decided (the clock)

Added by [module-api-and-clock](module-api-and-clock-done.md), whose Done
when made this decision its own rather than
[robustness-and-limits](../robustness-and-limits.md)'s. No new dependency:
`ab_glyph` as §1 chose it, and the mapping in `scootbg-mem`.

**Mapped only where the file lies on a read-only mount; read into the heap
everywhere else** (`crates/scootbar/src/font.rs`,
`scootbg_mem::file::map_if_read_only`). Why neither of the two options the
entry offered as they stood:

- *Map everywhere and document the hazard* leaves a `cp new.ttf
  ~/.local/share/fonts/font.ttf` (which opens the old file `O_TRUNC`)
  killing the bar with `SIGBUS`: a plausible user action, a crash, and a
  crash is treated like data loss (`CLAUDE.md`).
- *Read everything outside `/nix/store`* keys safety on a path: on a
  single-user Nix install the store is writable by the user, and a path
  check says nothing about a bind mount or a symlink into it.
- *The mount's read-only flag* (`fstatvfs`, `ST_RDONLY`, which Linux
  reports per mount) is what actually makes truncation impossible
  (`EROFS`, root included), and it covers NixOS's store (bind-mounted
  read-only), where Stylix and the modules take fonts from, and image-based
  systems' `/usr`. What it leaves, stated in the mapping's docs: the same
  filesystem written through another, read-write mount (on NixOS only
  `nix-daemon`, which never rewrites a store file in place) and disk errors.
  The check and the `unsafe` live together in `scootbg-mem`, so the safety
  argument is enforced where it is made; scootbar stays
  `#![forbid(unsafe_code)]`.

Measured in the bar (release build at `44fc656`, headless scoot, DejaVu
Sans 2.37, 742 KiB, idle 300 s after a 30 s settle, M0's harness), the same
file read and mapped (a read-only bind mount of a copy):

| | Read (heap) | Mapped |
|---|---|---|
| RSS | 3,944 KiB | 3,388 KiB |
| PSS | 2,108 KiB | 1,552 KiB |
| Heap (`RssAnon`) | 940 KiB | 196 KiB |
| Wakeups in 300 s | 10 | 10 |

So the read path costs 744 KiB of heap and 556 KiB of RSS for this font,
in line with the entry's "about +750 KB"; a CJK or Nerd font would cost
its size (the cap is 64 MiB). And, checked live on the same machine: with
the read-only font, `/proc/PID/maps` shows `r--p ... /tmp/sb-rofont/DejaVuSans.ttf`
and a truncation is refused (`Read-only file system`); with the writable
one, no mapping, and the bar kept ticking every second after the file was
truncated to 0 bytes under it.

## Not measured, and why

- **Suspend and resume** (§2b): no safe suspend here, and the dev VM was not
  reachable. Covered by reading the kernel, to confirm on hardware.
- **A real DST change in real time**: the DST transitions in §2c are reached
  by stepping the clock to 10 s before them, then letting the boundary pass
  in real time. The wall clock was not left to run into a DST change.
- **musl**, and **`cargo-geiger`/`cargo-audit`/`cargo-deny`**: not installed;
  the `unsafe` counts are a grep, as in scootbg's record, and no advisory
  database was checked for these crates.
- **Shaping, fallback fonts and emoji**: out of scope for a clock (see
  [icons-and-fonts](../icons-and-fonts.md)).
- **`cosmic-text`**: excluded by the entry (too heavy for digits).
- **The fuzzing of `tzif.rs`**: a mutation smoke test only (§3b).
- **i3status-rust** (§4a), and every bar on a **third compositor or real
  hardware**: both compositors here are headless, pixman, in one VM.
- **Repeated idle windows**: one per bar and compositor (§4b), where
  scootbg's gate used three. Enough for a first table; the ratchet's
  runner should repeat them.
- **Idle detection** for the bars: a fixed 30 s settle, not a detected one.
- **Where the competitors' wakeups come from** (ironbar's ~5 a second,
  ashell's ~3.5): measured, not explained.
