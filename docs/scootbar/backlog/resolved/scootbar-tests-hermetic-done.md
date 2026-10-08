---
title: "scootbar integration tests are hermetic (no real config, no sound server needed)"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-06"
---

# scootbar integration tests are hermetic (no real config, no sound server needed)

Filed 2026-10-05. Serves **daily-drive**: a contributor running the suite on
their own desktop (the M2 is the maintainer's daily machine) should get the
same answer CI does, and a red local run should mean a real bug.

## The gap

Found reviewing PR #463. `cargo nextest run --workspace` on the Asahi M2
reported 22 failures, all `scootbar::{bar,clock,visibility,workspaces}`, and
every one reproduced identically on `origin/main`. CI was green. There were
two causes, neither of them in the code under test:

- **The tests read the maintainer's real config.** `scootbar/tests/common`
  does not sandbox `XDG_CONFIG_HOME`, so the daemon loads
  `~/.config/scoot/bar.toml`. That file puts `clock` in `center`, and the
  clock tests' own placement then fails `clock is placed twice; a module goes
  in one place`.
- **They need a running sound server.** The volume module fails `no sound
  server at …/pulse/native` in a shell with no PulseAudio/PipeWire-pulse
  socket.

A reviewer spent a round proving these were environmental. The next one may
not, and may block a good PR or wave through a bad one.

## What to do

- In `scootbar/tests/common`, give every spawned daemon a fresh temp
  `XDG_CONFIG_HOME` (and `XDG_STATE_HOME`/`XDG_CACHE_HOME` where read), so no
  file outside the test's control can change a result. Pin it with a test that
  writes a conflicting `bar.toml` into the real-looking path and passes anyway.
- Make the sound-server dependency explicit. Either the volume module's tests
  run against a stub server (preferred, if cheap), or they detect a missing
  server and skip with a message, the way
  `config::tests::an_unwritable_parent_is_a_loud_refusal` probes its premise.
  Tests that are not about volume must not fail for its absence: leave volume
  out of their layouts.
- Document any remaining prerequisite in `dev/research/scootbar-testing.md`.

## Not in this ticket

The scoot compositor suites (they already run headless with their own
runtime dir) and CI changes (CI is already green, which is the point).

## Resolution (2026-10-06)

Both causes were reproduced on the M2 first, with a built `scoot` and the
maintainer's real `~/.config/scoot/bar.toml` present: `cargo nextest run
-p scootbar --test bar --test clock --test visibility --test workspaces`
gave 13 passed, 22 failed. With `XDG_CONFIG_HOME` pointed at an empty temp
dir the same 35 all passed, so the config was the whole of it for these
suites — none of them needs audio. Two failure modes were seen live: ``
`--right`: `clock` is placed twice; a module goes in one place `` (the
flag against the file's section) and `scootbar: volume: no sound server
at ...` (the file's `right` list starts volume, whose warning breaks the
empty-stderr assertions).

What landed (`test(scootbar)`, in one commit):

- `tests/common` sandboxes every child it starts: each `Session`'s
  scratch carries its own `XDG_CONFIG_HOME`/`XDG_STATE_HOME`/`XDG_CACHE_HOME`,
  applied to the compositor, the bar, and `msg` (and to bare `daemon`
  runs through `Scratch::command`), always after the test's own
  environment so the harness wins. `XDG_STATE_HOME`/`XDG_CACHE_HOME` are
  not read today; they are sandboxed anyway.
- `tests/hermetic.rs` pins it: a conflicting `bar.toml` (`left =
  ["clock"]`, `right = ["volume"]`) under a fake `HOME`, with the
  sandbox's `XDG_CONFIG_HOME` empty. Proven to fail before (the same
  shape through the pre-fix API fails with `` `clock` is placed twice ``)
  and pass after; it also asserts the layout holds only the flag's clock
  and the daemon's stderr stays empty.
- Volume stays as the ticket allowed: every integration layout that places
  it already runs against the stub server in `tests/pulse/` (via
  `PULSE_SERVER`), and the one unit test that wants a real server skips
  with a louder reason naming the stub.
- The missing-binary trap: the harness skip now names the `REQUIRE` knob
  that turns it into a failure (`SCOOTBAR_REQUIRE_SCOOT=1` fails loudly
  when the binary is absent, proven), and CI's integration job — which
  sets `REQUIRE_SCOOT`/`REQUIRE_SWAY` — now also runs `--test visibility
  --test workspaces`, the two failing suites it never ran.
- `dev/research/scootbar-testing.md` documents the sandbox, the stub, and the
  skip-vs-fail choice.

Proof on the M2 (real `bar.toml` present, built `scoot`, no sound server,
no sway): full `cargo nextest run -p scootbar` 1360 passed, 4 skipped,
3 failed — the 3 failures are pre-existing unit-test environment issues
(no D-Bus on the box: `media` bus-drop, `power` popup-connect, the module
contract), failing identically on the base commit; `cargo test -p
scootbar` agrees (1217 passed, same 3). CI stays green (PR #470).
