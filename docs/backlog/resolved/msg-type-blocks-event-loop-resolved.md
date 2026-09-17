---
title: "A large `flexwm msg type` blocks the whole event loop for its whole duration (LOW, pre-existing). — RESOLVED"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# A large `flexwm msg type` blocks the whole event loop for its whole duration (LOW, pre-existing). — RESOLVED

## What it said

`ipc.rs` runs `Request::Type` to completion synchronously on the sole
event-loop thread, so a request near the 1 MiB inbound cap stalls wayland
dispatch, input and rendering until every character has been sent. PR #23's
benchmarks put a figure on it: ~2.3 s per MiB before, ~2.4 s all-lowercase,
~3.9 s all-shifted (extrapolated from 112 ms and 188 ms per 50,000
characters, release, `--headless`). Nothing deliberate gets near this — a
shell command line is a few hundred characters, i.e. under a millisecond —
so this is a hostile-input/accident bound. Fixing it properly means
chunking across event-loop iterations (per-connection progress state of the
kind item 10 deliberately avoided); the ticket's own cheap answer is a
separate, smaller cap on `Request::Type`'s text specifically, refused
rather than delayed, the same shape the screenshot rate limit took.

## Resolution

**Option (b), the separate smaller cap** (`ipc.rs`, `MAX_TYPE_CHARS`). Past
it a `type` request is refused with a `Response::error` naming the limit
and the workaround (split the text across several `type` requests), before
`type_text` runs — so a refused request types nothing, not even a prefix.
Checked in `handle_request`, the single funnel every `Type` goes through,
so no path bypasses it. Option (a) (chunking) is rejected per the ticket:
it adds exactly the per-connection progress state item 10 deliberately
avoided, for a worst case no deliberate traffic reaches.

**Sized by measurement, not feel.** Reproduced end to end first: release
build, `--headless` on the dev VM, a real `foot` mapped and focused,
`time flexwm msg type` of exactly 50,000 characters, 7 reps alternating
classes (balanced order), medians:

| text (50,000 chars) | median `msg type` wall time |
| ------------------- | --------------------------- |
| all `a`             | 388.8 ms                    |
| all `A`             | 796.7 ms                    |

That includes a live `foot` plus its shell competing for the VM's CPUs, so
it overstates the compositor's own stall; re-run clientless (window
closed, key events reaching no client — the per-character keymap and input
work is identical either way) at three sizes to fit the slope:

| n      | all-lowercase median | all-shifted median |
| ------ | -------------------- | ------------------ |
| 10,000 | 25.2 ms              | 47.0 ms            |
| 20,000 | 43.9 ms              | 84.6 ms            |
| 50,000 | 98.0 ms              | 214.9 ms           |

Roughly linear at ~2us/char plain and ~4.3us/char shifted — the same ~2x
class ratio the ticket reports (112/188 ms per 50k), within the spread a
different harness and CPU contention account for. The cap is
**16,384 characters**: worst case (all-shifted) ~75 ms, comfortably
sub-second with an order of magnitude to spare, while a few hundred
characters never notices it. Named once (`MAX_TYPE_CHARS`), next to the
refusal it gates.

**Characters, not bytes** — stated in the constant's doc and pinned by
`the_type_cap_counts_characters_not_bytes`: a character is the cost unit
(each becomes key events), whatever its UTF-8 length. No multi-byte
character the US-layout test seat can type exists to assert `Ok` with, so
the test pins it from the other side: 9,000 `é` (18,000 bytes, past the
number, but 9,000 characters) must reach `type_text` and be refused by the
*layout* (`no key for ...`), never by the cap. Even 16,384 four-byte
characters encode far under the 1 MiB line limit, so this cap always fires
first — the layering is deliberate, not incidental.

**Edges**: exactly-at-cap served in both classes; empty text accepted as a
no-op (current behavior, confirmed not changed); over-cap refused in both
classes plus multi-byte past the cap.

**Tests** (`ipc/connection/tests.rs`, all fail-first where behavior is
concerned — the refusal test fails on uncapped code, where over-cap
currently succeeds): over-cap refused naming limit and workaround (both
classes + multi-byte); at-cap served (both classes); chars-not-bytes;
a realistic ~200-character shell line pinned unaffected; empty accepted.
The pre-existing `a_large_but_legal_request_split_across_writes_still_works`
sent a 500,000-character `Type` and now sits above the cap, so it sends
10,000 characters instead — still over one 8 KiB read chunk (asserted), so
it still proves multi-read reassembly rather than the cap.

## Siblings checked, none capped

- `Request::Key` is one `KeyCombo`, not a sequence: `resolve_combo`
  dedupes modifiers (at most four distinct → at most four keymap lookups)
  and `press` sends at most nine key events. The modifiers `Vec` walk
  itself is a match plus a bit-test per element — a 1 MiB line of
  `shift+shift+...` costs sub-milliseconds, not the microseconds-per-unit
  `type` pays. Bounded by the line cap; no identical shape, no cap.
- `Action::Spawn { command }`: one process spawn per request, args bounded
  by the line cap, no per-element loop of comparable cost.
- Everything else on the socket (`Version`, `Outputs`, `Windows`,
  pointer/click/scroll, `Screenshot`, `WaitIdle`) is O(1) or already
  bounded (rate limit, in-flight caps, 60 s wait cap). `type` was the only
  request whose cost scales per client-controlled unit.
