---
title: "Screen capture: unbounded sessions per client, and unbounded frames per session — RESOLVED (frames capped, sessions decided)."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Screen capture: unbounded sessions per client, and unbounded frames per session — RESOLVED (frames capped, sessions decided).

## The entry as filed

Two unbounded dimensions, found while building output capture (PR #52) and
reviewed 2026-09-16:

- **Sessions per client**: N sessions with N parked frames cost N full-screen
  copies on every tick the screen changes, and Smithay's `capture` dispatch
  plus `FrameData::destroyed` walk the *global* session list -- so client A's
  session count adds latency to client B's `capture` and teardown.
- **Frames per session**: `create_frame` pushes onto `active_frames` with no
  cap and Smithay never raises `duplicate_frame`; flexwm's
  `Capture::pending` throttle only runs on `capture`, so a `create_frame`
  loop with no `capture` is unbounded objects from one client and one
  session.

The entry prescribed a `dispatch.rs`-style intercept for the frames half and
left the sessions half to either per-client accounting shared with the
sibling entries, or a won't-fix with the per-surface precedent made airtight.

## Resolution (2026-09-17, PR #70)

Split, as filed: the frames half is capped, the sessions half is closed as
won't-fix. Both halves are recorded here so neither is re-derived.

### Frames per session: capped, per client rather than per session

`dispatch.rs` refuses a `create_frame` past **16 live frame objects for the
requesting client** with the protocol's own `duplicate_frame` error, before
Smithay's handler ever sees it. The count (`Screencopy::frames_per_client`)
is incremented pre-delegation and decremented in the same file's destruction
hook for every dead frame object -- which is what bounds the map itself: an
entry exists only while the client holds a live frame, including across
client disconnect (whose cleanup destroys every object) and for frames that
legally outlive their session.

Two adjustments to the entry as filed, both found once in the code rather
than assumed:

- **Per client, not per session.** The entry said the intercept "only needs
  to know whether *this session* already has an outstanding frame, which is
  local state this module already half-tracks." That is not implementable at
  the pinned rev: `Session`/`SessionRef` expose no protocol id or client
  accessor, `SessionData`'s fields are private with no accessor, and
  `ImageCaptureSource` (all `capture_constraints` sees) names the source,
  not the session. Nothing outside Smithay maps a session protocol object
  back to its bookkeeping. What the dispatch seam *does* see is the `Client`,
  so the bound is keyed by that -- and it still bounds the entry's exact
  attack, which is single-session. Re-verified field by field against
  `0ff0098/src/wayland/image_copy_capture/mod.rs` (`SessionRef` methods:
  `update_constraints`, `current_constraints`, `source`, `draw_cursor`,
  `user_data`; `Session`: `stop`, `as_ref`; no `impl` block at all on
  `SessionData`/`FrameData`).
- **Protocol error, not silent ignore.** The spec says the error "is raised",
  and a silent ignore would be worse than the leak: returning without
  initialising the request's `New` leaves `UninitObjectData` in place, whose
  `request` is a `panic!` (`rs/server_impl/mod.rs:126`), and the dispatch
  loop's own `(Some(child_id), None)` arm panics too unless the client is
  already `killed` (`common_poll.rs:288-296`). Either takes the whole
  compositor down the moment the client touches the frame it was never
  given. `post_error` kills synchronously, covering both -- the same
  argument `dispatch.rs` already makes for the `wl_shm` guards. A
  well-behaved client cannot trip it by racing its own lifecycle (one
  connection dispatches in order; a sent `destroy` always runs before a
  later `create_frame`), and the kill lands only on the client that
  overflowed -- never an innocent one, which is what rules out the
  global-cap shape.

16 is set far above anything legitimate on purpose, because tripping it
disconnects the client: the protocol allows one live frame per session,
`grim` holds one per run, a persistent preview holds one per session, and
the suites never exceed two. A tighter number would bound a few more small
objects per abuser; the failure mode of a false positive is a dead client.

### Sessions per client: closed as won't-fix

The per-surface precedent, made airtight: N sessions with N parked frames
cost N full-screen copies per changing tick -- the same shape of per-frame,
per-object work a client can already demand by mapping N surfaces, which
this compositor accepts unbounded. A cap on one and not the other is
inconsistent, not safer. And the only non-punitive cap is per client, which
has no hook: `new_session` carries no client identity (see above), and
`new_with_filter` counts manager *binds*, not sessions -- a client binds
once and creates without bound. Building per-client session accounting here
would be a fifth independent mechanism, contradicting the entry's own
direction that sessions fold into whatever shared mechanism closes the
sibling entries (`ext-workspace-object-binding-cap.md`, the IPC
`connection-cap-denies-the-same-user.md`, `shm-total-per-client-
unbounded.md`). When that mechanism lands, sessions join it; until then no
sessions-only cap.

The residual, stated honestly rather than minimised: client A's session
count still adds latency to client B's every `capture` and every frame
teardown through Smithay's global-list walks. Accepted because the unit cost
is a mutex lock plus a pointer comparison per session (tens of nanoseconds),
the same shape already accepted elsewhere (`wl_output` binds walk every
manager; layer/wlr paths walk every handle), and the frames cap above bounds
each session's share of the scan to at most 16 entries per abusive client.

## Tests

`screencopy/tests.rs`, four new real-client tests alongside the fifteen
that already passed unchanged (including the earlier second-outstanding-
*capture* refusal, which still refuses at `capture` time -- with the
per-client cap set at 16 the second frame's *creation* succeeds and its
capture fails `Unknown` exactly as before):

- a `create_frame` flood (21 frames, none captured or destroyed) is refused
  with `duplicate_frame`, the flooding client dies, the bookkeeping drains
  to zero and both session lists return to empty;
- a second, innocent client captures successfully after the first flooded
  itself into a protocol error -- the anti-`connection-cap-denies-the-same-
  user` property, pinned per client;
- 50 rounds of create/destroy cycling leave zero bookkeeping and the session
  still captures afterwards;
- three frames held while their session is destroyed still count (3) until
  the client disconnects, then drain to zero -- no dangling entries either
  way.

Fail-first: with the guard neutered Mac-side (chain entry forced false) the
flood test fails with "the compositor accepted 21 live frames with no
refusal"; restored, it passes. The cycling/dangle tests pass neutered (the
bookkeeping still counts; only the refusal is neutered), which is the
expected split.

## Verification

All captured on the dev VM (`ssh -p 2222 dev@localhost`,
`CARGO_TARGET_DIR=/var/cargo-target`, 9p mount at `/mnt/flexwm`), against
commit `4134456` (the code + tests + README; this record and the ROADMAP
entry followed as a second commit with no executable change):

```
cargo test -p flexwm            TEST EXIT=0  736 passed; 0 failed; 1 ignored
cargo nextest run --workspace   NEXTEST EXIT=0  835 run: 835 passed, 1 skipped
cargo clippy -p flexwm --all-targets -- -D warnings   CLIPPY EXIT=0 (0 warnings)
cargo fmt --check -p flexwm     FMT_CLEAN (one Mac-side join pass; the VM side
                                cannot write through 9p, same as .git/index.lock)
MODE=--headless scripts/smoke-test.sh   SMOKE EXIT=0 (15 `ok:` lines)
```

(`cargo fmt -p` ran Mac-side because the VM side gets `Permission denied`
writing through 9p; `fmt --check` re-verified clean on the VM afterwards,
and the screencopy + dispatch suites re-run green after the join.)

### Benchmark: the added per-`create_frame` cost

Temporary harness test (200k iterations, dev VM debug build,
`--test-threads=1`, removed before commit), three runs:

```
empty loop:              9-11ns/iter
Client::id:             12-15ns/iter
claim+forget pair: 1314-1834ns/iter
```

i.e. roughly 0.7-0.9us per `create_frame` and per frame destroy in a debug
build -- noise against requests that are socket-I/O-dominated and arrive at
most once per capture (even a 1000-frames/sec flood pays ~1ms/sec). Every
other interface's cost is a `TypeId` comparison both sides of which are
compile-time constants once monomorphized, i.e. folded away exactly like the
four guards before it. Merged on bound-not-performance grounds all the same.

### Live `grim` (the tolerance half of the refusal-form decision)

`grim` 1.5.0 against `flexwm --headless --width 800 --height 600` with one
real `foot` window mapped (`id 1, rect 12,12`), post-change binary:

```
grim run 1: 0.064s   grim run 2: 0.060s   grim run 3: 0.060s
PNG image data, 800 x 600, 8-bit/color RGB, non-interlaced (all three)
sha256 identical across all three captures
```

A well-behaved single-frame client is unaffected, as designed. Artifacts at
`dev@flexwm-vm:/tmp/grim-cap{1,2,3}.png`. Compositor, `foot` and `grim`
all killed and confirmed gone (`pgrep -x` empty) afterwards.

Not re-run, and named: the `--tty` cursor-capture deviation from PR #52
(the guard is backend-agnostic dispatch code; no `--tty` run was repeated
for this change).

### VM state

Both VMs were up before this work and neither was started, stopped or
restarted by it (`nc -z localhost 31022`, `nc -z localhost 2222` both
succeeded first). No `--tty` seat was claimed (every live run here is
`--headless`). One 9p gotcha hit, worked around per convention: VM-side
writes fail (`cargo fmt -p` → `Permission denied`, same family as the
`.git/index.lock` failure), so formatting ran Mac-side and was re-checked
on the VM.
