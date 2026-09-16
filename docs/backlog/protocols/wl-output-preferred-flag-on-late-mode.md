---
title: "An already-bound `wl_output` client is never told a `--nested` resize's new mode is preferred"
status: "open"
area: "protocols"
priority: "low"
blocked: null
---

# An already-bound `wl_output` client is never told a `--nested` resize's new mode is preferred

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
