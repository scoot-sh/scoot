# Notification daemon A/B bench: scootnotify vs mako (dev VM, 2026-11-05)

Run on the **dev VM** (aarch64 NixOS, 6 vCPU, kernel 6.18.50), not the
Asahi M2 the ticket named: the M2 did not answer
(`ssh steve@192.168.1.64`: connection timed out). Do not set these
numbers against the M2 numbers in `dev/design/scootnotify.md` (mako
7,332 kB PSS idle there) or against the launcher bench; different
machine, different library mappings.

Idle cells ran as the ticket asked: **N=30 windows of 60 s** each
(30 minutes per cell, two passes). Every latency and memory cell is
N=30 per pass, two passes (order reversed in the second), no outliers
discarded. Nothing here is below N=30.

Release binaries `scoot 0.7.7` / `scootnotify 0.7.7` (aarch64,
`gh release download v0.7.7`, SHA256SUMS-checked:
`scoot-0.7.7-aarch64-linux`
`a3538d8078eb76f59f7b552ed6092ba51390e8d5748a817da10db079bc30de68`,
`scootnotify-0.7.7-aarch64-linux`
`04e99ee6aceca146fac1882b7fd401fee648e674d136bd1f7ede3276b543cbf5`)
against nixpkgs `mako 1.11.0` (`nix build nixpkgs#mako`, store path
`/nix/store/5x3nwidxrwmc531l7ia6a2xrisnq1qzv-mako-1.11.0`). NixOS cannot
exec the generic release binaries, so the bench ran patchelf'd copies
(interpreter and rpath point at nixpkgs glibc 2.42-84, libinput,
libseat, systemd-minimal-libs, pixman, libxkbcommon, libgcc; same
bytes otherwise; `meta.txt` records the sha256 of the copies).

Script: `scripts/notify-ab-bench/bench.sh` (shares `stats.awk` with the
launcher bench). Raw per-iteration values:
`dev/benchmarks/notify-ab-2026-11-data/run1/` (scootnotify measured
first) and `run2/` (mako first), plus `run1.log`/`run2.log`, each cell
`timeouts/n/min/median/p95/max/mean` over its file:

```sh
awk -f scripts/launcher-ab-bench/stats.awk dev/benchmarks/notify-ab-2026-11-data/run1/mako-cold.txt
```

(`*-mem-cold.txt` holds six space-separated columns: idle Rss, Pss, USS,
then card-showing Rss, Pss, USS, kB; `idle-*.txt` holds
`voluntary-ctxt-switches cpu-ticks threads` per 60 s window.)

## Method

- One private stack per phase: `dbus-run-session` (own session bus),
  its own `XDG_RUNTIME_DIR` on `/dev/shm/nab/...` (Wayland socket,
  scoot control socket, screenshots), empty `XDG_CONFIG_HOME` /
  `XDG_STATE_HOME` / `XDG_CACHE_HOME` apart from mako's config, and a
  fresh `scoot --headless --width 800 --height 600 --config /dev/null`
  (cpu renderer). Latency phases run alone, one daemon at a time; the
  four idle phases run in parallel, one stack each.
- Same card geometry and face: both top-right, 360 px wide, 12 px
  margin; DejaVu Sans, `scootnotify --font DejaVuSans.ttf` (13.0 px
  default) against mako `font=DejaVu Sans 10` (13.3 px at 96 DPI);
  neither reads operator config.
- One trial = `busctl --user call ... Notify` (summary "hello summary",
  body "body text here", `expire_timeout` 0 = never) timed from just
  before the call to the first `scoot msg screenshot` that differs from
  the card-less base frame (byte `cmp`; base verified stable 5x first).
  Then the card is closed with each daemon's own tool (`scootnotify msg
  dismiss ID`, `makoctl dismiss -n ID`; scootnotify refuses a bus
  `CloseNotification` from a different connection than the sender, and
  every `busctl` call is a new one) and the next trial waits for the
  frame to return to base.
- **Warm**: one long-lived daemon, 3 untimed warm-ups, then 30 trials.
  **Cold**: a fresh daemon process per trial (stopped, name released,
  respawned), 1 s after it owns the bus name the first card is timed;
  daemon start-up itself is not in the number.
- **Memory**: `/proc/PID/smaps_rollup` Rss, Pss and USS
  (`Private_Clean + Private_Dirty`) of the daemon, taken in the cold loop
  1 s after start with no card ("idle") and 0.5 s after the first card
  shows ("card showing"). Sampled only in the sequential latency phases:
  in the parallel idle phases two instances of one binary share its pages
  and skew Pss/USS (a first draft of the script sampled there:
  scootnotify USS 1,768 kB alone, 224 kB beside its twin). A 3-minute
  trace of both daemons (probe, not committed) held every figure
  constant to within 1 kB from 0 s to 183 s after start and after close,
  so the 1 s settle does not catch a transient.
- **Idle wakeups**: `voluntary_ctxt_switches` summed over every thread
  in `/proc/PID/task/*/status`, plus utime+stime ticks, read at the
  start and after each of 30 windows of 60 s, in two no-card states:
  **fresh** (never showed a card) and **closed** (one card shown and
  closed; for scootnotify this is the bar-less steady state, with its
  bar-feed send owed and the inotify watch armed instead of a timer).

## Timing floor

The detector is a screenshot round trip, so its round trip is the poll
period; there is no 10 ms sleep poll, the loop runs back to back.
Measured with the daemon up and no card (ms,
timeouts/n/min/median/p95/max/mean):

| pass | screenshot RTT (poll period) | bare bus round trip (`GetId`) | bus + screenshot (the harness part of every latency below) |
|---|---|---|---|
| run1, with scootnotify | 0/30/8/**11**/13/14/11 | 0/30/4/**7**/8/13/6 | 0/30/13/**16**/19/19/16 |
| run1, with mako | 0/30/6/**12**/16/19/11 | 0/30/3/**7**/9/11/6 | 0/30/8/**16**/22/24/15 |
| run2, with mako | 0/30/6/**12**/15/17/11 | 0/30/3/**7**/8/10/6 | 0/30/7/**17**/20/20/15 |
| run2, with scootnotify | 0/30/6/**10**/14/15/10 | 0/30/2/**6**/8/10/6 | 0/30/10/**15**/19/20/15 |

Every latency below includes about 15-17 ms of harness. Differences
smaller than the floor's spread (about 10 ms) are ties.

## Notify to first frame (ms)

| cell | scootnotify run1 | scootnotify run2 | mako run1 | mako run2 |
|---|---|---|---|---|
| warm (long-lived daemon) | 0/30/15/**20**/24/25/20 | 0/30/17/**23**/25/25/21 | 0/30/15/**21**/26/28/21 | 0/30/14/**20**/29/31/20 |
| cold (first card of a fresh daemon) | 0/30/11/**22**/26/27/21 | 0/30/13/**22**/27/27/21 | 0/30/26/**40**/44/46/38 | 0/30/25/**38**/46/47/36 |

Warm: a tie (medians 20-23 against 20-21, inside the floor's spread).
Cold: scootnotify's first card lands about 16-18 ms sooner at the
median (22 against 38-40) in both orders; that gap is on the order of
the harness floor's spread, so read it as "scootnotify is not slower
and probably faster", not as a precise figure. Nothing timed out.

## Memory (kB, one daemon alone, N=30 fresh processes per pass)

| state | scootnotify Rss / Pss / USS | mako Rss / Pss / USS |
|---|---|---|
| idle, no card (median) | 3,600 / 1,989 / 1,768 | 8,428 / 5,507 / 4,168 |
| first card showing (median) | 3,748 / 2,089 / 1,820 | 14,768 / 11,365 / 9,544 |
| after the warm run (33 cards shown and closed) | 3,676 / 2,075 / 1,856 | 15,076 / 11,673 / 9,852 |

(Medians, identical across the two passes within 1 kB; run1 and run2
`*-mem-cold.txt`.) A card costs scootnotify about +100 kB Pss and mako
about +5.9 MB Pss; mako does not give that memory back after the card
closes. scootnotify's Pss is 36 % of mako's idle and 18 % with a card
showing.

## Idle wakeups (30 windows of 60 s per cell, two passes)

| state | scootnotify voluntary ctxt switches per window | mako | threads (sn / mako) | cpu ticks per window |
|---|---|---|---|---|
| fresh | median 0, max 1 (window 1 only) | median 0, max 0 | 1 / 1 | 0 both |
| closed (after one card) | median 0, max 1 (window 1 only) | median 0, max 0 | 1 / 2 | 0 both |

Exactly one voluntary context switch in total per scootnotify cell, in
the **first** window after the 3 s settle (windows 2-30 are all 0, in
both states and both passes: 4 of 4 cells). mako recorded zero in every
window of every cell (4 cells, 120 windows). The single first-window
switch is consistent with a start-up tail; its cause was not isolated.
scootnotify's closed-state steady state (a send owed to a bar that is
not running) therefore costs no wakeups, which is what its design
promises (an inotify watch on the runtime dir, no timer, while no bar is
there). mako has a second thread once it has shown a card.

## Caveats

- **Not the M2; headless only.** The dev VM is a 6-vCPU virtual machine
  and also runs unrelated headless
  scoot sessions and bus daemons from other work (listed in `meta.txt`,
  left alone; load average 0.00-0.05 at start and end of both passes).
  Absolute milliseconds will not transfer to real hardware; the
  ordering is what to read.
- **The cards are not identical.** scootnotify draws a three-line card
  (summary, `app · now`, body; rounded border, urgency border color);
  mako a two-line one (summary, body). Heights and buffer sizes differ,
  so the work per frame differs.
- **"First frame" means the first frame that differs from base**, so a
  partly faded-in frame counts. scootnotify animates a card in; mako
  draws its final frame at once. The first differing frame is not the
  final one for scootnotify.
- The poll period is the screenshot round trip (about 10-12 ms), so each
  latency carries up to one period of detection quantization on top of
  the harness floor.
- Cold excludes daemon start-up (name-wait is not timed); it measures
  the first card, including any lazy surface and font setup the daemon
  defers to it.
- Memory is Rss/Pss/USS of one process; Pss and USS depend on which
  libraries other processes on the box also map, and the two daemons
  link different libraries (scootnotify needs only libc, libgcc and
  libxkbcommon). Rss is the one least sensitive to the
  box, and it shows the same ordering.
- Not covered: a bar attached (scootnotify then sends feed updates on
  every change), pause-when-idle, shadows, icons and images, markup,
  rendering quality (the ticket's non-goal and the flip ticket's
  side-by-side screenshots), a real GPU or a scaled output, a desktop
  with many monitors.
- Run1 used the script before the memory sampling was removed from the
  parallel idle phases (the discarded idle `mem-*` files are not
  committed); the committed script is byte-identical to what ran
  run2 (`sha256 273386bc8de9a156bbae2043eae7a8e3a501b9f359fa5867d10c2cb9e3256c9f`). Neither affects any figure here.
- Mako was run unmodified, with the default (non-lean) nixpkgs build;
  the design doc's "lean mako" build was not used. No upstream issue or
  PR was opened.

## Reproduce

```sh
# on the box: BIN_DIR holds scoot and scootnotify (patched copies on NixOS)
BIN_DIR=~/nab/bin MAKO=$(nix build --no-link --print-out-paths nixpkgs#mako)/bin/mako \
  OUT=~/nab/run1 scripts/notify-ab-bench/bench.sh
# second pass, the other way round:
ORDER="mako scootnotify" OUT=~/nab/run2 ... scripts/notify-ab-bench/bench.sh
```

Wall time about 40 minutes per pass (roughly 9 of latency, then 30 idle
minutes with four stacks in parallel). `N=2 IDLE_N=1
IDLE_SECS=3` is the 20 s smoke that proves the harness without a
result.

## Summary

scootnotify is at or ahead of mako on every axis measured here: a tie
on warm latency, a first card about 16-18 ms sooner, about a third of
the resident set at idle and about a sixth with a card showing, and no
idle wakeups in either state after a one-switch first window. The
cards differ (see Caveats), and the cold-latency gap is the least firm
of the four findings. The numbers do not argue against the shipped default.
