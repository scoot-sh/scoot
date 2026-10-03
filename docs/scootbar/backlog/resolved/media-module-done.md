---
title: "Media module: now playing, play/pause/next over MPRIS"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M6"
resolved: "2026-10-03"
---

# Media module

Filed 2026-09-29. Serves **daily-drive**.

MPRIS players (`org.mpris.MediaPlayer2.*`) on the session bus, through the
[shared D-Bus client](dbus-client-done.md): watch `NameOwnerChanged` to learn players
appear and vanish, `PropertiesChanged` for track and playback state. All
signals, no polling; playback position is deliberately not shown (it has no
change signal and would need a timer).

## What to build

Artist and title (truncated by pixel width), a state class, click for
play/pause, scroll or right-click for next/previous, a `player` config key to
prefer one when several run. Nothing shown when no player is present.

## Edge cases

A player that dies without releasing its name, several players (follow the
most recently playing), metadata with arbitrary text (bound and sanitize, as
for [window titles](window-title-module-done.md)), a player that reports position
constantly (ignored).

## Done when

Track changes show within a frame, controls work against two real players,
and the module costs nothing with no player.

## What landed

PR #393 (`feat(scootbar): media module, now playing and controls over
MPRIS`), code commit `242b2dce7` (`crates/` tree
`864d03f35d18ea3a87f2b421744e929d49a280c0`); every number below was taken
at it, on the dev VM, and the raw runs, scripts and screenshots are in
[`bench/m6-media-vm`](../../bench/m6-media-vm/README.md). The reference is
[cli.md](../../cli.md#media); the cost table is in the
[resource ratchet](../lightest.md#m6-media-module-level-cost-measured-2026-10-03).

- **The module** (`crates/scootbar/src/modules/media/`, Cargo feature
  `media`, in `default`): `artist - title` with a play or pause icon (the
  muted class while paused), cut to `max-width` (default 320 logical pixels)
  with an ellipsis measured in pixels; `play-pause`, `next` and `previous`
  as module actions, bound with no config to a click, a right click or
  scroll down (next), a middle click or scroll up (previous); `[media]`
  takes `player`, `max-width`, `margin` and the five interaction keys.
  Nothing is shown with no player, and a stopped player shows nothing.
- **Which player** (`player.rs`, `select`): of those playing or paused, the
  configured one, else the one that most recently started playing, else the
  one that played last; a tie by name, so the choice is the same every run.
  The controls go to the one shown.
- **Signals only** (`session.rs`): `NameOwnerChanged` of the
  `org.mpris.MediaPlayer2` namespace and `PropertiesChanged` of the Player
  interface on the one MPRIS object, both filtered by the bus, so unrelated
  apps and a player's `Seeked` never wake the bar; a track change is one
  signal carrying the value, and a property a player only invalidated is
  read once, at most every 50 ms; `GetAll` is asked of the owner's unique
  name, one in flight per player.
- **Client changes**, small and tested: `dbus/mpris.rs` (the shape readers,
  `std` only, in the `dbus` fuzz target and checked against two messages
  sd-bus marshalled) and `dbus/link.rs` (a connection held across the bus
  coming and going, with the tray's rules and tests against a real daemon
  and a bus that drops the bar). `conn::runtime_dir` is public and
  `render::art_extent` crate-visible; the window title's pixel ellipsis moved
  to `modules/ellipsis.rs`, behavior unchanged, for both modules. The tray
  was not moved onto `link` ([tray-onto-dbus-link](../tray-onto-dbus-link.md)).
- **Hostile or buggy players** cannot crash or hang the bar or another
  player, grow it without bound, or make it believe another player's state,
  and hold one of the 8 slots at most (the exact guarantee, and what they
  can do, is cli.md's Bounds): only the bus's own `NameOwnerChanged` is
  believed and a `PropertiesChanged` only from the connection that owns a
  held name; one player a connection and at most 8 held; a newcomer to a
  full room takes the place of the oldest read, stopped player, else waits
  in a list of 16 and is held when a slot frees or a held player stops (so
  eight connections do not hide a ninth until it restarts; a second name of a
  connection waits the same way); owners asked about a window of 8 names at
  a time (a connection owning hundreds of names hides no real player listed
  behind it); strings cleaned and cut to 120 bytes where stored, through a
  reused buffer; an answer that does not parse is dropped whole, one that
  errors (a timeout, or `UnknownObject` from a player that has the name
  before it exports the object) leaves the player held and unshown, read
  again at its next signal; one that never comes is forgotten by age; a reply
  meant for the old owner of a name that changed hands finds nothing; a skip
  within 250 ms of the last is refused with its reason (a scroll of thirty
  steps, or a fling, is one skip); a run of changes (title, state, the player
  shown) is drawn ten times a second at most, the first at once. What was *not* done about a
  player: nothing throttles the work of parsing a flood of Position signals
  (0.65% of a core at 500 a second, measured), only the pump bounds.
- **Edge cases, each a test** (`modules/media/tests.rs` against a scripted
  bus, `daemon_tests.rs` against a real `dbus-daemon`, `dbus/mpris/tests.rs`
  for the wire): zero players; several (which is shown); one appearing or
  vanishing mid-update; a name changing hands; a player that never answers
  (forgotten after the TTL); one that errors; PlaybackStatus `Stopped`;
  missing and empty metadata; a 2 MiB reply (skipped, the player keeps its
  state, the connection lives); very long titles, controls and ESC in
  strings; a string that is not UTF-8 (the answer is dropped, the player
  keeps its state); Position and Volume changes (no redraw, no read); the
  bus dying with a change held; unrelated bus traffic (zero wakeups).

## Evidence

All at code commit `242b2dce7` (`crates/` tree `864d03f35d18`), dev VM (aarch64, 6 CPUs, rustc 1.97.1); the
logs are `bench/m6-media-vm/logs/` and the command lines are in its README.

- `cargo fmt --check -p scootbar`: clean (run on the Mac, the Linux-only
  crate needing only rustfmt).
- `cargo clippy -p scootbar --all-targets -- -D warnings`: `OK` for the
  default, none, each of the 15 features alone, `media` with each other
  feature, `popup` with each other feature, the default with `icon-image`,
  and two mixed sets (48 runs; `logs/ci.txt`).
- `cargo nextest run -p scootbar --bin scootbar` with every feature (1025
  tests), none (414), and each feature alone (432 to 527; `media` 527), all
  passing, `SCOOTBAR_REQUIRE_DBUS_DAEMON=1`.
- `cargo nextest run -p scootbar --no-fail-fast` and `cargo test -p scootbar
  --no-fail-fast` with `SCOOTBAR_REQUIRE_DBUS_DAEMON=1 SCOOTBAR_REQUIRE_SCOOT=1
  SCOOTBAR_REQUIRE_SWAY=1 SCOOTBAR_TEST_SCOOT=/var/cargo-target/debug/scoot`:
  1134 tests, 1133 passed; the one failure, `popup::a_bar_with_no_popup_binding_binds_nothing_for_popups_until_invoke_asks`,
  fails the same way on `main` (`01c33f09f`) with the same `scoot`, which is
  the VM's build of 1 October (`logs/popup-on-main.txt`); CI builds `scoot`
  from the tree. `agent::layout_rectangles_are_where_a_click_lands_on_two_outputs_at_two_scales`
  passed here. Both runners fail only that test.
- Flake loop: the media, link and mpris tests and the two help tests (91
  tests), 100 runs under `cargo nextest` and 100 under `cargo test`
  (one process, concurrent), the VM loaded by other work: 200 of 200 pass
  (`logs/flake.txt`). Two earlier loops found two races in the tests (the
  link test sent a signal before its match rule was in place, and a daemon
  test assumed two connections' signals have an order); both are fixed and
  the loops above are on the fixed tree (the review round's new tests are in it).
- Fuzz: `cargo fuzz run -s none dbus` at CI's budget (1,000,000 runs,
  `-seed=1`, 45 s) and a 300 s run (7,829,996 runs), no finding
  (`logs/fuzz.txt`); the dbus corpus replays in `cargo test`. The
  `fuzz` crate compiles with the new include (`cargo check --locked`).
- The zero-wakeup claim is not vacuous: with `arg0namespace` removed from the
  owner-changes rule, `unrelated_bus_traffic_wakes_nothing` and the rule test
  fail (`logs/mutation.txt`).
- Idle, 60 s windows, one run per row (`logs/measure-final.log`): **zero
  wakeups** with no bus, a bus and no player, one paused player, one playing,
  eight playing, and a real mpv playing a file; 2 with the clock (its own);
  one thread; the binary +65,536 B on disk (+3.4%) and +51,704 B of loaded
  sections (+2.9%) over `main`, `ldd` unchanged (the numbers are in the
  ratchet entry). **The maintainer waived this size row on 2026-10-03** (in
  chat; this row only) and `media` stays in `default`.
- End to end on headless `scoot` (`logs/live-private.txt`,
  `logs/live-broker.txt`, `screenshots/`), once on a private `dbus-daemon` and
  once on the VM's own `dbus-broker` session bus, with jeepney players and
  two real mpv instances: found by listing, a second player appearing, the
  controls reaching the player shown (read back with `playerctl`), the skip
  refusal, the most recently playing player shown, a long title cut with an
  ellipsis, `kill -9` of the shown player, the empty module and its
  refusals.

**Not verified**: a real browser or Spotify as the player; the Asahi M2 and
the full `scripts/scootbar-bench`; `nix build` of the package and the Nix
module check; a competitor's media module (Waybar's `mpris` is not in the
nixpkgs build on the box); the tooltip on screen with a player behind it (the module says it has
one, `Module::tooltips`, pinned by a test that fails without it, and the
bar's tooltip tests are module-agnostic; no run hovered the media module with
a player playing).

## Left

- [tray-onto-dbus-link](../tray-onto-dbus-link.md): the tray's copy of the
  bus lifecycle, and whether the modules should share one connection.
- Two modules, two connections: with both placed the bar holds two sockets to
  the session bus.
