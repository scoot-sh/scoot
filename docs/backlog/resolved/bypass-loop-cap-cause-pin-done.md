---
title: "Pin the budget cause by message in the two bypass-loop cap tests — DONE."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Pin the budget cause by message in the two bypass-loop cap tests — DONE

RESOLVED 2026-09-20 (test-only; no production code changed). Filed
2026-09-20 from `scoot-reviewer`'s pass on PR #169, which hardened the
neighboring dispatch floods and explicitly scoped these two out. Not a
flake (never reddens) — a vacuous guard, which is quieter and worse:
`retaining_a_buffer_past_its_pool…` and `destroying_a_pool…`
(`dispatch/tests.rs:982-1004`, `1011-1036` at filing time) asserted via
`assert_shm_protocol_error` (code+interface only), and per
`dispatch.rs:817-838` the pressure and budget refusals on
`wl_shm_pool::CreateBuffer` post the **identical** code
(`InvalidStride`) on the **identical** object — so under a pressured
table the kill lands at ~grace-129 with the pressure cause and the test
still passes green, guarding the pressure path instead of the 512-cap.

## Stance decision (the ticket's design question, as shipped)

Per-call-site choice, exactly as the coordinator decided — **not** a
helper-contract change. `assert_shm_protocol_error` is byte-identical
before and after (`git diff` shows the one test file only); its "stays
valid whichever side answered" stance stands for its other users, where
no twin shares code+object. These two call sites switch to the
siblings' `assert_raw_protocol_error_with_message` with
`maximum of {MAX_BUFFERS_PER_CLIENT} live buffers`, and each site
carries a comment saying why it discriminates by message while the
others don't: shared code+object makes code-only vacuous here, and only
here. No helper signature changed, no new shared helper — the call
sites needed nothing that wasn't already there
(`assert_raw_protocol_error_with_message` takes a numeric code, so the
sites pass `wl_shm::Error::InvalidStride as u32`, keeping the enum
linkage the code-only helper had).

The discrimination was verified in source, not assumed:
`too_many_buffers()` (`dispatch.rs:1284-1289`) renders
`wl_buffer refused: this client already holds the maximum of 512 live
buffers` (`MAX_BUFFERS_PER_CLIENT` is 512, `wl_buffers.rs:213`), while
`too_many_buffers_under_pressure()` (`dispatch.rs:1294-1299`) renders
`wl_buffer refused: compositor-wide file-descriptor pressure, and this
client holds more than the 128-buffer pressure grace` — the budget
substring is present on the budget path and absent from the pressure
path, so the pin cannot be satisfied by the wrong cause.

## Resolution

Test-only (`compositor/dispatch/tests.rs`); `git diff --stat` shows the
one test file plus this record and the index/roadmap lines.

- **`retaining_a_buffer_past_its_pool_trips_the_buffer_cap`:** final
  assert widened to `assert_raw_protocol_error_with_message` with the
  budget message, with the full why-comment (shared code+object, the
  `prlimit 650` proof, the helper stance standing elsewhere).
- **`destroying_a_pool_with_live_buffers_keeps_every_buffer_counted`:**
  same widening, with a shorter why-comment pointing at the same reason
  (these two sites need the discrimination, not the helper a change).

## Evidence (dev VM, 2026-09-20)

Pre-fix at `60d54d6`, clean tree, binary
`/var/cargo-target/debug/deps/scoot-9de053b31125ddf7`:

| runs | mode | result |
|---|---|---|
| 1 | `retaining…` solo, `prlimit --nofile=650:650` | **green-while-guarding-pressure, the ticket signature**: `Protocol error 1 on object wl_shm_pool@6: wl_buffer refused: compositor-wide file-descriptor pressure, and this client holds more than the 128-buffer pressure grace`, test `ok` (0.12s) |
| 1 | `destroying…` solo, `prlimit --nofile=650:650` | same signature: pressure message, test `ok` (0.13s) |

Pin-bite proof (both pins temporarily pointed at the pressure text,
normal table — Mac-side edit, restored + `touch`ed after):

| runs | mode | result |
|---|---|---|
| 1 | `retaining…` solo, normal table, pin at pressure text | **red, the pin fires**: `the refusal came from the wrong cause (budget and fd-pressure share code+object): ProtocolError { code: 1, object_id: 6, object_interface: "wl_shm_pool", message: "wl_buffer refused: this client already holds the maximum of 512 live buffers" }` (0.18s) |
| 1 | `destroying…` solo, normal table, pin at pressure text | red the same way (0.18s) |

Post-fix (pins correct, tree reverted clean — `git diff` shows only the
two intended call sites), same binary path rebuilt, clippy `-D
warnings` and `fmt --check` clean:

| runs | mode | result |
|---|---|---|
| 1 | `retaining…` solo, `prlimit --nofile=650:650` | **red, loud by design**: the vacuous green is now a wrong-cause failure (0.13s) — a pressured table genuinely cannot fit the fill, and the pin says so instead of passing |
| 1 + 1 | both tests solo, normal table, `cargo test --exact` | green (0.19s, 0.18s) |
| 1 + 1 | both tests solo, normal table, nextest | green (0.193s, 0.165s) |
| 1 | `cargo nextest run --workspace` | **1211 passed, 4 skipped** (~32s) |

The message pins are proven live, not vacuous: every post-fix green run
passed *through* the message assertion, and the two red proofs above
show a wrong-cause kill reddens it in both directions (pressure text
demanded where the budget answered, budget text demanded where pressure
answered).

## Left out, stated plainly

- **No headroom machinery** (`ensure_dispatch_flood_headroom`,
  RLIMIT changes): the ticket scoped these tests as needing none, and
  the reproduction confirms it — under a normal table both are green
  through the pin, and under a pressured table they now fail loudly
  rather than passing vacuously, which is the intended signal (the
  table genuinely cannot fit a 512-retaining fill), not a flake to
  silence. CI/ulimit changes likewise untouched.
- **The `msg_broken_pipe` accept-hang** (separate ticket, different
  mechanism) — explicitly out per the ticket; untouched.
- **Residual in-closure `hold_flood_lock()`** in these two tests —
  out of scope per the ticket (no headroom check inside the closures,
  so no in-closure panic to strand; the test thread holds nothing, so
  no deadlock shape). Untouched.
- No VM hardware beyond test runs was needed. `git diff --stat`
  confirms zero non-test, non-doc files changed.
