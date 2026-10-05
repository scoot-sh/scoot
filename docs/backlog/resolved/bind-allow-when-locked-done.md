---
title: "No per-bind allow-when-locked: volume and brightness die at the lock screen"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-05"
---

# No per-bind allow-when-locked: volume and brightness die at the lock screen

Filed 2026-10-05 from the `desktop-keys` verification
(docs/backlog/resolved/desktop-keys-done.md: "whether `[binds]` fire
while the session is locked (volume/brightness should,
launcher/clipboard must not)"). Serves **daily-drive**: volume and
brightness keys going dead the moment the session locks is the kind of
papercut that sends a user back to their old compositor.

## Resolution (PR TBD)

Landed together with `bind-repeat` in one PR (shared dispatch path,
table flags and tests). A `spawn` bind opts in with
`allow_when_locked = true` beside its action
(`docs/configuration.md#binds`, niri's shape); every other bind stays
refused at all three gates exactly as now. The desktop keymap
(`nix/modules/keys-home.nix`) opts exactly the volume, brightness, mute,
mic and media binds into both flags; launcher, clipboard, lock, capture
and notifications stay plain strings (fire once, never locked).

Decisions, all deliberate:

- Flagged non-`spawn` actions stay refused: `config.rs` warns and clears
  the flag at load, `act_bind` re-checks the action shape rather than
  trusting the flag, and the key filter checks both. Three layers so no
  single bug can let a layout/focus/close/quit bind through the lock.
- `State::act` (the backstop every other caller reaches) still refuses
  everything while locked. Only the keybinding path may let an opted-in
  `spawn` through -- and only its config-pinned command, so a locked
  session can never be made to launch anything but the opted-in
  commands. Spawning under lock is safe for focus too: the `apply()` it
  ends in re-derives keyboard and pointer focus onto the lock surface.
- IPC `Request::Action` stays refused while locked, *including* a spawn
  naming a command some bind allows: an IPC request carries an arbitrary
  command from whoever sent it, while a bind can only run its
  config-pinned command. Allowing IPC spawns would turn "volume keys
  work on the lock screen" into "anything with socket access runs
  anything while locked". Documented in `docs/protocols.md`'s
  session-lock section.

Evidence: `session_lock/tests/input.rs` (allowed spawn fires while
locked and is intercepted -- the lock client sees nothing; unflagged
spawn forwarded; flagged non-spawn refused; allowed repeat keeps
stepping while locked; lock cancels an in-flight repeat; IPC spawn
refused) -- each behavior test proven to fail with the dispatch
sabotaged back to total refusal, while the refusal tests stay green;
`cargo test -p scoot` 2086 passed 0 failed; `cargo nextest run
--workspace --no-fail-fast` 4280 passed with the only failures 22
scootbar tests that fail identically with the baseline binary (no sound
server in that box's test env); clippy clean, fmt clean,
`scripts/smoke-test.sh` exit 0, `nix build
.#checks.aarch64-linux.scoot-modules` exit 0. Live `--tty` proof on the
M2: with swaylock up (IPC `action` refused, proving the lock), an 800 ms
hold of volume-up fired exactly 16 times into a counter spawn, and the
screenshots show the grey lock screen and, after killing the locker, the
red abandoned-lock screen. The `docs/nix.md` "Hardware keys" limitation
notes are gone, replaced by the new behavior plus troubleshooting for
binds that lost their flags through a plain-string override.

## The gap

While the session is locked, no `[binds]` action fires except VT
switching (`crates/scoot/src/compositor/input.rs::key`, gated on
`session_lock.is_locked()`; backstopped in `State::act`; pinned by
`an_action_keybinding_does_not_fire_while_locked`). That gate is
correct for `spawn` in general -- a terminal from behind the lock
screen would be a complete bypass -- but it cannot tell a harmless
`spawn` (volume, brightness, media) from a bypass (terminal,
launcher, clipboard). So the `desktop-keys` keymap's hardware binds
go to the locker as ordinary keystrokes while locked. niri solves
exactly this with `allow-when-locked=true` on `spawn` binds (its
default config marks the volume binds that way); Hyprland has a
`locked` bind flag.

## What to do

- A per-bind `allow-when-locked` flag for `spawn` binds only (never
  for layout/focus/close/quit actions, which keep today's refusal):
  config syntax, the gate in `input.rs`, the `act` backstop's
  position on an allowed spawn, and tests that fail before (an
  allowed volume spawn fires while locked and reaches no window; a
  terminal spawn from behind the lock still does not).
- The `desktop-keys` keymap then marks exactly the volume,
  brightness and media binds allowed (launcher, clipboard, lock and
  capture stays refused), and the `docs/nix.md` "Hardware keys"
  limitation notes go away.
- Decide the IPC shape too: `Request::Action` is refused while
  locked today -- whether an allowed-spawn action through IPC stays
  refused (it bypasses focus; the keystroke path does not) needs a
  deliberate answer, not an accident.

## Not in this ticket

Bind repeat (sibling ticket `bind-repeat`); which keys the keymap
binds (that is `desktop-keys`, landed).
