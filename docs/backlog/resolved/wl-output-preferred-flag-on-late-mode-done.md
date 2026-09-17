---
title: "An already-bound `wl_output` client is never told a `--nested` resize's new mode is preferred — RESOLVED."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# An already-bound `wl_output` client is never told a `--nested` resize's new mode is preferred — RESOLVED.

## The entry as filed

Found 2026-09-16, reviewing PR #49 (`output_management.rs`'s module doc,
which used to claim this ages the same way `wlr-output-management`'s
`preferred` flag does — it doesn't; the two protocols diverge).

`headless.rs`'s `set_mode` (the sole path that ever adds a second mode,
via `State::resize_output`, itself gated to run at most once per process —
see `output_management.rs`'s module doc) calls
`output.change_current_state(Some(mode), ...)` and only afterwards
`output.set_preferred(mode)`:

```rust
output.change_current_state(Some(mode), ...);
output.set_preferred(mode);
```

Smithay's `wl_output` implementation sends mode events off the state at
the time `change_current_state` runs, to every client already bound —
before `set_preferred` has been called. So a client that bound `wl_output`
*before* the resize is told about the new mode with no `preferred` bit
set at all, and nothing resends it afterwards: not "goes stale," never
sent true in the first place.

A client that binds *after* the resize is unaffected — the whole current
state, `set_preferred` included, is sent at bind time.

Not a regression from PR #49: this is a pre-existing `headless.rs`/
`wl_output` ordering issue, only noticed because writing
`wlr-output-management`'s own (correct) handling of the same event forced
tracing the comparison. Low priority: reachable only through `--nested`'s
one-shot startup resize, and no probed client keys behavior off `wl_output`
mode's `preferred` bit today.

Fix, whenever it's worth the churn: call `output.set_preferred(mode)`
before `output.change_current_state(...)` in `set_mode`, or check whether
Smithay's `Output` batches both into one flush regardless of call order at
a newer pinned rev.

## Resolution (2026-09-17) — fixed as filed, first alternative checked and ruled out

**No batching at the pinned rev (`0ff00983`).** Verified in source, not
assumed: `Output::change_current_state` (`src/output.rs`) updates the
inner state and then synchronously calls `wl_change_current_state`
(`src/wayland/output/mod.rs`), which computes
`flags = Current | (preferred_mode == new_mode ? Preferred)` and
immediately sends `mode(flags, …)` + `done` to every already-bound
instance. `set_preferred` only mutates inner state and sends nothing. So
call order is exactly what the wire sees, and the old order guaranteed
the bit missing. The fix is the one-line swap in `headless.rs`'s
`set_mode`, with a comment naming the mechanism.

**`--tty` shares the fix.** It has no mode-setting sequence of its own:
`tty/hotplug.rs` reaches `State::resize_output`, which calls the same
`set_mode`. One fix covers both backends. (The "once per process" gating
the ticket names lives in `--nested`'s `Host::is_configured`, not in
`resize_output` itself — a second `resize_output` with a new size works
as before, which the resize-back test below keeps proving.)

**`wlr-output-management` agrees post-fix.** Its snapshot is taken after
`set_mode` returns, so it was correct even before; the new fail-first
test asserts both protocols' resize batches in one place, pinning them to
each other.

## Verified

Three new tests in `output_management/tests.rs`, all real-client and
wire-level (`WlMode` records every `wl_output.mode` event with its flags;
the suite's `TestClient` previously dropped the `preferred` bit entirely):

- `wl_output_tells_an_already_bound_client_the_resized_mode_is_preferred`
  — **confirmed to fail unfixed**: pre-fix run showed the bind-time mode
  arriving `{current: true, preferred: true}` and the post-resize mode
  arriving `{current: true, preferred: false}`; post-fix both carry the
  bit, and the `wlr` batch carries `ModePreferred` alongside.
- `wl_output_reports_preferred_to_a_client_bound_after_resize` —
  regression pin, passes pre- and post-fix by design (stated, and
  confirmed passing pre-fix).
- `resizing_to_the_same_mode_keeps_it_current_and_preferred` — same-mode
  edge pin, passes either way (the preferred mode already named it, so
  even the old order sent both bits); the swap changes nothing there.

`set_preferred` before any mode exists is the `init` path, exercised by
every existing bind test. Full set green post-fix: `cargo test -p flexwm`
(778 passed), `cargo nextest run --workspace` (877 passed),
`cargo clippy -p flexwm --all-targets -- -D warnings` clean,
`cargo fmt --check -p flexwm` clean, `scripts/smoke-test.sh` 15/15 ok.

Live `--nested` under cage (post-fix binary, `--width 640 --height 480`,
host configured 1280x720 so `resize_output` really ran): a `wayland-info`
steady-state read shows both modes with `1280x720` carrying
`current preferred`. The transient resize event on an already-bound
client was **not** captured live — attempted three ways (post-startup
`WAYLAND_DEBUG` client, tight poll, child spawned pre-dispatch): every
external client binds after the queued host configure is already
processed, so each saw the combined bind burst instead. That ordering is
wins-by-construction only inside the harness, which dispatches manually —
the fail-first test above *is* the wire evidence for the transient, and
this is stated plainly rather than papered over.

No hot-path benchmark: this runs once per resize (startup / hotplug),
never per frame or per event. No README change: no user-facing behavior
any user or integrating agent relies on today — no probed client reads
the bit; the only lasting text is the corrected `output_management.rs`
module doc, which had documented the divergence and now documents the
agreement (plus its pinning test).
