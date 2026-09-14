---
item: "12"
title: "Four missing bounds"
status: "done"
area: "core"
pr: null
commit: null
---

# Four missing bounds

**(a) `State::listen` no longer panics when the wayland socket cannot be
created.** `State::new` and `listen` return
`Result<_, Box<dyn Error>>` (the two `insert_source().expect()`s inside
`listen` became `?` with them, via `.map_err(|e| e.error)` — `InsertError`'s
own payload is the source that could not be inserted, which is no use to an
operator and would force the error type to carry a
`ListeningSocketSource`), and a new pure `socket_error` maps every
`BindError` variant to an operator-facing message. Five call sites traced
and updated: `compositor::run` (propagates, so `main` prints
`flexwm: <message>` and exits `FAILURE`) plus the four live-`State` test
harnesses. `seat.add_keyboard`'s own `expect` in `State::new` was left
alone deliberately — a different failure (no xkb keymap for the default
layout), not this ticket.

Before/after on the dev VM, **release** builds both ways (what makes the
difference visible: `panic = "abort"`), `env -u XDG_RUNTIME_DIR flexwm
--headless` — before: `panicked at state.rs:196: a free wayland socket:
RuntimeDirNotSet`, `Aborted (core dumped)`, exit 134, and a real
174.2K coredump recorded by `systemd-coredump`; after: `flexwm: no wayland
socket: $XDG_RUNTIME_DIR is not set or invalid; set it to a writable
directory (a login session normally provides one)`, exit 1, no coredump. A
second scenario (`XDG_RUNTIME_DIR=/proc`, i.e. set but unwritable) prints
the `PermissionDenied` message — traced to wayland-server's
`bind_absolute`, where a lockfile it cannot create is mapped to that
variant, and where `bind_auto` returns it immediately instead of trying the
other 31 names.

**(b) A client's `min_size` is clamped where `shell.rs` reads it**, to the
usable area (`area.inset(gap)`) of the largest output the core knows —
`learned_min`'s existing bound, widened from "the window's own output" to
"any output" because nothing in `World`'s public surface says which output
an unplaced toplevel will land on, and narrowing it further could wrongly
shrink a hint a window could legitimately fill its own screen with. With
the one output this compositor creates the two bounds coincide.

**The backlog entry's diagnosis was one step off, found by disabling the
clamp and watching the tests fail**: `ring_rects`'s `rect.w + 2 * width` is
not the first unchecked add an `i32::MAX` minimum reaches.
`flexwm_core`'s own `World::place_workspace` (`arrange.rs`, `x + width` in
the on-screen test) overflows first, during `arrange`, before anything
renders — a debug-build panic, a silently wrong `visible` flag in release.
Both are closed by clamping at the read site.

Two further facts the fix turned on: `xdg_toplevel.set_min_size` is
unvalidated at the pinned rev (`handlers/surface/toplevel.rs` stores
`(width, height)` verbatim, negatives included), and it is
double-buffered — so it only becomes the `current()` value `info_of` reads
on commit, and `info_of` itself only runs from `add_window` (on
`get_toplevel`, before any commit) and `refresh_window`
(`app_id_changed`/`title_changed`). The live test drives exactly that
sequence, which is also what a real toolkit produces.

**(c) `flexwm_core::Config`'s `gap` is bounded above as well as below.**
`Config::MAX_GAP` = 10,000 px, past the long edge of an 8K display (7680),
so no real output has usable area left at it; `Config::clamp_gap` is public
because two places must agree on it — `validated()`, and `flexwm`'s config
loader, which sizes the focus ring against half the gap *before* a `World`
exists to validate one. This is not just a visual-proportionality fix:
sizing the ring against the raw, unclamped gap means `focus_ring_width =
i32::MAX` at `gap = i32::MAX` clamps to `gap.max(0) / 2 = 1073741823`
(`Appearance::clamped`, unaware the gap itself is out of range), and
`decorations::ring_rects`'s `rect.w + 2 * width` then overflows for any
`rect.w >= 2` on every single render — a debug panic or a release
wraparound, from a config file alone, found by `flexwm-reviewer` while
checking this fix and covered by a dedicated test
(`an_out_of_range_ring_width_is_also_clamped_against_the_capped_gap`).
The doc comment deliberately does **not** claim the cap bounds
`gap * (windows - 1)` in `column_heights`: that product is bounded by
window count, not by this.

Live on the dev VM, release binaries both ways, `gap = 2147483647` with
one `foot` window on a 640x480 output, read back over IPC — before:
`rect {x: 2147483647, y: 2147483647, width: 1073742145, height: 482}`
(wrapped garbage; `2 * gap` in `Rect::inset` wraps, and that `x + width`
then overflows too); after: `{x: 10000, y: 10000, width: 1, height: 1}` —
degenerate, as a 10,000px gap on a 640px output should be, but coherent.
`gap = 12` on the same binary still gives `{12, 12, 302, 456}`, so ordinary
configs are untouched.

**(d) `wl_shm` pools are capped at `MAX_SHM_POOL_BYTES` = 512 MiB**, at
*both* requests that reach the same `mmap`: `wl_shm.create_pool`'s initial
size and `wl_shm_pool.resize`'s target. Extends `dispatch.rs`'s existing
hand-written blanket `Dispatch` impl (item 7's seam — the pinned rev has no
`delegate_shm!`). 512 MiB is four full-screen 8K ARGB frames (126.6 MiB
each) or sixteen 4K ones in a single pool; it is a per-pool bound, not a
total, and the entry says so rather than overclaiming. `flexwm-reviewer`
confirmed the total is still genuinely unbounded, live: 40 pools at
exactly the cap (well within it individually) reserve ~20 GiB from one
connection, more than the pre-fix 8-pool/16.1 GiB finding this item was
written to close, just needing more requests to get there. Bounding the
sum needs per-client accounting this module doesn't have — **not**, as an
earlier version of this note said, "the separate connection-cap entry's
territory" (that entry is about the IPC control socket's own connection
count, unrelated to wayland client accounting). Tracked as its own
Backlog entry instead.

**Refused, not clamped**, deliberately: a clamp would leave client and
compositor disagreeing about the pool's size, so the client would go on
placing buffers at offsets it believes are inside its own mapping and
collect confusing `invalid offset` errors from `create_buffer` later,
somewhere other than the request that was wrong. Each refusal uses the code
upstream itself uses for a bad size on *that* request — `InvalidStride` on
`wl_shm` for `create_pool`, `InvalidFd` on `wl_shm_pool` for `resize` — so
a client sees one consistent code whichever side decided.

Refusing `create_pool` means returning without initializing the
`New<WlShmPool>` it carries, and an uninitialized object's
`UninitObjectData::request` is a `panic!`. That is safe for a specific
reason, checked in wayland-backend 0.3.17 rather than assumed and recorded
in the module doc so it needn't be re-derived: `post_error` calls `kill`
synchronously, and `Client::next_request` returns `EPIPE` once `killed` is
set, so no later request from that client is dispatched — *including one
already buffered in the same `write()`*. `UninitObjectData::destroyed` is an
empty no-op (so the `cleanup`/`queue_all_destructors` pass over that object
does nothing) and `wayland-server`'s `New` has no `Drop` impl. A test
pipelines `create_pool(i32::MAX)` + `resize(4096)` in one write for exactly
this case.

Bug-bashed on the dev VM against release binaries both ways, ten scenarios
each (one under the cap, exactly at it, one over, and `i32::MAX`, for both
requests; the pipelined pair; eight oversized pools in one batch; three
oversized attempts from three fresh connections; an ordinary 4096-byte pool
afterwards). After: in-range sizes accepted (`VmPeak` 677,168 kB confirms
the 512 MiB mapping is really made), everything over refused with the
message naming both numbers, the compositor alive and answering IPC
throughout, the innocent client still served. Before: *every* oversized
request accepted, and the eight-pool batch took the compositor's `VmPeak`
to **16,864,580 kB (~16.1 GiB)** of reserved address space from one client
in one batch — which is the finding, stated in numbers.

**Tests: 24 new against the merge base** -- 22 in `-p flexwm` (200 total,
from 178) and 2 in `-p flexwm-core` (45 total, from 43). The clamps
are pure functions tested one under each bound, at it, and one over; the
pool cap and the `min_size` read site are driven through a real
`wayland-client` connection and a real `State`, the pattern
`dispatch/tests.rs` established (extended here so the offending client is a
closure, which is what let `create_pool` and `resize` share one harness).
`shell/tests.rs` and `state/tests.rs` are new modules, split out rather
than appended to their parents. Three **negative controls**, each run and
recorded: reverting the gap clamp fails both new core tests (one with
`attempt to multiply with overflow` inside `Rect::inset`); making
`clamp_hint` the identity fails 6 shell tests (two with `attempt to add
with overflow` inside `arrange.rs` — the finding in (b)); and disabling the
`create_pool` guard fails the two tests that cover it, one with "the pool
size was accepted" and one showing the pipelined `resize` then really does
reach the pool object.

**Benchmarked, because the pool cap adds a second `TypeId` check to the
per-request path** — but the real proof `flexwm-reviewer` found is in the
object code, not the jiffies: built the release profile with
`strip=false` and counted `DisplayHandle::post_error`'s monomorphizations.
It exists for exactly **3** concrete types: `WlShmPool` (this guard's
resize check), `WlShm` (this guard's `create_pool` check), and
`XdgWmBase` (Smithay's own, unrelated). If the `TypeId` comparison this
guard adds had *not* folded away after monomorphization, that call would
have been instantiated for every interface flexwm dispatches at all
(`WlSurface`, `WlPointer`, `WlKeyboard`, `XdgSurface`, `XdgToplevel`,
`WlBuffer`, ...) — it isn't. The guard body is compiled out of every
interface's dispatch path except the two it actually checks, so a
regression on an unrelated request type (like the `wl_surface.damage`
flood this item benchmarks) was never possible in the first place. This
is the finding; the jiffies below are corroboration, not the proof.

Jiffies, for the record (same method as item 7: 1M `wl_surface.damage`
requests, compositor `utime+stime`, one discarded warm-up rep, one
discarded run of 16 balanced interleaved reps each of `a7ffef3` before
and this commit after): before mean 36.19 (sd 4.21), after mean 35.69 (sd
2.44) — a difference of 0.41 standard errors, indistinguishable from
noise, nominally in the *faster* direction. An earlier, smaller two-binary
run (12 reps) had shown before 35.33 vs after 38.50 and looked like a real
~9% regression; it was not reproducible at 16 reps and could not have been
the guard regardless, once the object-code argument above is the actual
basis for the conclusion rather than this measurement. Recorded as a
caution: on this VM, a benchmark below roughly 16 balanced reps is not
powerful enough to trust for a single-digit-percent effect, and jiffies
alone should not be the only evidence for a hot-path change when a
compile-time argument is available instead.

Also verified at the same commit: `cargo test -p flexwm` 200/200 and
`-p flexwm-core` 45/45 on the dev VM, clippy `-D warnings` clean for both
crates there and `--workspace --all-targets` clean on macOS, `cargo fmt
--all --check` clean on both hosts, `cargo check --workspace` clean on
macOS (the cross-platform build), and `scripts/smoke-test.sh` green under
`--headless` (11 `ok:` checks, exit 0) — which exercises the ordinary
`wl_shm` buffer path through the new dispatch with a real `foot` window.

**Found while bug-bashing, deliberately not fixed here** (see the new
Backlog entry): `--width`/`--height` are raw unbounded `i32`s, so an
operator's own `--width 2000000000` can still overflow the same
`x + width` in `arrange.rs` that (b) closes for client-declared minimums.
Out of this ticket's scope, and a CLI flag rather than a client- or
config-supplied value, but it is the same family.
