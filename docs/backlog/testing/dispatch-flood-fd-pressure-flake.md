---
title: "Flake class: dispatch flood tests die on fd-pressure kills under a pressured process table"
status: "open"
area: "testing"
priority: "low"
blocked: null
---

# Flake class: dispatch flood tests die on fd-pressure kills under a pressured process table

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
