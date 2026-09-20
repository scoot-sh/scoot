---
title: "Flake class: dispatch flood tests die on fd-pressure kills under a pressured process table — DONE."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Flake class: dispatch flood tests die on fd-pressure kills under a pressured process table — DONE

RESOLVED 2026-09-20 (test-only; no production code changed). Both dispatch
halves run under the icon test's treatment from PR #167: a raised
`RLIMIT_NOFILE` ceiling with verified headroom (fast loud panic naming
the numbers where the table cannot fit the fill, never a skip), and the
final refusal pinned to the budget by its message (same code+object as
the pressure refusal on both factories). The single-pixel flood, which
took no flood lock at all, now takes the shared `FD_FLOOD_LOCK` like
every other flood.

Filed 2026-09-20 from `scoot-reviewer`'s pass on PR #167 (the icon-buffer
flake fix), which defused the same shared-table class in
`toplevel_icon/tests.rs` and explicitly scoped the dispatch halves out.
Recommend filed-not-fixed there; this entry is the filing.

## What the reviewer proved (not reasoned)

Reproduced deterministically on the dev VM:
`dispatch/tests.rs`' `a_dmabuf_create_past_a_full_budget…` under
`prlimit --nofile=650:650` dies red with `code: 1, object "wl_shm_pool",
message: "…file-descriptor pressure…128-buffer pressure grace"` where the
test expects code 7 on the dmabuf params. Mechanism: the fill-phase kill
lands on the wrong cause because the live count crosses grace-128 under a
pressured process table, and the test has no headroom check and no cause
pin — `assert_raw_protocol_error` checks code+interface only.

The earlier `BrokenPipe` signature the implementer saw once under 16-thread
pre-fix load is a race on the same early-kill event (EPIPE on the next flush
write vs reading the pending protocol error first), which reconciles the
two signatures: same kill, different observer.

## The harder half

`flooding_single_pixel_buffers…` (`dispatch/tests.rs:1150`): exposure
confirmed by construction (513 creations past grace-128, asserts only
code+interface, takes no `FD_FLOOD_LOCK`, holds no fds so never
self-pressures — it needs a concurrent fd-holder overlapping its
microsecond grace-crossing), but 6 tightened-table pair runs with the shm
flood stayed green, so that half is unreproduced. Worse: a pressure kill
there is *assertion-invisible* — budget and pressure refusals post the
identical code 0 on the identical manager object — so only the EPIPE race
can redden it, which explains both its rarity and its errno.

## What the fix looks like

The icon-test treatment from PR #167, applied to both sites
(`dispatch/tests.rs:1069` + `:1150`): a headroom check up front (loud fail,
never a skip — a skip stops guarding the bound) and cause-pinning by
message, which is required in the single-pixel case, not optional (code 0
on the same object cannot discriminate). Shared root, unchanged by this
entry: `pressure_refusal` (`dispatch.rs:1239`) over the process-global
`fd_pressure::table()`.

## Out of scope

Production behavior (the pressure refusal itself is correct — this is
test-only), the `icon_buffers_fill…` test (fixed in PR #167), CI/VM ulimit
changes.

## Resolution: the icon treatment, adapted to what these fills retain

Test-only (`compositor/dispatch/tests.rs`); no production code changed.

- **Headroom, parameterized by retention.** New
  `ensure_dispatch_flood_headroom(retained)` reuses the sibling's
  vocabulary whole — `TARGET_SOFT` 4096 (the same number, so one ceiling
  covers the whole suite), `NEIGHBOUR_SLACK` 256 with the same sizing
  reasoning, raise-only-then-verify, fast loud panic naming
  used/soft/need, never a skip — and adapts only the need to what each
  fill actually retains: 512 + 128 (`RESERVE_FDS`) + 256 = 896 free for
  the dmabuf fill (one server fd+mapping per retained shm buffer, client
  peak 64 by the 64-chunk flush), 0 + 128 + 256 = 384 free for the
  single-pixel flood, which holds no fd at all and trips only on a
  neighbour-pressured table.
- **Cause pinned by message on both.** New
  `assert_raw_protocol_error_with_message` keeps the old code+interface
  assertions and adds the server message: both factories' twins share
  code+object (7 on `zwp_linux_buffer_params_v1`, 0 on
  `wp_single_pixel_buffer_manager_v1`), so only the message proves the
  budget said no. Both pins assert the `too_many_buffers()` text
  (`maximum of 512 live buffers`), which the pressure twin
  (`too_many_buffers_under_pressure()`) never carries. The old helper is
  untouched for the tests outside this ticket's scope.
- **Lock membership fixed.** The single-pixel flood took no
  `FD_FLOOD_LOCK` — the audit the ticket asked for confirmed it was the
  only flood outside the lock — and now takes it. The dmabuf test already
  participated.
- **Lock and check moved to the test thread.** First cut put both inside
  the `drive` offender closure (where the buffer floods take the lock),
  and the tightened-table proof showed why that is wrong here: a failed
  check panics the client thread, strands the dispatch loop, and the run
  dies on the 10s deadline instead of the headroom numbers. Both ticket
  tests now take the lock and check headroom on the test thread before
  `drive` — the same shape the pool-count floods already use — so a
  short table fails in 0.00s with the numbers. Single mutex, taken once
  per test, never nested: no deadlock shape (verified by reading every
  `hold_flood_lock` site; the in-closure acquisitions in the out-of-scope
  buffer tests are untouched).

## Evidence (dev VM, 2026-09-20)

Pre-fix at `edebefd`, binary
`/var/cargo-target/debug/deps/scoot-9de053b31125ddf7`
(`ulimit -Sn` 1024 / `-Hn` 524288, 4 vCPU):

| runs | mode | result |
|---|---|---|
| 1 | dmabuf-`create` test solo, `prlimit --nofile=650:650` | **red, the ticket signature**: `code: 1` on `wl_shm_pool` with the `…file-descriptor pressure…128-buffer pressure grace` message where code 7 on the dmabuf params is expected (0.14s) |
| 1 | single-pixel test solo, `prlimit 650` | green (0.03s) — unreproduced, as the ticket says |
| 2 | single-pixel + shm flood pair shapes, `prlimit 650` | green — unreproduced |
| 1 | full `dispatch::tests` suite (31 tests), `prlimit 650`, 4 threads | 28 passed, **3 failed**: the ticket's `create` half plus `a_dmabuf_immed_…` (byte-identical mechanism, code 1 vs expected 7) and `a_second_client_buffers_…` (fill refused mid-way, client-thread panic → 10s deadline) — both out of this ticket's scope, see below |

Post-fix, same binary path rebuilt from the branch (clippy `-D
warnings` and `fmt --check` clean):

| runs | mode | result |
|---|---|---|
| 1 | dmabuf-`create` test solo, `prlimit 650` | fast loud panic (0.00s, not the wrong-cause red and not the 10s deadline): `no deterministic headroom for the dispatch flood: table is 4/650 used/soft, need 896 free (512 retained + 128 reserve + 256 neighbour slack); raise this process's RLIMIT_NOFILE hard limit` |
| 1 | single-pixel test solo, `prlimit 650` | green (0.03s) — its 384-free need fits a 650 table, so it runs and passes |
| 6 | full binary, default parallelism | 6/6 green, 1080 passed each, ~20.6s each |
| 4 | full binary, `--test-threads=16` | 4/4 green, 1080 passed each, ~6.7s each |
| 2 | `cargo nextest run --workspace` | first: 1210/1211 PASS, one unrelated `scootctl::msg_broken_pipe a_full_read_is_unchanged` stuck >840s (different crate; passes solo in 0.00s — transient/environmental); rerun: **1211 passed, 4 skipped** in 32s |

The message pins are proven live, not vacuous: every post-fix green
run passed *through* the message assertion (a wrong-cause kill would
redden it — the pressure text carries no `maximum of 512 live buffers`
substring, verified against both refusal constructors in
`dispatch.rs`).

## Left out, stated plainly

- **`a_dmabuf_immed_past_a_full_budget_is_refused_before_validation`**
  reproduces byte-identically under `prlimit 650` (same fill, same
  wrong-cause kill, code 1 vs expected 7) and is **not** hardened here:
  the ticket scopes this item to the two tests and names any other test
  explicitly out. Same two-line treatment applies whenever it is filed —
  [since applied](dispatch-flood-remainder-flake-done.md).
- **`a_second_client_buffers_while_the_first_sits_at_the_cap`** likewise
  dies under `prlimit 650` (fill refused mid-way) and is likewise out of
  scope — its assertion shape differs (it asserts the fill *succeeds*,
  so it wants the headroom half only, no message pin) —
  [since applied](dispatch-flood-remainder-flake-done.md).
- **The single-pixel half never reproduced** (green solo, green paired,
  green in the pressured suite run) — hardened per the ticket's shape
  anyway (lock + headroom + required message pin, which converts any
  future pressure kill from assertion-invisible to loud red), documented
  here rather than claimed.
- No VM hardware beyond test runs was needed. `git diff --stat`
  confirms zero non-test, non-doc files changed.
