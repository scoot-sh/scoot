---
title: "A global fd/buffer ceiling across Wayland connections — RESOLVED (compositor-wide pressure ceiling with shed strategy)."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# A global fd/buffer ceiling across Wayland connections — RESOLVED (compositor-wide pressure ceiling with shed strategy).

## The entry as filed

Split out of [the Wayland connection-cap
verdict](./wayland-connection-cap-done.md) (2026-09-18), which fixed the
compositor-killing half and closed the connection-count half as an
accepted tradeoff. What remained open was the residual: per-connection
bounds (512 buffers, 128 pools, 8 binds, 16 capture frames) still multiply
across connections, so two greedy connections hold ~1039 fds against the
1024-fd table with nothing tripped. The verdict decided a shared ceiling
against *for now* for one reason: the refusal form, not the accounting --
a shared ceiling fires on an innocent client for another's greed, harsher
than the IPC cap's refused-with-reason and the shape `bind_budget.rs`
deliberately refused to build. Landing it needed the refusal designed
first.

## Verify-first: what the numbers actually say

Re-derived against the current code and live measurement (dev VM,
2026-09-18), not the ticket prose:

**1. Global pressure is observable, portably-enough.** `/proc/self/fd`
entry count against `getrlimit(RLIMIT_NOFILE)` soft. Both primitives were
already used in-tree (the accept-loop exhaustion tests). The compositor
is Linux-only (`mod compositor` is `cfg(target_os = "linux")`), so no
fallback path exists or is needed; any observation failure (a failed
`getrlimit`, an infinite limit, an unreadable `/proc`) returns `None`
and every enforcement site admits on `None`. Shedding on unknown would
deny innocents for a broken gauge; the `EMFILE` shed still catches real
exhaustion underneath.

**2. Normal usage, measured.** Idle `--headless`: **14 fds**. Plus one
`foot` window with a shell: **17** (delta +3: one socket, two pools;
buffers retain pool fds rather than opening new ones). The shell's own 16
fds live in its own process, not this table -- the ticket's "a shell at
startup opens MANY fds transiently" concern is client-side; compositor
cost per app window is ~3-4 fds. A login storm (bar, panels, launcher, a
handful of apps, ~30 connections) lands near **100-150** by the same
per-connection arithmetic. One connection at every hard cap holds
512 + 128 + 1 ≈ **641 fds** -- one such connection plus a normal session
(~750) never trips anything here, correctly, since a single connection
cannot exhaust the table alone.

**3. The refusal form the verdict asked for.** Two halves, matching the
two things a ceiling must do:

- *Newcomers shed, nobody killed.* While fewer than 128 fds stand free,
  a Wayland newcomer is dropped (immediate EOF -- there is no protocol
  channel for a reason, the same wire shape as the `EMFILE` shed) and an
  IPC newcomer is refused with a reason naming the pressure (the same
  shape as the 64-slot cap refusal -- IPC has a channel for it). No slot
  is claimed, nothing established is touched, and recovery is immediate
  when pressure lifts for held connections: no back-off, nothing disabled.
  (Connect-and-instantly-die bursts reap slowly -- minutes for a
  several-hundred-deep pile, all fds eventually returned via pre-existing
  Smithay cleanup this PR doesn't touch -- so newcomers keep shedding
  through that lag. Review-observed on PR #123, not a regression.)
- *Creations refuse past a per-client grace, never onto innocents.*
  While the table is pressured, a client already holding past **128 live
  buffers** or **64 live pools** is refused its next creation with the
  same protocol error the per-connection cap would post
  (`InvalidStride` / `InvalidWlBuffer` / bare 0 per interface). A client
  under grace -- every legitimate client, at 64x/32x the measured
  single-window floor and ~2x/~1.6x the heaviest reasoned legitimate use
  -- is never refused for another's greed. The grace is what makes this
  a ceiling rather than a lottery, and what answers the verdict's
  objection: the kill always lands on a contributor, whose disconnect
  then frees what it held, so the pressure it caused lifts with it. Two
  connections sitting exactly at grace hold 2 x (128 + 64 + 1) + 14
  baseline = 400 fds -- pressure still requires someone past grace, so
  the refusal cannot land anywhere else.

**4. Sizing.** `RESERVE_FDS` 128 (trip past 896 of 1024): ~6x above the
reasoned login storm, still room for a whole greedy connection's burst,
and the two-greedy fill trips the creation guard with the second greedy
near ~370 of its 512 buffers -- before exhaustion, not after.
`MIN_TABLE_FDS` 512: below it the guard stays off entirely and the
`EMFILE` shed is the only backstop -- a 128 reserve on a 256-fd table
would shed a normal login storm, so the honest answer is no guard
rather than a hair-trigger one. Observation costs ~8us per call (dev
VM, debug build, 2000-call sample): once per *connection* at the accept
sites, and at the creation sites only once a client is already past its
grace (a `HashMap` lookup short-circuits everything under it, and the
`TypeId` gates keep every non-creation request at zero added cost).

## Resolution (2026-09-18, this PR)

New `compositor/fd_pressure.rs`: one observation (`table()`), one pure
predicate (`Table::pressured`, with `free()` saturating rather than
wrapping), the four numbers above with the full sizing argument in the
module doc, and the fork-child discipline stated (`table()` allocates,
so never from `drain`/`shed_one`/`classify` or anything a forked
exhaustion-test child executes -- every call site runs on the loop
thread).

Enforcement at three sites, all reading the same predicate:

- `wayland_accept::admit` (new; the `listen` callback in `state.rs`
  calls it with the live table): pressured means drop (EOF), unknown or
  calm means the pre-existing `insert_client`. Never touches the spare,
  so a pressure that deepens into exhaustion still finds it armed -- no
  double-shed weirdness with PR #97's accept shed (mutually exclusive
  per backlog entry: accepted reaches here, failed-`accept` reaches the
  shed arm).
- `ipc::accept_under` (new; `accept()` reads the live table and
  delegates): pressured means the static `PRESSURE_REFUSAL` line plus
  close, before the slot claim, after the uid check and the non-blocking
  set (the write must not block either). Unknown or calm falls through
  to the pre-existing cap path untouched.
- `dispatch.rs`'s pool claim and all three buffer claims: a
  check-before-claim pressure rule (`pressure_refusal(live, grace)`)
  ahead of the per-client claim, so a pressure refusal never takes a
  count unit -- balanced by construction, not by a compensating release
  (strictly cleaner than the cap path's phantom-unit shape). Refusal
  codes are each interface's own; only the messages are new, naming the
  pressure and the grace.

What is deliberately not here: creation-time wait/queue/silent-ignore
(no channel carries "retry later", and silent ignore panics the
compositor), any connection-count cap (the verdict stands), and any
change to the per-connection numbers (all untouched).

## Tests

Thirteen new, all fail-first (each toggled and restored; the exact
neuter runs are below):

- Seven in `fd_pressure/tests.rs`: the reserve boundary trips exactly
  (`<`, not `<=`), full table pressured, used-past-soft saturates to
  zero free rather than wrapping, small tables calm whatever they hold
  (mirroring `table()`'s `None`), free-is-soft-minus-used, and the live
  observer agreeing with the kernel (count sane, soft matching an
  independent `getrlimit`). Neutered `pressured()` (`false`): 5 fail,
  2 pass by design.
- Three in `wayland_accept/tests.rs` around `admit` with canned tables
  and a real `State`: pressured gets EOF with no bytes (shed arm),
  calm stays open (WouldBlock -- nothing is written before dispatch),
  unknown admits (fail-open). Forced-insert neuter: exactly the shed
  test fails. Forced-shed neuter: exactly the two admit tests fail.
- Three in `ipc/connection/tests.rs` through `accept_under`: pressured
  gets `Response::Error` naming the pressure plus close plus no slot
  taken plus a later calm newcomer still served; calm and unknown both
  answer `Version` (fail-open pinned, not assumed). Pressure-arm-skipped
  neuter: exactly the pressure test fails (by deadline, no answer ever
  comes). Always-refuse neuter: exactly the two admit tests fail.

The pre-existing flood suites are the no-false-positive canaries: they
hold ~600 process fds with exact-count assertions (512 accepted, then
refused with the cap's code), so a guard misfiring early fails them
loudly. All green unchanged.

## Verification

Standard set, dev VM (`ssh -p 2222 dev@localhost`,
`CARGO_TARGET_DIR=/var/cargo-target`, 9p mount at `/mnt/flexwm`),
final tree:

```
cargo test -p flexwm            TEST EXIT=0  965 passed; 0 failed; 1 ignored
                                         (+ 3 passed in the integration binary)
cargo nextest run --workspace   NEXTEST EXIT=0  1070 run: 1070 passed, 1 skipped
cargo clippy -p flexwm --all-targets -- -D warnings   CLIPPY EXIT=0 (0 warnings)
cargo fmt --check -p flexwm     FMT EXIT=0 (also Mac-side)
MODE=--headless scripts/smoke-test.sh   (below)
```

Live pressure cycle, dev VM 2026-09-18 (branch debug binary,
`--headless`, soft pinned to 520 with `prlimit` -- note the bare
`--nofile=520` form sets hard too, so recovery below is proven by
draining the horde rather than by restoring the limit, which is the
stronger demonstration anyway):

- Calm: 14 fds idle, 17 with `foot` (`msg windows` shows id 1 mapped).
- Horde: ~420 `nc -U wayland-1` against the 520 table. 374 admitted
  (391 fds), 46 shed at the trip (see next).
- Pressure: every further newcomer shed -- 52 total `WARN ... shed a
  pending wayland connection (fd pressure ...) used=393 soft=520
  free=127` in the log, six probed `nc` all exit 0 with 0 bytes, fds
  pinned at 391, process alive. `msg version` answered `refused: flexwm
  is under file-descriptor pressure (fewer than 128 fds free); retry in
  a moment ...` (exit 1). `foot` (pid 2569029) alive throughout.
- Recovery: `pkill -x nc` (exact name -- the `pkill -f` pattern would
  match the invoking `bash -c` itself, the documented gotcha) drains
  the horde to 0, compositor back to 17 fds. A fresh `nc` stays
  connected past 3s (held, not shed), `msg version` answers normally,
  `msg windows` still lists `foot` id 1, and a 1600x1000 screenshot
  proves it is serving, not a zombie entry.

Coverage stated honestly: the over-grace creation-kill branch is
logic-tested (`pressured()` boundaries) and live-justified (same
observer the accept shed proves end to end), but no in-suite test
drives a real dispatch past grace under pressure -- filling 900 fds in
the test process would starve every sibling sharing its table, so that
branch is unreachable in-suite by construction. Under-grace survival is
covered live (`foot` untouched throughout) and by the flood canaries.

## Adjacent, named rather than fixed here

- A greedy connection that stops creating at 500 buffers is never
  killed by the creation guard (no next creation to refuse) -- but it
  also never grows, newcomers shed, innocents under grace are
  unaffected, and everything recovers when it leaves. Stable-degraded,
  disclosed, no action.
- `MIN_TABLE_FDS` (512) leaves sub-512 tables to the `EMFILE` shed
  alone -- status quo ante there, by design.
