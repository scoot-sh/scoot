---
title: "presentation-time (wp_presentation) advertisement: frame-timing feedback for video/animation clients — DONE"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# `wp_presentation` (presentation-time) — DONE

~~Precise frame-timing feedback, mainly useful for smooth video/animation
clients.~~ — DONE, the last bundle child of
`docs/backlog/protocols/protocol-gaps-general.md` (which now marks every
child done). Clients get `presented` events with a `CLOCK_MONOTONIC`
timestamp, the output's refresh, a frame sequence and flags -- or
`discarded` for content superseded before it ever reached the screen.

## What landed

`wp_presentation` (version 2), advertised to every client with no filter.

Two findings on the way in, both verified in source rather than assumed:

- **Smithay carries the whole protocol at the pinned rev** (`0ff0098`:
  `src/wayland/presentation/mod.rs`), so this is the PR #88/#89 shape: one
  hold-alive field on `State` (`presentation_state`, with `CLOCK_MONOTONIC`'s
  id as the `clk_id` the bind handshake reports), no hand-rolled handler, no
  Smithay patch. The blanket `Dispatch` in `dispatch.rs` forwards everything
  to Smithay's own `Dispatch2` impls, so that file is untouched.
- **The frame-completion half is flexwm's to write, per backend.**
  Smithay's docs leave it explicit: drain committed feedback before frame
  callbacks, mark presented once the frame goes out. `State::render` in
  `headless.rs` is the one funnel all three backends present through, so the
  hook lives there (`State::present_feedback` in the new
  `presentation_time.rs`): taken if and only if the frame actually went out
  -- a `--tty` flip issued, a `--nested` commit handed to the host (a new
  `bool` on `Host::present`), or a presenter-less headless frame drawn --
  and never for a rendered-but-dropped frame. While locked only the lock
  surfaces are stamped (`SessionLock::take_presentation_feedback`, mirroring
  `send_frames` over the same surface set including popups); windows keep
  theirs queued until unlock.

## Timestamp semantics, per backend (stated, not implied)

Every timestamp is `CLOCK_MONOTONIC`, read once per presented frame. What it
*is* differs honestly by backend (see `presentation_time.rs`'s module doc
and `README.md`'s new section, which say the same thing in both places):

- **Headless with no presenter: render-complete time.** There is no scanout;
  the framebuffer is the final image, so finishing the render is the moment
  the new image became current.
- **Nested: host-commit time.** When the host scans the committed frame out
  is the host's business and unknowable from inside; this reports the
  commit, not a guess at the host's vblank.
- **TTY: flip-issue time.** Read when the page flip is handed to DRM, so it
  leads the photons by up to one vblank -- the `VBlank` event carries no
  timestamp to report instead, and deferring feedback to vblank receipt
  would open a dispatch window in which the client could destroy its
  feedback object before the `presented` addressing it goes out (take and
  mark are atomic inside one render today, so that window does not exist).

`seq` is zero on headless and nested (the protocol requires zero with no
retrace to count) and the issued-flip number on `--tty` -- corrected in
review from the frame serial first shipped here; see "Review round" below.
`refresh` is the output mode's own (60 Hz fixed on every backend today). Flags are `vsync` on `--tty` only --
the flip is vblank-synchronized; the other backends have no retrace and no
zero-copy path behind a pixman copy, so they report none.

## Edge cases (all pinned by tests)

- **Superseded commit gets `discarded`, shown commit gets `presented`.**
  Smithay's own `merge_into` discards at commit time; the test commits twice
  with no frame between (a single step, so the frame timer the first commit
  arms cannot fire between them -- see below).
- **Undisplayed surface gets nothing.** A `wl_surface` with no shell role
  is never in the space and never drawn; its feedback stays queued --
  neither presented nor discarded.
- **Disconnect with pending feedback is clean.** Smithay's teardown owns
  the queued callback; later renders run normally.
- **Locked window waits for unlock.** Requested after the lock is
  established, the feedback survives locked renders untouched and is
  presented by the first unlocked frame.
- **Client destroys nothing mid-flight.** Take and mark run atomically
  inside one render with no dispatch between, so no `presented` can address
  a dead object; pending feedback on a destroyed surface or for a
  disconnected client is Smithay's own `Drop`/`client()` path.

## Evidence

- Fail-first, dev VM (`ssh -p 2222 dev@localhost`, branch
  `feat/presentation-time`, module docs + tests with no `State` wiring):
  all 7 fail -- `no wp_presentation -- the global is missing` at startup.
- Post-fix, same VM: `cargo test -p flexwm` 842 pass 1 ignored (7 new),
  `cargo nextest run --workspace` 947 pass 1 skipped,
  `cargo clippy -p flexwm --all-targets -- -D warnings` clean,
  `cargo fmt --check -p flexwm` clean (Mac-side),
  `scripts/smoke-test.sh` (`SMOKE_PREFIX=/tmp/smoke-pres3`) exit 0, 17
  `ok`, zero `BUG`/`FAIL` lines.
- Live, same VM, clean `cargo clean -p flexwm && cargo build` first (the
  9p gotcha): real `foot` against a headless flexwm sees
  `wl_registry#2.global(31, "wp_presentation", 2)` in its registry
  (`WAYLAND_DEBUG` sighting) and binds nothing -- foot uses neither half
  of this protocol, which is expected. Bind/feedback/event flow is
  harness-proven, not live-proven: no video/animation client exists on the
  VM. `--nested` host-commit and `--tty` flip-issue timestamping are
  likewise construction-verified (the hook sits on each present path, and
  the headless tests pin the shared take/mark machinery), not
  live-measured -- stated as an environment limit.
- Cost: one monotonic read per presented frame, plus the take walk over
  mapped surfaces and the per-feedback socket writes only for surfaces that
  asked -- and, since review, measured: 16 windows, 60 frames x 5 reps,
  690-760 us with the walk vs 664-816 us without (fully overlapping, kept
  as-is; see "Review round" item 2). Nothing at all runs on frames that
  present nothing.

## Bug bash (found while testing, fixed in place)

Three failures on the way to green, each a real harness/compositor finding
rather than a typo:

1. **The frame timer can fire between two test steps.** Two `RequestFeedback`
   steps with a settle between them let the first commit's 16ms timer arm
   and fire, presenting the first feedback before the superseding commit --
   so the discard test now commits twice inside one step
   (`RequestFeedbackPair`), where no timer can interleave.
2. **The locker's unlock never flushed.** `unlock_and_destroy` was acked
   over the step channel without a round trip, so the unlock sat in the
   client's unsent buffer while the compositor rendered a still-locked
   session. The locker now round-trips before acking (the window client
   already did -- every other step ends in a round trip).
3. **Reporting invalidated live feedback indices.** Draining the client's
   feedback vector on report left an already-reported (still live)
   server-side object addressing a cleared index -- the unlock presents the
   *same* object the locked frame reported as pending. Reports now advance
   a cursor over a never-cleared vector, plus a `ReportAll` snapshot step
   for re-reading an old object's later outcome.

## Review round (PR #90): one blocking finding, three smaller items

1. **BLOCKING: `seq` violated the protocol's explicit contract.** The
   `presented` event's own doc requires zero with no retrace, and the
   shipped frame serial was nonzero on headless/nested; tty already had a
   per-scanout counter in hand (`FlipTracker::issued`) and now reports it.
   Fixed as prescribed: `presented_frame` (new pure function in
   `presentation_time.rs`) maps tty flip / 0 / 0, and `render()` stamps
   its answer. The module doc's "orders distinct images" defense was the
   misreading -- corrected. Fail-first: the rewritten headless test failed
   pre-fix (`seq is 1 ... MUST be zero`); the six new tests (five
   `presented_frame` arms incl. nested committed/dropped, one wire
   passthrough at seq 42) fail pre-fix too (stashed-impl run:
   `unresolved import super::presented_frame`, 3-args-vs-4). `FlipTracker`
   gained an exact-numbering unit test (0,1,2 across settle cycles) pinning
   the values tty reports, with the issued-not-MSC limit stated in the doc.
   Nested-zero and the dropped-frame arms are unit-level (`presented_frame`
   takes no backend): a live host connection is unconstructible in-harness
   (registry-bound surface/shm/buffers against a real host compositor), so
   the harness covers headless end to end and the units cover every arm.
2. **Take-walk benchmark.** Temporary bench (16 mapped windows, 60
   full-redraw frames x 5 reps, removed after): 690-760 us/frame with the
   walk vs 664-816 us neutered -- fully overlapping, unmeasurable against
   the pixman redraw. No early-out added: Smithay exposes no cheaper global
   query (`PresentationFeedbackCachedState` is per-surface only, pinned-rev
   `src/wayland/presentation/mod.rs`), and flexwm-side bookkeeping would
   spend to save nothing measurable. Numbers recorded in the module doc.
3. **Nested `present()` ignored a dead host's flush refusal**
   (`let _ = flush(); true`). The flush result is now the return, so a
   frame that never left stamps nothing. Single caller verified by grep
   (`headless.rs` only). No harness test: same unconstructible-Host reason
   as above, stated not papered over -- the dropped-frame unit arm
   (`presented_frame(true, false, ...) == None`) pins the compositional
   half (any `false` from `present()` stamps nothing).
4. **README refresh caveat.** The presentation section now cross-references
   Display information's known 60-Hz-on-faster-panels inaccuracy and tells
   pacing clients to trust timestamps, not `refresh` arithmetic.

## What this deliberately leaves open

- Nothing on presentation-time itself. Its bundle entry
  (`docs/backlog/protocols/protocol-gaps-general.md`) is now fully done --
  every child resolved.
- Unbounded `feedback` requests are accepted, like upstream: each is a small
  queued callback with no per-request event and no scan it feeds, so a
  client spamming them spends only its own socket budget. Recorded, not
  built.
- True scanout timestamping on `--tty` (reporting at vblank receipt rather
  than flip issue) is not built: the `VBlank` event carries no timestamp,
  and carrying taken feedback across frames would open the destroyed-object
  window the atomic take-and-mark deliberately avoids. The ~1-vblank
  earliness is documented where the timestamp is defined, not hidden.
