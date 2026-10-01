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

## M3 gate: clock and workspaces (measured 2026-09-30, does not pass)

Run on the Asahi M2 by `scripts/scootbar-bench`, release scootbar from `main`
at `3211551` (`crates/scootbar` has not changed on `main` since). Tables, machine
readings and the method are in the
[README](../README.md#m3-clock-and-workspaces-on-the-asahi-m2) and
[testing.md](../testing.md#benchmark); the raw runs are
`bench/m3-asahi-*`. This records the findings; it waives no row and moves no
target. Nothing in it was fixed in the PR that recorded it.

**Rule 1 (no row regresses against the last milestone) fails.** The only
published M1 run (`bench/m1-clock`) came from a different machine, so M1's
scootbar (`3801c12`) was rebuilt on the Asahi box and both sides ran the same
harness at the clock scope, A-B-B-A, two runs each; a row counts only when it
regresses in all four pairings. That test is a measurement-method choice
stricter than the rule's "beyond the noise margin"; the maintainer can overrule it
(it leaves scoot idle CPU, flagged in 3 of 4, uncounted).

| Row (gated) | M1 `3801c12` | M3 `3211551` | Pairings regressed |
|---|---|---|---|
| Idle RSS, scoot / sway | 2.9 / 2.9 MiB | 3.5 / 3.5 MiB | 4 and 4 |
| Idle PSS | 1.5 / 1.5 MiB | 2.1 / 2.1 MiB | 4 and 4 |
| Idle heap (`RssAnon`) | 0.3 / 0.3 MiB | 0.4 / 0.5 MiB | 4 and 4 |
| Peak memory (`VmHWM`) | 2.9 / 2.9 MiB | 3.5 / 3.5 MiB | 4 and 4 |
| CPU while switching workspaces (240 switches) | 0.2 / 0.1 ms | 12.0 / 10.5 ms | 4 and 4 |
| Size, stripped binary + non-glibc `ldd` closure | 924,320 B | 1,383,080 B | 4 |

Not counted: idle CPU (scoot flags in 3 of 4 pairings at 0.8 to 1.2 ms, sway
in 1 of 4), startup (0 of 4) and idle wakeups (0 of 4, still 2 a minute).
The wakeups while switching (2 to 242) are not a gated row but are the same
finding as the switching CPU. The noise rule's own false-positive rate: M1
against its own rerun flagged one row (sway `RssAnon`, 272 against 288 kB, one
16 KiB page), exit 1; M3 against its rerun flagged none, exit 0. **Likely
cause of the switching row**, confirmed by a protocol trace: the daemon binds
`ext_workspace_manager_v1` whenever the `workspaces` feature is built, placed
or not (`crates/scootbar/src/daemon/wayland.rs`, the bind before the seat), so a
bar with no workspaces module is woken and parses every workspace change
(one wakeup per switch; open descriptors also went from 5 to 8, which a bound global does not explain). The memory and size
growth is the work since M1 (the module config, `toml`, `serde`, `png`: 6 to 10
direct dependencies; 10,628 to 25,347 lines of Rust, 6,995 to 16,312 outside
`tests.rs` files), not yet attributed row by row.

**Rule 2 (no competitor beats scootbar at the milestone's scope) is not
passed, though no row is lost.** At clock and workspaces, on every gated row
measured, there are 0 competitor wins: Waybar is 15 to 27 times larger in
memory and 18 to 64 times slower while switching, yambar (on sway) is 4 to 8
times larger and 8 times slower while switching; the ties (startup against yambar,
idle wakeups against yambar and Waybar, both on sway) are inside the noise
rule. **yambar on scoot was not compared**: 1.11.0 has no ext-workspace-v1
module, so it cannot show workspaces there and the harness does not invent a
number. The harness counts that as not passed (`report` exits 1); whether
that pair is waived is the maintainer's call, which the rule does not make
for an agent.

**Rule 3 (size of the codebase and dependency count)**: reported above, 6 to
10 direct dependencies and 6,995 to 16,312 lines outside tests, no threshold
to judge them against.

Verdict: **M3's gate does not pass**: rule 1 fails on the rows above, and rule 2
has one pair not compared. Neither was waived here.

**After the fix (2026-09-30, `cbbeffd`, branch `fix/scootbar-unplaced-workspace-bind`).**
The daemon now binds `ext_workspace_manager_v1` and `wl_seat` only while a
workspaces module is placed (at connect, when the global appears later, and
after a `reload`, which also lets them go when the module is removed). One run
of the same harness and settings on the same machine
([`bench/m3-asahi-clock-bindfix`](../bench/m3-asahi-clock-bindfix/table.md);
a single run, not repeated, so each verdict below is one pairing):
`compare` against [`m3-asahi-clock`](../bench/m3-asahi-clock/table.md) exits 1
(one flag: sway idle CPU, 0.6 to 0.9 ms, a row the table above already did
not count; switching CPU is better on both compositors) and against
[`m3-asahi-m1-baseline-clock`](../bench/m3-asahi-m1-baseline-clock/table.md)
exits 1 (11 flags).

- **Fixed: CPU while switching workspaces**, 12.0 / 10.5 ms (scoot / sway) to
  0.2 / 0.2 ms, against M1's 0.2 / 0.1 ms (`compare`: same). Wakeups while
  switching (not gated), 242 to 2, as M1.
- **Still regressed against M1, not touched by this fix:** idle RSS, idle PSS,
  idle heap and peak memory (3.5 / 2.1 / 0.4 / 3.5 MiB, as at M3, against
  M1's 2.9 / 1.5 / 0.3 / 2.9) and size (1,383,080 against 924,320 bytes).
  Idle CPU is flagged on both compositors in this one pairing against M1
  (0.9 to 1.0 ms against 0.6 to 0.9), where the table above did not count it
  (it flagged in 3 of 4 pairings); one run does not settle it.
- **Open descriptors (5 to 8) are not this bind.** A clock-only bar on headless
  scoot holds 8 after the fix, as before it (`ls -l /proc/PID/fd`: three on
  stdio, the clock's timerfd, the daemon's lock file and three sockets, one of
  them listed twice). Binding a global opens none; which of these M1 did not
  have was not traced here. Not a gated row.
- Nothing here waives a row or changes M3's verdict: rule 1 still fails on
  memory and size.

**2026-09-30 note: ironbar and ashell, informational.** The same run was
repeated with ironbar 0.19.0 and ashell 0.10.0 as two extra columns
(`bench/m3-asahi-clock-workspaces-all`, tables in the
[README](../README.md#m3-all-five-bars-on-the-asahi-m2)). They are M0's "if
cheap" pair, not the ratified competitors: **the gate still judges yambar and
Waybar only, and this note changes no rule, target or verdict.** Whether to
promote either into rule 2 is the maintainer's call; the harness lists
them apart (`Informational, not gated`) and never counts them. What the columns
show: **neither beats scootbar on any gated row on scoot or sway**, so there is
no row to report as a loss. Two are ties by the noise rule (startup against
ashell on both compositors; on scoot ashell's median, 28.6 ms, is below
scootbar's 34.4 ms but inside the spread). Elsewhere scootbar is ahead by
8 to 16 times in memory, by about 100 to 160 times in idle wakeups (ironbar 313 a
minute, ashell 194 to 276, against 2) and by 5.7 to 10 times in the CPU of 240
workspace switches. ironbar cannot show the workspaces on scoot (no
ext-workspace-v1), so it is not run there, as yambar. The bind fix (#363) is
not in these numbers, and does not bear on this scope. The caveats that
bound the comparison (ashell's bar height and font size are not options; neither
has a clock interval setting; M0's ashell config used a clock table 0.10.0
ignores) are in the README subsection.

**2026-10-01 note: the M4 stack on the Asahi M2.** The three draft PRs
(#364 pointer input, #366 `button`/`push`/`exec`, #367 the agent interface)
were measured at each layer's tip against `main` and against M3's post-fix
run: one scope (the clock), scootbar alone, the same machine, harness and
settings, one pinned release `scoot` for every run, eleven runs in three rounds
(`bench/m4-asahi-clock-*`; tables and method in the
[README](../README.md#m4-pointer-input-exec-and-the-agent-interface-on-the-asahi-m2)).
**One departure from the baseline, stated plainly:** the M3 post-fix run ran
scoot and sway, and rounds one and two of this run ran **scoot only**; round
three ran both for `main`, #367 and the experiment, and **#364 and #366 have no
sway run**. **Rule 1 (no regression against the last measured state) fails for the
stack; nothing is waived, no target moved, no PR touched by this note.**

- **Regressed against M3 post-fix, per `compare`** (the harness's own
  verdict, exit 1 for each layer): the **size** row at every layer, 1,383,080 B
  to 1,514,152 (#364, +9.5%), 1,579,720 (#366, +14.2%) and 1,645,256
  (#367, +19.0%), against `main` at 1,383,080 (exit 0 in two of three runs; the
  third flags only sway's idle heap, by one page, on byte-identical code); the
  **idle memory** rows, at #366 RSS 3.5 to 3.7, PSS 2.1 to 2.3, heap 0.4 to 0.5
  and peak 3.5 to 3.7 MiB, and at #364 and #367 the heap (0.4 to 0.5 MiB, one or
  two 16 KiB pages). RSS, PSS and peak are +0.13 to +0.20 MiB at every layer in
  every run; which of them `compare` flags in a given run depends on where that
  run falls against a 0.18 MiB margin.
- **Unchanged** (`compare`: same, in every run of every layer): startup, idle
  wakeups (2 a minute), the CPU and wakeups of 240 workspace switches (0.2 ms
  and 2), threads (1). **Idle CPU**, pooled with `verdict()` over #367's three
  runs [1.044, 0.993, 1.096] ms against `main`'s three [0.926, 0.943, 0.951]:
  same (margin 0.128); single runs flag it in places (#367's first run against
  `main`'s first, round three's pair on both compositors, the experiment's
  second run against M3 post-fix), which is the row's noise on this box.
- **The noise rule**: the margin is the largest of 5% of the baseline's
  median, the two sides' spread and the unit's floor (0.1 ms, 0.01 MiB, 1
  unit). Each build against its own other runs on scoot: `main` exits 0 and 0,
  #367 exits 0 and 0, the experiment exits 1 and 0 (idle CPU 1.09 to 1.30 ms).
- **Per-PR compares cannot see the cumulative creep.** #366 against #364 and
  #367 against #366 each exit 0 (+65,568 and +65,536 B, inside the size row's
  5% margin of 75,708 B), while the stack's total is +262,176 B (+19.0%, exit
  1 against `main`). The loaded code and data grew +17.1%; at #366, about half
  of its new text and read-only symbols (53 of 106 KB, by `nm`) is `toml`
  deserialization of the three new table shapes, and sharing one instance
  would save about 33 to 36 KB (estimated, not implemented). The file grows in
  64 KiB-aligned steps, so a layer's size step is its real growth rounded
  across boundaries.
- **An experiment, not in any PR:** `[profile.release.package.scootbar]
  opt-level = "s"` (with `strip = true`, quoted in the README) on #367 takes
  `.text` from 1,105,320 to 927,720 B (-16.1%, 0.3% below `main`'s), the bare
  executable to 1,315,552 B (-13.0% against #367 but **still 65,568 B larger than
  `main`'s 1,249,984**: the "smaller than `main`" lead reproduced for `.text`
  only), the closure to 1,448,648 B (inside the size margin, so `compare` does not
  flag it) and idle RSS/PSS to within the margin of `main`'s. Its idle CPU,
  [1.086, 1.302, 1.062] ms against `main`'s three, is **the same** by the
  pooled verdict (margin 0.265; an earlier two-run reading said regressed); the
  median is +0.14 ms and three runs cannot say whether that is a cost. Redraw
  cost was not measured. The maintainer decides; nothing in the stack changed.
- **Status after the maintainer's ruling (2026-10-01).** The **size** rows are
  accepted by the maintainer (the 2026-10-01 bullet under
  [Decisions](#decisions): "Keep the button and exec inside the bar. Accept it.
  Aim for optimization in usage more than pure disk space."). The **memory
  rows are still regressed** and tracked by
  [m4-usage-optimization](m4-usage-optimization.md); the fixed heads re-measured
  ([the README's final stack](../README.md#m4-final-stack-the-fix-round-re-measured))
  show size and idle PSS flagged against `main` on both compositors, and idle
  RSS, heap, CPU, wakeups, startup and the switching CPU not. **Nothing else is
  waived**, and no target moved.
- Not remeasured: rule 2 (competitors), which nothing in this stack touches,
  and what *using* the new modules costs (the dev VM's table in
  [exec-push-button-modules-done](resolved/exec-push-button-modules-done.md)).

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
- **M4's binary size is accepted; optimize usage, not disk** (user, 2026-10-01:
  "Keep the button and exec inside the bar. Accept it. Aim for optimization in
  usage more than pure disk space."). Accepted, at the numbers measured on the
  Asahi M2 ([the README](../README.md#m4-pointer-input-exec-and-the-agent-interface-on-the-asahi-m2)):
  `button`, `push` and `exec` stay in the default features and in the bar's
  binary, and the **size row** (binary plus closure) at 1,383,080 B on `main`
  to 1,645,256 B at the #367 tip, +19.0%, about +65 KB per layer, is not a
  regression to be worked down. **Not accepted, and not waived: the idle
  memory rows.** The words are about size and about where optimization effort
  goes; the idle RSS/PSS growth of +0.13 to +0.20 MiB per layer is still a
  regression against M3 post-fix and against `main`, to be worked down as a usage-optimization
  item ([m4-usage-optimization](m4-usage-optimization.md)). Nothing else in the
  ratchet moves.
