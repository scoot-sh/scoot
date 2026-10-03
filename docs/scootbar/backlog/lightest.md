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

## M4 follow-ups: hover, state colors, dots, disc (measured 2026-10-02)

The [appearance follow-ups](resolved/appearance-followups-done.md) draw nothing the
existing looks do not: the `hover` token and the workspaces state colors
(`active-color`, `inactive-color`) recolor the same fills, and dots and
the grown disc are small maximally-rounded fills of the kind the pill
costs above. Micro-measured in release on the dev VM (x86_64, seven-segment
test font at a 50-pixel em, so the discs are ~50 device pixels — several
times real size): a dots redraw costs about 6 us per dot (25/49/100 us for
4/8/16 workspaces, linear, one fill each, no allocation), and a grown disc
about 8.5 us against 5 us for the plain pill it replaces, once per
workspace change. None of the four adds a file descriptor, a timer or a
wakeup — the contract test holds every module to the loop's source budget,
and these add no sources — so idle RSS, wakeups and jiffies are unchanged
by construction. No full hardware run: there is no new row for the bench
to regress, only recolored and bounded fills.

## M5 network: module-level cost (measured 2026-10-02, full bench pending)

The [network module](resolved/network-module-done.md) adds two netlink
sockets to the poll set, a `timerfd` only while a WiFi network is shown (10 s
signal re-read), a pidfd only while its picker runs, and a 5 s one-shot
retry timer only after a resync — no polling anywhere. The contract test
holds every module to the loop's source budget (5 sources at most here, of
63). Idle wakeups, live through the module harness: 1 in 20 quiet seconds
on ethernet (dev VM, aarch64), 2 in 20 quiet seconds on WiFi (Asahi M2, three
runs: exactly the timer's two ticks; one earlier run caught a background
scan completing, a real event). A scan-heavy window adds real-event wakes
against the same bound (the live test allows 4). Memory is fixed shapes: two
64 KiB read buffers, a 32-entry scan array, outboxes of a few hundred bytes
of dump requests, SSIDs sanitized once at parse time into ≤32-byte strings.
No new dependencies (the Cargo feature adds none); the release binary is
1,643,240 bytes on aarch64 and links only libc, libm and libgcc_s. No full
`scripts/scootbar-bench` run (idle RSS/PSS/jiffies/size rows): it needs the
bench runner under a compositor, which this lane did not have — the rows
above are the module's published cost until that run happens.

## M5 brightness: module-level cost (measured 2026-10-02, full bench pending)

The [brightness module](resolved/brightness-module-done.md) adds one netlink uevent
socket to the poll set, and nothing else: no timer exists anywhere in it
(re-reads happen on a backlight uevent, or synchronously inside the
`invoke` that wrote), so there is no periodic wakeup by construction. The
contract test holds every module to the loop's source budget (1 source
here, of 63), and the module's own test pins that count. Idle wakeups,
live through the module harness: 0 in quiet seconds on any machine (no
event, no wake; a driver that chatters uevents is drained into one
re-read per turn, as the storm test pins). Memory is fixed shapes: two
small stack buffers per file read, one 8 KiB drain buffer per turn, the
device name kept once. No new dependencies (the Cargo feature adds none);
the release binary is 1,708,768 bytes on aarch64 (the dev VM), +65,528
against the network module's 1,643,240 on the same box and profile, and
links only libc, libm and libgcc_s. The level itself is cross-checked two
ways on the Asahi M2: the bar's rounding (107 of 509 shows 21%) agrees
with `brightnessctl`'s independent 21%, and the module's tests pass there
against the real backlight. No full `scripts/scootbar-bench` run (idle
RSS/PSS/jiffies/size rows): it needs the bench runner under a
compositor, which this lane did not have — the rows above are the
module's published cost until that run happens.

## M6 tray and the D-Bus client: module-level cost (measured 2026-10-02)

The [tray](tray.md) is the [shared D-Bus client's](resolved/dbus-client-done.md)
first consumer, so this is the cost of both. By construction: the bus
socket is one source in the poll set (with `OUT` only while a write waits
or staged messages wait their turn), or one inotify watch on the socket's
directory while there is no bus; a one-shot timer exists only while an
item waits out the 50 ms floor between reads of it, or (30 s) after the
bus kept dropping the bar, and not otherwise; no thread, no polling. No
new dependency (`Cargo.lock` is unchanged). The contract test holds the
module to the loop's source budget (2 sources at most, of 63).

**Method.** Release builds (`lto = "fat"`, stripped) of `origin/main`
(`a60852c2e`) and of the branch at code commit `6a2662cd7` (`crates/` tree
`7bc1a04525c8`), each `git archive`d to the dev VM (aarch64, 6 CPUs, rustc
1.97.1) and built with its own target dir; a headless `scoot` (an existing
release build, used read-only), a private `dbus-daemon` 1.16.2, and the
items real D-Bus peers: Python scripts on `jeepney`, an independent
marshaller, each owning an item name, registering, and answering `GetAll`
with a pixmap. One bar per run, sampled from `/proc/PID` after 14 s of
settling and again 60 s later (`VmRSS`, `Pss`, voluntary context switches,
fds), **one run per row**, on a VM with load about 0 to 1 from other work.
Wakeup counts are per process and exact for the window; the RSS and PSS
differences under about 130 kB are not resolved by one run (two rounds of
this table, a different commit apart, disagreed by that much on the
unplaced rows). The scripts and raw logs are in
[`bench/m6-tray-vm`](../bench/m6-tray-vm/README.md).

| Row | RSS kB | PSS kB | wakeups in 60 s | fds |
|---|---|---|---|---|
| `main`, no module placed | 3636 | 2083 | 0 | 7 |
| branch, tray built, no module placed | 3764 | 2211 | 0 | 7 |
| `main`, clock | 4056 | 2293 | 2 | 8 |
| branch, tray feature off, clock | 4044 | 2292 | 2 | 8 |
| branch, tray built and not placed, clock | 4172 | 2420 | 2 | 8 |
| branch, tray alone, **no bus** | 4028 | 2292 | **0** | 8 |
| branch, tray alone, bus, no items | 4028 | 2291 | **0** | 8 |
| branch, tray alone, bus, 1 item | 4224 | 2484 | **0** | 8 |
| branch, tray alone, bus, 8 items | 4256 | 2507 | **0** | 8 |
| branch, tray and clock, bus, 1 item | 4264 | 2433 | 2 (the clock's) | 9 |

How to read it. **Zero wakeups**, tray alone, with no bus, a bus and no
items, and a bus with one and with eight items on it: nothing in the
tray's idle state wakes the bar, and the only wakeups in the rows with a
clock are the clock's two a minute (its tick and the compositor's buffer
release). One thread throughout. Against the same layout without the tray
placed, the first item costs about 90 kB RSS (4172 to 4264, with the
clock) or about 200 kB (4028 to 4224, tray alone), and each of seven more
about 5 kB (the 8-item row is 32 kB over the 1-item row); those deltas are
at or under what one run resolves, so read them as "tens to a couple of
hundred kB", not as a model. With the tray built but not placed, RSS was
about 125 kB over `main` in this round (3636 to 3764, 4056 to 4172) and
level with it in the first round: the larger binary's pages, or noise.

**Binary** (aarch64, stripped), measured on the three builds with `size`
and `readelf` because the file size alone is quantized to 64 KiB steps:

| Build | file bytes | `.text` | `.text`+`.rodata`+`.eh_frame*`+`.gcc_except_table`+`.data*` |
|---|---|---|---|
| `main` `a60852c2e` | 1,774,304 | 1,360,520 | 1,680,819 |
| branch, `--features` default minus `tray` | 1,839,840 (+65,536) | 1,363,400 (+2,880) | 1,684,131 (+3,312) |
| branch, default (tray on) | 1,905,376 (+131,072) | 1,457,416 (+96,896) | 1,793,315 (+112,496) |

So the **feature off is not free and not byte-identical to `main`**: it
costs about 3.3 KB of loaded sections (edits outside the tray module that
a tray-off build still carries: `render.rs`'s span for an icon-only
module, the resampling filter moved out of `image.rs` into the shared
`sample.rs`, the help and config text; not bisected further), which
crosses a 64 KiB boundary on disk, so the file is 65,536 B larger. The **tray on
costs about 94 KB of `.text`, 109 KB of loaded sections over the feature
off** (112 KB, +6.7%, over `main`), which the file size shows as another
65,536. `ldd` still shows only libc, libm and libgcc_s, and `Cargo.lock`
is byte-identical to `main`'s. The feature is in `default`, the way every
module is, which the maintainer decided knowing the above; the smallest
build (`--no-default-features`) has none of it, and that is where the row
is bought back.

**Which rows of the rules regress.** Rule 1 (no row regresses against the
previous milestone beyond noise, except a row the new module adds): the
stripped binary size row regresses by 131,072 B on disk (+7.4%, 112,496 B
or +6.7% of loaded sections), and tray built but unplaced regresses idle
RSS by about 125 kB in one of two rounds (within what a single run
resolves). Idle wakeups, jiffies, fds and threads do not regress in any
row, with the tray placed or not. The binary row is the same shape as
every module before it (brightness was +65,528 B over the network build);
the rule's own exception covers only a row the module adds,
so this is a regression for the maintainer to waive or not, as the M3 gate
entry below was.

**Maintainer's ruling (2026-10-02, given in chat): the stripped-binary size
regression is waived.** It covers the tray's +131,072 B on disk (+112,496 B of
loaded sections) in the default build, and only that row. Nothing else is
waived: no other row regresses beyond what one run resolves, and the
unplaced-tray idle RSS reading stays a one-run observation, not an accepted
cost. `tray` stays in the default features by the same decision.

**Maintainer's ruling (2026-10-03, given in chat): the tooltips binary-size
row is waived.** It covers `.text` +13,440 B (+0.9%) and `.rodata` +320 B in
the default build (the file's on-disk size is unchanged at 1,905,376 B, padded
in 64 KiB steps), and only that row. Nothing else is waived. The independent
review judged the row within rule 1's noise margin, so no waiver was strictly
needed; the ruling is given regardless. Numbers:
[tooltips-done](resolved/tooltips-done.md#evidence).

**A runaway item.** One item re-announcing its icon as fast as the bar
re-reads it, for 20 s (`flood.sh` in the PR): the bar read it 434 times
(the 50 ms floor), used 0.14 CPU-seconds (0.7% of a core), made about 68
wakeups a second, and its RSS did not move. Without the floor (the first
version of the module) the same item was read 20,354 times in 20 s and cost
1.93 CPU-seconds (9.7% of a core) and 1,850 wakeups a second, which is what
put the floor in. Many such items are bounded by the 32-item cap. A flood
of 60,000 signals from a peer that is not an item costs the connection
nothing (a real-daemon test), at the price of the bar working through them.

**Against Waybar** (rule 2), the same harness, nixpkgs' Waybar 0.15.0 with
only a `tray` module and the same item on the same private bus, measured
in the first round (not repeated; nothing about it depends on this
commit): RSS 49,528 kB, PSS 42,607 kB, 7 to 8 threads, 14 fds, against
scootbar's 4.0 to 4.2 MB, 2.3 to 2.5 MB, 1 thread and 8 fds with the tray
alone and one item. Its main thread made no context switch in the 60 s
either (23 and 23; the other threads' counters are not comparable, one
thread exited in the window). scootbar is about a twelfth of Waybar's RSS,
and nothing is behind it on a row both have. Yambar's tray is not measured
(no build of it on the box).

**Not measured**, and the rows above do not claim them: the Asahi M2 and
real hardware (this lane had the dev VM only; nothing here depends on a
GPU or a display); the full `scripts/scootbar-bench` rows (startup,
switching CPU), which need the bench runner under a compositor; the soak
(suspend and resume, DPMS, days of uptime) for growth; a real Qt, GTK or
Electron app as the item; yambar. Rule 2 for the tray is therefore passed
against Waybar and **open against yambar**.

## M6 media: module-level cost (measured 2026-10-03)

The [media module](resolved/media-module-done.md) is the second consumer of
the [D-Bus client](resolved/dbus-client-done.md), on its own connection. By
construction: the bus socket is one source in the poll set (with `OUT` only
while a write or staged messages wait), or one inotify watch on the socket's
directory while there is no bus; a one-shot timer exists only while a player
waits out the 50 ms floor between reads of it, while a change of title
waits out the 100 ms between draws, or (30 s) after the bus kept dropping
the bar; no thread, no polling. The bus filters what reaches the
bar: `NameOwnerChanged` of the `org.mpris.MediaPlayer2` namespace and
`PropertiesChanged` of the Player interface on the one MPRIS object, so
unrelated apps and a player's `Seeked` never wake it (a test against a real
`dbus-daemon` sends both, and was checked to fail when the namespace filter
is removed). No new dependency (`Cargo.lock` is unchanged). The contract
test holds the module to the loop's source budget (3 sources at most while
live: the bus and the two timers, of 63).

**Method.** Release builds (`lto = "fat"`, stripped) of `main` at `01c33f09f`
(`c2d82cc95`'s `crates/`; the tooltips PR landed after, see the size note below),
of the branch at code commit `242b2dce7` (`crates/` tree
`864d03f35d18ea3a87f2b421744e929d49a280c0`) with the default features, and of
the same with the default features minus `media`, each built on the dev VM
(aarch64, 6 CPUs, rustc 1.97.1) with its own target dir; a headless `scoot`
(an existing release build, used read-only), a private `dbus-daemon` 1.16.2,
and the players real D-Bus peers: Python on `jeepney`, an independent
marshaller (`player.py`), and a real mpv 0.41.0 with its own MPRIS script. One
bar per run, sampled from `/proc/PID` after 14 s of settling and again 60 s
later (`VmRSS`, `Pss`, voluntary context switches, fds), **one run per row**,
on a VM another agent was building and testing on (load 3 to 7 in the
windows). Wakeup counts are per process and exact for the window; the RSS and
PSS differences under about 130 kB are not resolved by one run. The scripts and
raw logs are in [`bench/m6-media-vm`](../bench/m6-media-vm/README.md).

| Row | RSS kB | PSS kB | wakeups in 60 s | fds |
|---|---|---|---|---|
| `main`, no module placed | 3760 | 2197 | 0 | 7 |
| branch, media built, no module placed | 3824 | 2259 | 0 | 7 |
| branch, media feature off, no module placed | 3824 | 2259 | 0 | 7 |
| `main`, clock | 4180 | 2409 | 2 | 8 |
| branch, media feature off, clock | 4232 | 2472 | 2 | 8 |
| branch, media built and not placed, clock | 4312 | 2537 | 2 | 8 |
| media alone, **no bus** | 4140 | 2407 | **0** | 8 |
| media alone, bus, **no player** | 4160 | 2405 | **0** | 8 |
| media alone, bus, one paused player | 4392 | 2553 | **0** | 8 |
| media alone, bus, one playing player | 4392 | 2552 | **0** | 8 |
| media alone, bus, eight playing players | 4388 | 2543 | **0** | 8 |
| media alone, bus, one real mpv playing a file | 4384 | 2544 | **0** | 8 |
| media alone, bus, mpv playing its `lavfi` sine | 4384 | 2542 | 60 | 8 |
| media and clock, bus, one playing player | 4380 | 2548 | 2 (the clock's) | 9 |

How to read it. **Zero wakeups** with no bus, with a bus and no player, with
one player paused or playing, with eight, and with a real mpv playing a
file: nothing in the module's idle state wakes the bar, and the only wakeups
in the rows with a clock are the clock's two a minute. One thread
throughout. The one row with wakeups is mpv's synthetic `lavfi` source,
whose duration keeps changing, so its script re-sends its (unchanged)
metadata about once a second (60 in 60 s; read off `dbus-monitor`): the
bar parses each, finds nothing changed, draws nothing (one stime tick in
the window) and the row is the player's traffic, not polling. Placing the
module costs the font every module needs (3760 to 4140 kB, with the clock's
4180 for comparison). The first player shown costs about 230 kB RSS over a
bus with none (4160 to 4392 kB: its title's glyphs rasterized and cached,
and the draw), and seven more cost nothing this resolves (4392 with one
playing, 4388 with eight). With the feature built but not placed RSS is +64
kB over `main` with no module placed (3824 against 3760, the same as the
feature-off build's) and +132 kB with the clock (4312 against 4180, against
the feature-off build's +52): the first is not the module's code, and the
second is a little over one run's resolution, with the same sign in the two
earlier runs of this table on earlier trees of the same module (+128 and +132
kB).

**A runaway player.** One stub signalling as fast as its loop runs, for
20 s (`flood.sh`): Position only (10,013 signals): 9,891 bar wakeups, 0.13
CPU-seconds (0.65% of a core), RSS flat at 4216 kB (the tests pin that it
draws nothing and reads nothing); a new title each time (9,583 signals):
9,740 wakeups, 0.16 CPU-seconds (0.8% of a core), RSS 4212 to 4220 kB; the
status flipping between Playing and Paused each time (10,061 signals): 10,216
wakeups, 0.15 CPU-seconds (0.75% of a core), RSS flat at 4212 kB. The runs did
not count draws: that they are ten a second is pinned by the unit tests (the
flap test hears one signal a turn, 400 of them in 10 ms, and counts 400 draws
without the hold). Work per wake is bounded (4 pumps of 64 events), a read of
a player is at most one in flight and 20 a second, and the cost is the
player's to pay in its own signals: about 13 microseconds of bar CPU for a
position, 15 to 17 for a title or a flip. **Held changes were the second
version**: the first drew each change, and a title flood (10,127 signals)
cost 18,754 wakeups and 0.79 CPU-seconds (3.95% of a core), a redraw and the
compositor's release of the frame it replaced for each; the first hold
covered titles only, and review found a state flap (and a switch of the
player shown) still drew per signal, capped only by the pump limit. Every
change but the module appearing or emptying now goes through the 100 ms draw
gap, as the window title's retitles do, and the first change after a quiet
spell is still drawn in its own turn. (Per signal the client makes six small
allocations, owning the header strings and the body: `Conn::dispatch`, not
this module's, and negligible at a player's rates.)

**Binary** (aarch64, stripped), measured with `readelf` because the file
size alone is quantized to 64 KiB steps:

| Build | file bytes | `.text` | `.text`+`.rodata`+`.eh_frame*`+`.gcc_except_table`+`.data*` |
|---|---|---|---|
| `main` | 1,905,376 | 1,457,416 | 1,793,315 |
| branch, default minus `media` | 1,905,376 (+0) | 1,460,168 (+2,752) | 1,796,571 (+3,256) |
| branch, default (media on) | 1,970,912 (+65,536) | 1,501,512 (+44,096) | 1,845,019 (+51,704) |

The media module costs about 41 KB of `.text` and 48 KB of loaded sections
over the feature off (the MPRIS readers, the link and the module); the feature built but off costs 3.3 KB of loaded sections
(edits outside the module that a media-off build still carries: the
window title's ellipsis moved into a shared file, the config and help text),
which does not cross a 64 KiB boundary on disk here. `ldd` still shows only
libc, libm and libgcc_s, and `Cargo.lock` is byte-identical to `main`'s.

**Which rows of the rules regress.** Rule 1 (no row regresses against the
previous milestone beyond noise, except a row the new module adds): the
stripped binary size row regresses by 65,536 B on disk (+3.4%, 51,704 B or
+2.9% of loaded sections), against a `main` from before the tooltips PR
(whose own waived row is above: its +13,440 B of `.text` is not in this
comparison, and the media row is the module's own). Idle wakeups, jiffies,
fds and threads do not regress in any row, with the module placed or not, and
idle RSS and PSS are within what one run resolves (the largest: +80 kB RSS
and +65 kB PSS with the clock over the feature off, 4312 against 4232 kB). The
size row is the same shape as every module before it, and the rule's own
exception covers only a row the module adds, so it was a regression for the
maintainer to waive or not.

**Maintainer's ruling (2026-10-03, given in chat): the stripped-binary size
regression of the media module is waived.** It covers the module's +65,536 B
on disk (+51,704 B of loaded sections, +44,096 B of `.text`) in the default
build, and only that row. Nothing else is waived: no other row regresses
beyond what one run resolves. `media` stays in the default features by the
same decision.

**Not measured**, and the rows above do not claim them: rule 2 for the media
module (a competitor's equivalent: Waybar's `mpris` module is not in the
nixpkgs 0.15.0 build on the box, which is built without `libplayerctl`, so
there was nothing to put beside it; yambar has no MPRIS module), the Asahi M2
and real hardware, the full `scripts/scootbar-bench` rows (startup, switching
CPU), the soak, and a real Spotify, Firefox or Chromium as the player (mpv
and two stubs only; the stubs and mpv name themselves differently and one
sends artists as lists, neither of which proves a browser's habits).

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
  [m4-usage-optimization](resolved/m4-usage-optimization-done.md); the fixed heads re-measured
  ([the README's final stack](../README.md#m4-final-stack-the-fix-round-re-measured))
  show size and idle PSS flagged against `main` on both compositors, and idle
  RSS, heap, CPU, wakeups, startup and the switching CPU not. **Nothing else is
  waived**, and no target moved. (Later the same day the maintainer also
  accepted the idle memory growth and cancelled the optimization: see the last
  Decisions bullet.)
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
  item ([m4-usage-optimization](resolved/m4-usage-optimization-done.md)). Nothing else in the
  ratchet moves.
- **M4's idle memory is accepted as it stands; no order-file machinery** (user,
  2026-10-01, on being shown that a linker order file would buy about 0.25 MiB
  of idle PSS against pre-M4 `main` and 0.45 MiB against the merged stack, for
  about 1,700 lines of build script, tooling and CI: "Let's cancel all this.
  I'm happy with the current size" and "Way too much risk for less than .5mb
  savings"). This **supersedes** the "not accepted, and not waived" half of the
  2026-10-01 bullet above: the idle RSS/PSS growth of M4 (+0.11 to +0.20 MiB,
  idle PSS about 2.24 MiB against `main`'s 2.11) is accepted by the maintainer
  at the numbers measured in [the README](../README.md#m4-final-stack-the-fix-round-re-measured).
  [m4-usage-optimization](resolved/m4-usage-optimization-done.md) is closed
  without a code change; PR #372 was closed unmerged and its branch
  (`perf/scootbar-m4-usage`) kept for reference. The rule itself is unchanged:
  a **later** change that grows an idle row beyond the margin fails the ratchet
  as before. Nothing else is waived.
