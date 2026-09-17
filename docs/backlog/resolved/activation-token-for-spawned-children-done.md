---
title: "`xdg-activation-v1`: flexwm's own `spawn` hands its child no activation token — DONE."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# `xdg-activation-v1`: flexwm's own `spawn` hands its child no activation token — DONE

Follow-up from implementing the protocol
(`docs/backlog/resolved/foot-protocol-warnings-done.md`). Whoever starts a
process conventionally puts an activation token in the child's
`$XDG_ACTIVATION_TOKEN`, so the child can activate its own window when it
finally maps one. `State::spawn` (every keybinding and IPC `spawn`) set
`WAYLAND_DISPLAY` and `FLEXWM_SOCKET` but no token, so an app flexwm itself
started could not ask to be focused the way one started by a launcher client
can. Invisible in practice, because flexwm focuses a newly mapped window
itself (`add_window` passes `focus: true`) — until a slow cold start lets
focus move elsewhere first.

## What landed

`State::mint_spawn_token` (`compositor/activation.rs`), called from
`State::spawn` (`compositor/state.rs`) for every child:

- The child's `Command` gets `XDG_ACTIVATION_TOKEN` with a token minted via
  `XdgActivationState::create_external_token`, stamped with the spawn time
  and naming the spawned program as `app_id`.
- Any inherited value is removed first (`env_remove`, not overwrite): the
  compositor itself may have been started with one (a launcher client, a
  nested session), and that token is a receipt for someone else's user
  action.
- A spawn that never starts (missing binary, overlong name) pulls its token
  back out of the table rather than occupying a slot until the sweep finds
  it. The empty command still returns before minting at all.
- `spawn` is `&mut self` now; both callers (`act`, `run`) already held it
  mutably. Concurrent spawns cannot interleave: the compositor is
  single-threaded, and rapid sequential spawns are what the cap test pins.

## The bounds decision (the ticket's design question)

Both of `activation.rs`'s bounds apply to a compositor-minted token,
uniformly with client-minted ones:

- **Freshness (`TOKEN_LIFETIME`, 30s), stamped at spawn time.** The `Default`
  token data's `Instant::now()` is the spawn as the "event" time, and
  `request_activation` refuses it past 30s exactly like a launcher's. No
  longer per-token lifetime: the motivating slow cold start is seconds,
  which is the case the 30s figure was sized for, and a second lifetime
  would need a table this change refuses to build. The sweep in
  `token_created` prunes expired spawn tokens too, so a leaked child that
  never redeems stops costing a slot after 30s.
- **The shared 64-token cap, with refuse-not-evict.** The spawn path sweeps
  expired tokens first, the way `token_created` does, and a genuinely full
  table mints nothing: the spawn proceeds without a token -- today's
  behavior, mapping still focuses the window -- never by evicting someone
  else's live entry. The eviction direction is the whole point, traced both
  ways:
  - Spawns never break interactive tokens. Many rapid spawns (an agent loop
    starting apps) degrading to tokenless is the safe direction; evicting
    the oldest to make room would let that loop steal a launcher's live
    token.
  - The reverse -- 64 *live* spawn tokens refusing one interactive mint --
    is bounded and self-healing (every spawn token is redeemed once or
    expires within 30s), where a separate uncapped table would be an
    unbounded leak. The alternative considered and rejected.

The serial gate is untouched: `create_external_token` never reaches
`token_created`, and that is safe because the gate binds whoever *asked* --
the compositor is not a client that can be tricked into asking; minting is
its own focus decision, delegated to the child. Redemption still runs the
shared `request_activation` path (freshness, non-window refusal, lock gate,
`clicked_layer` spend, single-use removal), unchanged by this change — the
tests below verify a spawn token through env-presence, focus-move,
single-use, cap, sweep, failure, and lock-refusal; expiry-refusal,
non-window-refusal, and the `clicked_layer` spend ride on the shared
function and are pinned by its own suites rather than re-exercised
per-origin.

## Tests

Seven new tests in `activation/tests/spawn.rs`, each on a real spawned child
(a `sh` probe that writes whatever `$XDG_ACTIVATION_TOKEN` it saw to a
file -- the closest honest fixture to a toolkit reading the variable), with
the token string handed back over the file then redeemed through the real
`request_activation`:

- `a_spawned_child_sees_its_activation_token_in_its_environment` — the var
  is present and the string is known to the table. (Redeeming an unknown
  string directly would still move focus -- `request_activation` trusts the
  data it is handed -- so the tests look the string up first; asserting
  focus alone would pass against an implementation that minted nothing.)
- `a_spawned_childs_token_moves_focus_after_focus_moved_elsewhere` — the
  slow-cold-start shape: focus first, spawn, focus second, redeem against
  the first, assert. The child maps nothing, so the move comes from the
  token.
- `a_spawned_childs_token_cannot_be_spent_twice` — redeem, move away, and
  the same string is unknown to the table, which is what makes Smithay's
  dispatch (called only for a known token, verified in the pinned rev's
  `dispatch.rs`) never reach the handler. Calling the handler directly with
  the stale clone would bypass that lookup -- a shape no protocol client
  can produce, since clients send only the string.
- `spawn_tokens_share_the_cap_and_a_full_table_mints_nothing` — 64 live
  tokens, then a real spawn gets nothing and every live entry survives.
  Passes without the implementation by design: it pins what spawn must
  *not* do.
- `expired_tokens_are_swept_before_a_spawn_mints` — 64 expired tokens, then
  a real spawn gets a token and the table holds exactly 1.
- `a_spawn_that_never_starts_leaves_no_token_behind` — missing binary,
  4096-character name, empty command. Guards the removal arm (see below).
- `a_spawned_childs_token_is_refused_while_locked` — a real locker client
  (no lock surface, like the keyboard suite's), spawn, redeem refused with
  focus untouched, unlock, fresh spawn redeems fine. Needs a headless
  backend: the lock is only confirmed once a blanked frame is drawn, which
  the bare harness never produces (found failing, fixed by moving the
  test).

Each behavior-changing test was confirmed to fail against the unfixed code
(the child writes `SPAWN_MINTED_NO_TOKEN`) before being kept; the two
boundary tests pass either way and say so. The removal arm was
toggle-verified separately (commented out, the failure test goes red,
restored). The lock test's first shape (bare harness) timed out waiting for
`Locked` and is recorded above as the environment it needed, not the
behavior.

## Evidence

(Standard verification set; exact commands, raw outputs, and commit SHA in
the PR report.)

- `cargo test -p flexwm`: 712 passed, 0 failed
- `cargo nextest run --workspace`: 811 passed, 1 skipped
- `cargo clippy -p flexwm --all-targets -- -D warnings`: 0 warnings
- `cargo fmt --check -p flexwm`: clean
- `scripts/smoke-test.sh` (`MODE=--headless`): green, including the new
  live line asserting the spawned `foot`'s `/proc/<pid>/environ` carries a
  non-empty `XDG_ACTIVATION_TOKEN`. Proven to go red with minting disabled
  (0 matches) and green with it enabled (a 32-character token) against the
  same binary shape.

## What this deliberately does not touch

- The serial gate, the cap size, the rename: all out of scope per the
  ticket.
- No `PROTOCOL_VERSION` bump: no protocol changed. README's
  `xdg-activation-v1` section gains the integrator-facing paragraph (an env
  var a launched app consumes), nothing else user-facing moves.
- Not benchmarked, and why: nothing touched is a per-frame or per-event
  path. A spawn already forks and execs (milliseconds); the added work is
  one table sweep, one insert and one small `String` alloc on that path.
  The render loop, input dispatch and IPC dispatch are unchanged.
