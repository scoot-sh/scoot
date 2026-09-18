---
title: "Flake: activation taskbar-click precondition misses under extreme parallel load — DONE."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Flake: activation taskbar-click precondition misses under extreme parallel load — DONE

## The entry as filed

> Found 2026-09-18 while stressing the
> [screencopy parked-poll flake fix](../resolved/screencopy-parked-poll-flake-done.md):
> one full-binary run out of twelve failed in
> `activation::tests::keyboard::an_activation_takes_the_keyboard_back_from_a_clicked_taskbar`
> (`activation/tests/keyboard.rs:304`):
>
> ```
> the click never reached the taskbar -- check TASKBAR_POINT against the layout
> ```
>
> i.e. `drive_with_taskbar`'s `click()` at `TASKBAR_POINT` landed behind the
> taskbar (`clicked_layer` stayed `None`).
>
> The file already names this shape (line 283-286): the taskbar client's ack
> proves the *client* finished its round trip, not that the compositor has
> dispatched the buffer commit yet, so the click can run against a layer map
> that does not have the taskbar in it yet. The existing single `settle()` was
> not enough this once.
>
> Load profile matters for reproducing: the failure appeared exactly once,
> under deliberately abusive oversubscription — two full test binaries plus a
> 40-iteration targeted loop running concurrently on the dev VM (three
> processes, each multithreaded) — and the same test passes 5/5 in isolation
> on the same tree. It has never been observed under the standard suite
> (`cargo nextest run --workspace`, ~15 consecutive green full runs across the
> same session). A fix in the test's settle discipline (settle-until-mapped
> rather than settle-once) is the likely shape; production behavior is not
> suspected.

## Resolution

Test-only fix in `activation/tests/keyboard.rs` (no production code touched —
`git diff` is one test file, +59/−10). The delivered-behavior assertions stay
byte-exact in wording and order; only the precondition sequencing learns to
synchronize. No README change (no user-facing surface).

**Verify-first found three distinct load-only trip mechanisms at the same
test, not one** — each caught by stressing the previous cut under the same
abusive oversubscription, each fixed test-only:

- **Settle-insufficiency (the filed shape).** Under load the compositor has
  not dispatched the taskbar's buffer commit when the click runs, so
  `layer_hit` finds no surface and the click falls through to bare desktop
  (`clicked_layer` stays `None`). Fixed with `settle_until_taskbar_hit`: a
  bounded (100 rounds, panics loudly instead of hanging) settle-until loop on
  the exact hit test the click itself uses (`State::layer_under`, the same
  `layer_hit` `focus_under_pointer` clicks through) — proving the click's
  precondition rather than hoping a fixed dispatch count covers it. Applied
  in both `drive_with_taskbar` and `drive_locked_with_taskbar` (identical
  precondition shape; the lock drive has no racing client but shares the
  ack-proves-client-only gap).
- **Racing `activate` spends the click before the assert.** The window client
  fires its `activate` the moment the key press reaches it, and `click()`'s
  trailing `settle()` dispatches it: production `request_activation` spends
  the click (correct behavior — the test's own final asserts demand it)
  before `clicked_layer.is_some()` runs. Proven distinct, not assumed: the
  settle half had already returned a hit on the same thread with no dispatch
  since (`pointer_move`/`pointer_button`/`key` never dispatch the loop, only
  flush — re-verified in source), so the layer tree was provably identical
  and the click cannot have missed. Fixed by pressing and asserting
  synchronously — move, button-down, the four precondition asserts, focus
  capture, then release + settle — with no dispatch in between.
- **Racing `activate` moves focus before `focus_before` is read.** Same
  racing client: `focus_before` was read after the release's settle, by
  which time the activation may already have moved focus to window 1, so the
  "would prove nothing" guard trips on a lie. Fixed by reading it with the
  press asserts, before the release.

None of the three is a production race (no STOP per the ticket's own gate):
the first is the test looking before the compositor dispatched; the second
and third are the test asserting pre-activation state after dispatching the
activation. Production `request_activation` behaved exactly as pinned in all
three trips.

### Evidence (dev VM, `ssh -p 2222 dev@localhost`, `CARGO_TARGET_DIR=/var/cargo-target`, direct-binary stress to avoid the shared-target-dir double-spawn flake)

- **Reproduced pre-fix:** 2/80 targeted iterations trip under load (2 full
  test binaries + a 40-iteration targeted loop, 3 processes on 4 vCPUs —
  both at `keyboard.rs:304` with the filed message verbatim); 2/2 full
  binaries green at 930 passed. (The loop accidentally ran doubled in round
  one — 80 iters, not 40 — under even heavier transient oversubscription;
  recorded, not hidden.)
- **Production pin still discriminates:** with `request_activation`'s
  `clicked_layer = None` temporarily neutered, the fixed test fails at the
  delivered assert (`keyboard.rs:406`, "activating a window left the
  keyboard on the taskbar"), past every precondition; the lock-gate sibling
  stays green; reverted after (`git diff` confirms production clean).
- **Intermediate cuts trip distinctly:** settle-half-only tree trips 1/40 +
  1 full-binary at the same `clicked_layer` line *after* a proven hit
  (mechanism two); press-assert tree trips 2/41 at the `focus_before` guard
  (mechanism three). Each stress round is what found the next mechanism —
  the fix was not declared done until a round came back clean.
- **Post-fix stress, final tree:** 80/80 targeted iterations plus 4/4 full
  test-binary runs (930 passed each) under the same 3-process
  oversubscription that tripped 2/80 pre-fix — zero trips. Full standard
  set green on the final tree: `cargo test -p flexwm` 930+3,
  `cargo nextest run --workspace` 1035 passed / 1 skipped,
  `cargo clippy -p flexwm --all-targets -- -D warnings` clean,
  `cargo fmt --check -p flexwm` clean, `scripts/smoke-test.sh` 17 ok rc=0.

### Found alongside

Nothing. Every failure across all stress rounds (10 full-binary runs, 240
targeted iterations) is this ticket's test; no second distinct flake
appeared, so nothing new is filed.
