---
title: "Probe: does scoot send wl_keyboard.leave when a client destroys its only surface"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-11-05"
---

# Probe: does scoot send wl_keyboard.leave when a client destroys its only surface

Filed 2026-10-18. Serves **computer use (stuck focus state delivers phantom input to the next focus)**.

## The gap

While testing #670 the implementer saw no `wl_keyboard.leave` after a client destroyed its only surface under headless scoot. R-670 traced the path statically (`toplevel_destroyed` -> `remove_window` -> `set_focus(None)` -> Smithay `leave`) and found no reason it would be skipped, but nobody has a repro. Worst case: a client-side `focused` flag and a live key repeat on a surface-less client.

## What to do

Write a minimal headless probe: map a surface, take keyboard focus, destroy the surface (not the connection), and assert `leave` arrives. If it does not, find where focus is dropped silently and fix it in scoot; if it does, close this with the test kept.

## Not in this ticket

The key-repeat logic itself, which disarms on `leave` already.

## Resolution

Confirmed, test kept. No bug: `wl_keyboard.leave` arrives in every case probed, so no fix to scoot was needed. The earlier "no leave" observation does not reproduce.

`crates/scoot/src/compositor/keyboard_focus/tests.rs` drives a real `wayland-client` connection and asserts the exact `enter`/`leave` sequence plus the compositor's own focus:

- only window, `xdg_toplevel` destroyed alone: `[Leave]`, focus cleared
- only window, toplevel and `xdg_surface` destroyed, `wl_surface` kept: `[Leave]`, focus cleared
- only window, whole chain destroyed in one burst: `[Leave]`, focus cleared
- second window, same client, toplevel destroyed: `[Leave(1), Enter(0)]`, in that order
- second window, same client, whole chain in one burst: `[Leave(1), Enter(0)]`
- second window in another client: the old client gets `Leave`, the other `Enter`

Evidence, tree uncommitted on top of 85136f21e (dev VM, `/var/tmp/kbleave`):

```
cargo nextest run -j2 -p scoot keyboard_focus
     Summary [   1.609s] 6 tests run: 6 passed, 3128 skipped
cargo test -j2 -p scoot keyboard_focus
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 3086 filtered out
cargo clippy -j2 -p scoot --all-targets -- -D warnings   (clean)
cargo fmt --check -p scoot                               (FMT_OK)
```
