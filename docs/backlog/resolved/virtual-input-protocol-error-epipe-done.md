---
title: "virtual-input protocol-error tests race EPIPE after the server closes"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-05"
---

# virtual-input protocol-error tests race EPIPE after the server closes

Filed 2026-10-05. Serves **computer use** (agent-driven input injection is the virtual-keyboard/pointer path; its regression net must not flake) and **daily-drive** (CI must stay green for unrelated PRs).

## The gap

`compositor::virtual_input::tests::key_before_keymap_is_no_keymap` (merged in #448) failed CI on PR #457 (nix-only, unrelated) in the `cargo test -p scoot` step of the `Linux (compositor)` job: `client 0 stopped while waiting for a client step acknowledgement: Err("expected a protocol error containing \"before any keymap\", got: Backend error: Io error: Broken pipe (os error 32)")` at `crates/scoot/src/compositor/test_support.rs:371`. The server posts the protocol error and disconnects the client; the test client sometimes writes after the server closed and sees EPIPE before it reads the error message already sitting in its socket buffer.

Every virtual-input test that expects a protocol error shares the shape: `crates/scoot/src/compositor/virtual_input/tests.rs` `Step::KeyExpectingError`, `Step::ModifiersExpectingError` (explicit `queue.flush()` then `expect_error`'s `roundtrip` loop) and `Step::ExpectError` (round trip whose own `wl_display.sync` write can hit an already-closed socket when the error was forged compositor-side before the step). The `dispatch` suite's refusal tests are not affected (single round trip carries poison and read together); `client_fds`, `drm_syncobj`, `dmabuf/pending_planes` and `fd_pressure` already drain read-side-only on I/O errors (`dispatch_pending`, `prepare_read`/`read`, `dispatch_pending`, then `conn.protocol_error()`).

## What to do

Fix the test harness, not the product: in `expect_error`, make the poison ride the round trip's own flush (drop the separate `flush()`), and on an I/O error drain read-side-only before deciding, following the `client_fds`/`drm_syncobj` `sync` shape. Unify `Step::ExpectError` onto `expect_error`. Prove it: reproduce the before rate under load (`cargo test -p scoot virtual_input` in a loop with parallel CPU load, as #434's flake fix did), then 300 iterations clean after, under both `cargo test` and `cargo nextest`.

## Not in this ticket

Product changes to virtual-input error posting; touching `test_support.rs` (the harness already reports the client's own error correctly — the client just handed it the wrong one).

## Resolution (PR #TBD)

Landed as a test-only change to `crates/scoot/src/compositor/virtual_input/tests.rs`: `Step::KeyExpectingError`/`Step::ModifiersExpectingError` no longer flush separately (the poison rides `expect_error`'s own round trip), `expect_error` takes the `Connection` and on an I/O error drains read-side-only (`dispatch_pending`, one `prepare_read`/`read`, `dispatch_pending`, the `client_fds`/`drm_syncobj` shape) and decides from `conn.protocol_error()`, and `Step::ExpectError` delegates to `expect_error`.

Evidence, all on the Asahi M2 (`~/fx/vflake-4k7d2/`, own `CARGO_TARGET_DIR`, own `XDG_RUNTIME_DIR`, headless):
- Before (pre-fix tree): 100 iterations of the `virtual_input` module under 8x CPU load → 3 failures, every one `modifiers_before_keymap_is_no_keymap` with the CI-identical `test_support.rs:371` `Broken pipe (os error 32)` signature.
- After: 300/300 clean under the same load via the `cargo test` runner, 300/300 clean via `cargo nextest run -p scoot virtual_input`.
- Full runs: `cargo test -p scoot` 2124 passed 0 failed; `cargo nextest run --workspace` 4364 passed 36 skipped 0 failed; `cargo clippy -p scoot --all-targets -- -D warnings` clean; `cargo fmt --check -p scoot` clean; `scripts/smoke-test.sh` rc=0 with 36 `ok:` lines.
