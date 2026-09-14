---
item: "7"
title: "wl_shm_pool.resize(0) crash-DoS"
status: "done"
area: "security"
pr: 12
commit: null
---

# wl_shm_pool.resize(0) crash-DoS

The bug is upstream, in the pinned Smithay rev (`0ff00983`,
`src/wayland/shm/handlers.rs:185-190`): the `wl_shm_pool.resize` handler
posts a protocol error for `size <= 0` but is **missing the `return`**
after it, so `size == 0` falls through into
`NonZeroUsize::try_from(0).unwrap()` and panics. `[profile.release]` sets
`panic = "abort"`, so this was not the contained single-client protocol
error `state.rs`'s dispatch-site comment assumes — it aborted the process
and every connected client's session with it. Trigger: any client,
`wl_shm.create_pool(fd, 1)` then `wl_shm_pool.resize(0)`. No crafted fd
contents, no privilege. Still present on Smithay `master` as of this
date, so there was no version bump to wait for.

Fixed inside flexwm, with no fork of Smithay — the fallback option, which
would have needed an externally-hosted one-line fork behind a Cargo
`[patch]`, turned out not to be necessary. What forced the shape:
(a) this rev has no `delegate_shm!` to partially override —
`delegate_dispatch2!` generates a single *blanket* `Dispatch` impl over
every interface, and without specialization any per-interface impl
overlaps it (E0119), so the blanket impl is the only seam flexwm owns;
(b) the valid-size path can't be reimplemented locally — `ShmPoolUserData`'s
only field is private and `shm::pool::Pool` isn't exported. So
`compositor/dispatch.rs` now holds a hand-written copy of what the macro
expanded to, plus one guard that rejects `size <= 0` before Smithay sees
it; `size > 0` still goes to Smithay untouched.

The guard is on the per-request path, so it's written to fold away:
`TypeId::of` is a `const fn`, so after monomorphization both sides of its
first comparison are compile-time constants and the body disappears for
every interface other than `wl_shm_pool`. Measured, not assumed — 1M
`wl_surface.damage` requests, compositor CPU in jiffies, 5 reps:
before 49/35/32/36/37, after 35/35/34/32/34 (~1.6-1.8M req/s either way).
Overlapping ranges, no measurable cost.

One documented claim was **wrong until hardware disproved it**: the first
draft said negative sizes would change error message from "mremap failed"
to "invalid wl_shm_pool size". They don't — upstream already posted
exactly that error before falling through, and only a client's *first*
protocol error is ever delivered, so negative sizes are byte-identical
before and after (verified on release builds both ways). What the guard
does drop for them is a pointless trip through `Pool::resize`, whose
`MemMap::remap` unmaps the existing mapping *before* discovering the new
one can't be made.

Verified on the dev VM against real `--headless` **release** binaries
(`panic = "abort"` is what makes the difference visible): before, the
compositor died with `Aborted (core dumped)`, exit 134/SIGABRT, and a
second client got "Connection refused"; after, it stays up, the offender
alone is disconnected, and a fresh client still sees all 9 globals. Nine
adversarial cases (0, -1, `i32::MIN`, `i32::MAX`, a legal no-op grow, the
ordinary grow, `create_pool` with 0 and -1, and a repeat from a second
connection) all leave it alive. Two regression tests drive a real
`wayland-client` connection through a real `State`'s real dispatch and
assert an innocent second client is still served; the zero case fails
against the unfixed tree with the upstream panic. 95 tests, clippy/fmt
clean, `scripts/smoke-test.sh` green (its `foot` windows exercise the
ordinary shm buffer path through the new dispatch), macOS cross-build
clean.

Delete `dispatch.rs`'s guard and go back to
`smithay::delegate_dispatch2!(State)` once a Smithay bump carries the
missing `return`.
