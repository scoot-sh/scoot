---
title: "Binds do not repeat while held; no key repeat at all"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-05"
---

# Binds do not repeat while held; no key repeat at all

Filed 2026-10-05 from the `desktop-keys` verification (docs/backlog/resolved/desktop-keys-done.md:
"whether a held key repeats a bind (holding volume up must step, not fire
once)"). Serves **daily-drive**: holding volume-up stepping once per
press is not daily-drivable, and neither is a terminal with no key
repeat.

## Resolution (PR #444)

Landed together with `bind-allow-when-locked` in one PR: both ride the
same bind dispatch path, the same table flags and the same tests, so one
PR with two ticket closures. A bind opts in with the table form
(`docs/configuration.md#binds`):

```toml
[binds]
"XF86AudioRaiseVolume" = { action = "spawn wpctl set-volume @DEFAULT_AUDIO_SINK@ 5%+", repeat = true }
```

Opt-in (`repeat = false` default), not niri's repeat-by-default with
per-bind opt-out: fire-once is today's behavior for every existing bind,
and defaulting the other way would change what every held key does for
every existing user. Hyprland's `binde` is opt-in per bind the same way.
The re-fire uses the seat keyboard's own delay and rate (200 ms, then
25/s -- the pair the session already hands clients through
`wl_keyboard.repeat_info`, now shared as constants with `add_keyboard`
so the two cannot disagree). One in-flight repeat at most (the latest
press wins); the calloop one-shot exists only while such a key is held,
so there are no idle wakeups. It stops on release, VT switch, lock
change, session pause, output removal and a `[binds]` reload that swaps
the table. `quit` and `close` never repeat even when flagged (warned at
load, backstopped in the dispatch).

Correction to this ticket's diagnosis, verified against the pinned fork
(`035d447`, `src/input/keyboard/mod.rs` `key_input` +
`KeyboardHandle::input`) and scoot's `state.rs::State::new`: the Smithay
half is right (a same-source re-press is absorbed before the filter, so a
kernel repeat never reaches the bind filter), but "nothing sending repeat
to clients either" is wrong -- `seat.add_keyboard(..., 200, 25)` hands
every client `repeat_info` and toolkits repeat client-side off it, so a
held key in a terminal already repeats. The gap was binds only, and that
is all this fixes.

Evidence: `bind_repeat/tests.rs` (synthetic clock -- first re-fire at
the delay, stepping at the rate, release stops it, unflagged binds fire
once and arm nothing, `quit`/`close` never repeatable, latest-press-wins,
early firing re-arms, cancel drops; every behavior test fails with the
dispatch sabotaged back to fire-once); hot-path bench before/after on the
Asahi M2 (unbound key ~2.0 us/event both, bound chord ~2.9 us/event both,
overlapping -- no regression, and non-repeat presses allocate nothing
new); `cargo test -p scoot` 2086 passed 0 failed; `cargo nextest run
--workspace --no-fail-fast` 4280 passed with the only failures 22
scootbar tests that fail identically with the baseline binary (no sound
server in that box's test env); clippy clean, fmt clean,
`scripts/smoke-test.sh` exit 0, `nix build
.#checks.aarch64-linux.scoot-modules` exit 0. Live `--tty` proof on the
M2 (scoot-test session, uinput hold of `KEY_VOLUMEUP`, counter spawn):
1200 ms hold fired 25 times (1 + 25/s past the 200 ms delay), a 50 ms tap
fired exactly once, the count held stable over 3 s idle, and an 800 ms
hold on the swaylock screen fired exactly 16 times. Volume and
brightness left untouched (a counter script stood in for the tools).
Docs in the same PR (`docs/configuration.md`, `docs/nix.md`
troubleshooting).

## The gap

Holding a bound key fires its bind exactly once. Two layers agree:

- Smithay (pinned fork rev `035d447`, `src/input/keyboard/mod.rs`
  `key_input` + `input_from_source`): a press of a keycode already
  held by the same source is absorbed before the filter -- "don't
  double-run the filter (avoids re-triggering shortcuts)". A kernel
  repeat arriving as another press from the same device never reaches
  scoot's bind filter.
- scoot runs no repeat timer of its own: no `repeat_rate` /
  `repeat_delay` configuration, no timer re-firing binds, and nothing
  sending repeat to clients either (`grep repeat_rate
  crates/scoot/src` is empty outside pixman/upscale and prose).

So `XF86AudioRaiseVolume` held steps once, and (same root cause) a
held key in a terminal does not repeat. niri repeats binds by
default with per-bind `repeat=false`; Hyprland has a `repeating`
flag. scoot has neither.

## What to do

- A repeat timer on the compositor's key path: while a bound key is
  held past the delay, re-fire its bind at the rate (and, separately,
  forward repeat to the focused client per the keymap's repeat
  info). Per-bind opt-out (a volume step wants repeat; `close` or
  `quit` must never repeat).
- Pin: hold fires N>1 times, release stops it, unbound holds stay
  silent, opt-out binds fire once. Live proof on `--tty` (hold a Fn
  key, count steps) since that is the path that matters.
- The `desktop-keys` docs (`docs/nix.md` "Hardware keys") state the
  current once-per-press behavior until this lands; update them
  there.

## Not in this ticket

The `allow-when-locked` per-bind flag (sibling ticket
`bind-allow-when-locked`); per-device repeat rates; anything about
which keys the keymap binds (that is `desktop-keys`, landed).
