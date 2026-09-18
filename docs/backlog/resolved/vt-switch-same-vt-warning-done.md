---
title: "Suppress the same-VT no-op case of the IPC VT-switch warning (5c) — RESOLVED."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Suppress the same-VT no-op case of the IPC VT-switch warning (5c) — RESOLVED.

## The entry as filed

`docs/backlog/tty/vt-switch-same-vt-warning.md` (LOW):

> Suppress the same-VT no-op case of the IPC VT-switch warning (5c).
> `change_vt`'s `VtSwitchOutcome::Requested` also fires — with a hedged
> warning, per 5c — when the requested VT is the one the session is already
> showing on, since libseat itself returns `Ok(())` for that request too
> (confirmed on real hardware; see 5c's verification). A precise fix would
> need `Tty` to track which VT it currently occupies and compare before
> calling `session.change_vt`, so this case can be `Ignored` instead of a
> hedged `Requested`. Not done in 5c because libseat exposes no query for
> "what VT is this session on" at `init` time — the number would have to be
> sourced some other way (the kernel's own active-VT ioctl on the console
> fd, perhaps) and 5c's warning-not-guarantee wording already makes this a
> false-positive risk, not a silent-failure one, so it wasn't worth blocking
> 5c on. Low priority: cosmetic (an agent gets told to be careful once for
> no reason), not correctness-affecting.

## Resolution (2026-09-18)

`State::change_vt` (`compositor/tty/mod.rs`) answers a same-VT request
with a new quiet outcome instead of calling libseat at all, so `ipc.rs`
replies plain `Ok` where it used to send the hedged `Warning`.

The ticket's anticipated design (a `Tty` field tracking the session's own
VT, sourced at `init`) proved unnecessary once the question was restated:
the init-time question "which VT is this session on?" has no libseat
answer, but the per-call question "which VT is the kernel displaying
*right now*?" does — `/sys/class/tty/tty0/active`, read live on each
`change_vt` call. No new `Tty` field, so no new write-site semantics to
audit the 05b way; a session property that never changes needs no field.
The read costs one small sysfs open per call on a path that runs only for
an explicit VT-switch keybind or IPC `key` — never a hot path — so there
is deliberately no benchmark (same standing as the one-branch paused
render gate: the cost is unmeasurable at this call rate).

Decision predicate, unit-testable (a free function, the
`tty_blocks_render` shape — no harness can construct a `Tty`):

```rust
fn same_vt_noop(session_paused: bool, requested: u32, displayed: Option<u32>) -> bool {
    !session_paused && displayed == Some(requested)
}
```

- `!session_paused` is load-bearing, not redundant with `change_vt`'s own
  paused gate: while paused the displayed VT belongs to another session,
  so an equality there could only be a stale read, and skipping on that
  basis would be the one-way-door silence `IgnoredPaused` exists to
  prevent. A paused session never no-ops here, whatever the kernel
  reports — pinned by its own test.
- `displayed: None` (no sysfs here, unreadable node, unparsable content
  such as a serial console's `ttyS0`) falls through to the old behaviour:
  ask libseat, report the hedged `Requested`. Uncertainty resolves toward
  the cosmetic false positive, never toward skipping a possibly-real
  switch.

New `VtSwitchOutcome::IgnoredSameVt` variant rather than reusing
`Ignored`: a verified no-op on a live session and "no backend to ask" are
different reasons, and `ipc.rs`'s match stays exhaustive (a future sixth
variant still fails to compile there instead of silently becoming `Ok`).
`Requested`'s doc is updated to match: reaching libseat now already means
a different VT was asked for. The `Warning` text itself is unchanged — its
"no-op if already on the target VT" hedge still describes the residual
`displayed: None` fallback honestly.

### Tests

Seven unit tests in `tty/mod.rs` (`parse_active_vt` truth table incl. the
`ttyS0`/`tty0`/overflow/garbage refusals, plus the four-leg `same_vt_noop`
table). Fail-first, each leg's pin proven separately: with the predicate
neutered to `false`, `an_active_session_requesting_the_displayed_vt_is_a_noop`
fails (6 passed, 1 failed); with the nonzero filter removed,
`non_vt_consoles_are_unknown_not_zero` fails (6 passed, 1 failed); both
pass restored.

### Live `--tty` evidence (dev VM, release build, seatd-backed session on tty1)

Pre-fix baseline (unmodified `main`, `1b706b8`): `msg key ctrl+alt+F1`
while tty1 is displayed → `{"type": "warning", ...}` (exit 0), no pause —
the false positive, recorded before the fix.

Post-fix (branch `fix/vt-switch-same-vt-quiet`):

- `msg key ctrl+alt+F1` → `{"type": "ok", "locked": false}` (exit 0).
  At `RUST_LOG=debug` the skip logs exactly once:
  `change_vt requested for the VT already displayed; skipping the libseat call vt=1`.
  At the default `info` level nothing is logged, by design (contrast the
  paused skip's `info!`: nothing the caller asked for is lost here).
- `msg key ctrl+alt+F2` → the hedged `Warning` (exit 0), session pauses,
  foreground VT reads `tty2` — the real-switch path still warns.
- Switch-back retry over IPC while paused → `IgnoredPaused` warning, zero
  `could not change vt` (EPERM) lines — 5b unregressed.
- `sudo chvt 1` back → `session activated` + `drm: modeset (full commit)`;
  post-reactivate screenshot byte-identical to the pre-pause one
  (`09a91ae837c2118f5b0ed6dac8bab4b6`, 19665 bytes both) — pause/reactivate
  with modeset + repaint intact, no master-state desync from the skip.
- 3× same-VT repeats → three plain `ok`, zero pauses, foreground VT still
  `tty1`.

### Bug bash

- Log sweep across both live sessions: only known-benign startup lines
  (dmabuf feedback, keymap init, cursor-theme fallback, DRM surface init,
  libinput init, output creation) plus the expected pause/activate/modeset
  lines.
- Seat discipline: seat verified free (`ps`, no flexwm/seatd-client) before
  each session; both sessions quit via IPC `quit`; foreground VT restored
  to tty1; seat free afterwards.
- Full standard set on the final tree: `cargo test -p flexwm` 930 passed +
  3 doctests, `cargo nextest run --workspace` 1035 passed / 1 skipped,
  `cargo clippy -p flexwm --all-targets -- -D warnings` clean,
  `cargo fmt --check -p flexwm` clean, `scripts/smoke-test.sh` exit 0.

### Not verified live (stated plainly)

- The `displayed: None` fallback (sysfs missing/unreadable) has no live
  leg: `/sys/class/tty/tty0/active` is world-readable on every kernel this
  project runs on, so the fallback was reached only by construction
  (the `None` unit test), not by deleting sysfs under a running session.
- No live client held a popup grab or IME composition across any switch;
  the skip touches neither path (it returns before libseat is called, and
  grab/IME state lives in the seat, untouched — same construction argument
  as the paused-render gate).

No README change: the IPC `Warning` text is not documented in the README,
and a spurious warning disappearing in one no-op case is not a new
user-facing surface (stated explicitly per the implementer instructions
rather than silently skipped).
