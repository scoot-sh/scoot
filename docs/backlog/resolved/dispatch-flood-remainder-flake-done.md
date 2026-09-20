---
title: "Same fd-pressure flake class in two more dispatch flood tests (`immed`, second-client) — DONE."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Same fd-pressure flake class in two more dispatch flood tests — DONE

RESOLVED 2026-09-20 (test-only; no production code changed). Both
remaining dispatch floods run under the PR #168 treatment a third time:
`FD_FLOOD_LOCK` plus `ensure_dispatch_flood_headroom` on the test thread
before `drive`/`drive_both` (fast loud panic naming the numbers where the
table cannot fit the fill, never a skip, never the 10s deadline), and the
`immed` refusal pinned to the budget by its message. The second-client
test takes the headroom half only — it asserts the fill *succeeds*, so
there is no kill to discriminate.

Filed 2026-09-20 from `scoot-reviewer`'s pass on PR #168, which hardened
two dispatch floods against fd-pressure kills and explicitly scoped these
two out. The reviewer confirmed both are real and correctly left out;
this entry was the filing.

## `a_dmabuf_immed_…` (`dispatch/tests.rs:1244`)

Was the old shape (in-closure lock, code+interface-only assert, no
headroom) with the byte-identical fill as the fixed `create` half, and it
reproduced red under `prlimit 650` in PR #168's suite table. Got the same
two-line treatment: headroom check + cause-pinning by message.

## `a_second_client_buffers_…` (`dispatch/tests.rs:1071`)

Asserts the fill *succeeds*, so it got the headroom half only — no
message pin (there is no kill to discriminate). Reproduced red under
`prlimit 650` the same way (fill refused mid-way → 10s deadline).

## Constraint (from the same review, honoured)

Note 4 on PR #168: residual in-closure lock acquisitions remained at
`tests.rs:984, 1013, 1073, 1246`, and the fixture strands an in-closure
panic on its 10s dispatch deadline (`drive_both`, lines 254-275 — a
closure panic never sets `finished`). Both headroom checks here went on
the test thread per PR #168's precedent, not the neighboring in-closure
shape: each test takes `hold_flood_lock()` and calls
`ensure_dispatch_flood_headroom` before `drive`/`drive_both`, and the
in-closure acquisitions are removed. Single mutex, taken once per test,
never nested: no deadlock shape (the client thread no longer acquires in
either test). The two remaining in-closure sites (`:984, :1013` — the
bypass-loop cap tests, out of this ticket's scope) are untouched.

Headroom need is 896 free for both (512 retained + 128 `RESERVE_FDS` +
256 `NEIGHBOUR_SLACK`): the `immed` fill retains like `create`, and the
second-client fill sits at the cap on the success path, so both size for
512 retained.

## Out of scope (unchanged)

Production behavior (the pressure refusal is correct — test-only, like the
siblings), CI/ulimit changes. See the sibling records for the shared
vocabulary (`FD_FLOOD_LOCK`, `TARGET_SOFT` 4096, `NEIGHBOUR_SLACK` 256,
raise-only-then-verify, loud-panic-never-skip):
`icon-buffer-budget-fd-pressure-flake-done.md`,
`dispatch-flood-fd-pressure-flake-done.md`.

## Resolution

Test-only (`compositor/dispatch/tests.rs`, commit `03d666e`); no
production code changed (`git diff 53ec6d1 03d666e --stat` shows the one
test file).

- **`a_dmabuf_immed_past_a_full_budget_is_refused_before_validation`:**
  lock + `ensure_dispatch_flood_headroom(512)` on the test thread, and
  the final assert widened to `assert_raw_protocol_error_with_message`
  with `maximum of 512 live buffers` (both causes post 7 on
  `zwp_linux_buffer_params_v1`, so only the message proves the budget
  said no). Doc comment extended to name the treatment, mirroring the
  `create` half.
- **`a_second_client_buffers_while_the_first_sits_at_the_cap`:** lock +
  `ensure_dispatch_flood_headroom(512)` on the test thread; assertion
  shape untouched (no kill to pin). Doc comment extended to say why
  headroom-only.
- `assert_raw_protocol_error` stays for the tests outside this ticket's
  scope (e.g. the failed-import kill); no helper added, no shared helper
  changed.

## Evidence (dev VM, 2026-09-20)

Pre-fix at `53ec6d1`, binary
`/var/cargo-target/debug/deps/scoot-9de053b31125ddf7`
(`ulimit -Sn` 1024 / `-Hn` 524288, 4 vCPU):

| runs | mode | result |
|---|---|---|
| 1 | `immed` test solo, `prlimit --nofile=650:650` | **red, the ticket signature**: `code: 1` on `wl_shm_pool` with the `…file-descriptor pressure…128-buffer pressure grace` message where code 7 on the dmabuf params is expected (0.14s) |
| 1 | second-client test solo, `prlimit 650` | **red, the ticket signature**: fill refused mid-way (`filling to the cap is served` fails on the client thread with the pressure kill) → 10s dispatch deadline (`the client thread never finished`) |
| 1 | full `dispatch::tests` suite (31 tests), `prlimit 650` | 27 passed, **4 failed**: the two ticket tests plus the two already-hardened halves (`create`, single-pixel), which now fail fast-loud on headroom by design |

Post-fix at `03d666e` (evidence tree identical to the commit — the
working tree was clean at commit time), same binary path rebuilt,
clippy `-D warnings` and `fmt --check` clean:

| runs | mode | result |
|---|---|---|
| 1 | `immed` test solo, `prlimit 650` | fast loud panic (0.00s, not the wrong-cause red): `no deterministic headroom for the dispatch flood: table is 4/650 used/soft, need 896 free (512 retained + 128 reserve + 256 neighbour slack); raise this process's RLIMIT_NOFILE hard limit` |
| 1 | second-client test solo, `prlimit 650` | fast loud panic (0.00s, not the 10s deadline) |
| 1 + 1 | both tests solo, normal table | green (0.18s, 0.17s) |
| 1 | full `dispatch::tests` suite (31 tests), normal table | 31 passed (0.73s) |
| 6 | full binary, default parallelism | 6/6 green, 1080 passed each, ~20.5–20.8s each |
| 4 | full binary, `--test-threads=16` | 4/4 green, 1080 passed each, ~6.6–7.0s each |
| 2 | `cargo nextest run --workspace` | 2/2 green: **1211 passed, 4 skipped** in ~30–32s each |

The message pin is proven live, not vacuous: every post-fix green run
passed *through* the message assertion (a wrong-cause kill would redden
it — the pressure text carries no `maximum of 512 live buffers`
substring, verified against both refusal constructors in
`dispatch.rs` and the pre-fix red output above).

## Left out, stated plainly

- **The `msg_broken_pipe` accept-hang** (separate ticket, different
  mechanism) — explicitly out per the ticket; untouched. (The two
  post-fix nextest runs above both passed it without the >840s stick
  PR #168 saw once.)
- **Residual in-closure locks** in the two bypass-loop cap tests
  (`retaining_a_buffer_past_its_pool…`, `destroying_a_pool…`) — out of
  scope per the ticket; a headroom failure there still strands the 10s
  deadline, same as before.
- No VM hardware beyond test runs was needed. `git diff --stat`
  confirms zero non-test, non-doc files changed.
