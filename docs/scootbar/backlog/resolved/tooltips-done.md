---
title: "Tooltips: a module's `tooltip` shown after a hover delay"
status: "resolved"
area: "resolved"
priority: null
blocked: null
milestone: "M6"
resolved: "2026-10-03"
---

# Tooltips

## Resolution (2026-10-03)

A module's `tooltip` is shown in an `xdg_popup` under it after the pointer has
rested on the module for `bar.tooltip-delay` (500 ms by default, 0 turns them
off), and goes when the pointer leaves. Reference:
[cli.md](../../cli.md#tooltips).

- **The delay is the loop's `poll` timeout, not a timerfd** (a change from this
  entry's text, for the same rule). `Hover::wait` is `Some` only in the one
  state "resting on a module with a tooltip that has not shown"; every other
  state (nothing hovered, shown, dismissed) has no timeout, no fd and no clock
  read. A unit test pins it (`no_timer_and_no_clock_read_unless_the_pointer_rests_on_a_tooltip`),
  and an integration test shows zero wakeups over 2 s with the pointer resting on a
  module with none, on bare bar, with a tooltip shown, and after it went.
- **Shape.** `src/popup/hover.rs` is the pure state machine (no clock of its
  own: the daemon hands it one), `src/popup/wrap.rs` the word wrap
  (30 ems, at most six lines, an ellipsis where cut, a word wider than the line
  broken where it fills it), `Layout::compute_tooltip` its tighter layout, and
  `src/daemon/popup/tooltip.rs` the Wayland half. The popup's surface
  construction is one function (`build_popup`) both a click popup and a tooltip
  go through, and its drawing another (`draw_open`).
- **Its own slot, so a press still acts.** A tooltip lives in `Popups::tip`,
  beside `Popups::open`. `open` keeps its one meaning (a popup that may hold a
  grab): a press that finds one open still closes it and is spent (the second
  click that toggles), while a press that finds only a tooltip closes it and
  **acts**. Every site that reads `open` was traced: `is_open` and `is_for`
  (so `invoke popup` is not "toggle" because a tooltip is up) stay popup-only;
  `close`, `close_for` and the bar-surface destroy sites close both;
  `find_mut` (configure, buffer release, `popup_done`) looks in both;
  `open_popup` closes the tooltip first; the `xdg_wm_base` release in
  `sync_binds` waits for either.
- **Never the keyboard, a grab or a click.** No `get_keyboard`, no `grab`, and
  an empty input region (so the pointer cannot enter it: a tooltip flipped over
  the module under the pointer would otherwise close itself and reopen).
  Traced in `it_never_takes_the_keyboard_a_grab_or_the_pointer`.
- **Which modules**: every one whose view already writes a tooltip, nothing
  new read: window title, network, battery, volume and microphone, brightness,
  tray, `push` and `exec`. `Module::tooltips` (static, default false) says so,
  so a bar of a clock alone takes no pointer for tooltips; a contract
  test fails a module that writes one and does not say so. `xdg_wm_base` is
  bound when the first tooltip shows (as an `invoke` does), so a bar nobody
  hovers binds nothing (traced).
- **Dismissed by**: leave, any press (and the click goes on to act), any
  scroll, a click popup opening (and none shows while one is open), the text
  going empty, the module or output or bar going, hide, a
  `reload` (which also takes the new delay), the compositor's `popup_done`
  (the session lock); a scale change draws it again at the new scale. After a press, a scroll or `popup_done`
  it stays away
  until the pointer has left that module, so it does not come back half a
  second after every click.
- **While shown**: a text that fits the opened size redraws in place (damage
  is the tooltip's own surface); one that needs more room, or the module moving
  along the bar, makes it again at once. A popup is never resized after its
  configure, so this is the cheapest correct choice.
- **Config**: `bar.tooltip-delay` (milliseconds, 0 to 10000), config file only,
  absent (and refused) in a build without the `popup` feature, which is also
  the tooltip code.

### Decisions

| Question | Decision |
| --- | --- |
| Delay and key | `bar.tooltip-delay`, 500 ms, 0 is off, at most 10000; no flag |
| Timer | the poll timeout while resting on a module with a tooltip, nothing else |
| Motion within the module | does not restart the delay |
| Tooltip and a click popup | the popup wins; one popup at a time; a popup opening takes the tooltip down |
| After a press, scroll or `popup_done` | stays away until the pointer leaves the module |
| Keyboard | never; no grab either |
| Pointer and clicks | empty input region; a press closes it and acts |
| Position | the popup's positioner (slide along, flip across), under the module, above a bottom bar |
| Size and text | wrapped at 30 ems and never wider than the bar, at most six lines, `…` where cut |
| Default modules | every module whose view already had a tooltip; no data source added; the clock has none |
| exec/push | the `tooltip` key of the payload (already there) |

### Found on the way

- **A scoot bug, filed**: after a client ends a popup *grab* (the bar closing the
  volume popup on Escape), the first IPC `pointer_move` is delivered at the old
  position or not at all, so an IPC `click` right after can land on the wrong
  module. It predates this PR (no scoot code changed here), a popup that never
  grabbed (a tooltip) does not do it, and it is filed with the trace lines and a
  repro shape as
  [pointer-motion-after-popup-grab-end](../../../backlog/core/pointer-motion-after-popup-grab-end.md).
  The tests nudge the pointer twice after an Escape for it.
- The popup tests and benchmark now set `tooltip-delay = 0`: volume has a
  tooltip, and with the pointer resting on the module a tooltip stands where
  the tests look for a closed popup's absence.

### Evidence

Captured on the dev VM (6 CPUs, others building on it, load average 5 to 6
during the runs), against this PR's head commit (the tree was shipped to the VM
uncommitted, its `crates/` identical to the commit's; the PR body names the
SHA), the scoot being this tree's own debug build (no scoot
change in this PR). "Before" is the branch point, `c2d82cc95`, with the new
benchmark file copied in and `SCOOTBAR_BENCH_NO_TOOLTIP=1` (no key in the
config). Release builds (`lto = "fat"`, stripped), 60 s windows after the bar
settled, `crates/scootbar/tests/tooltip_bench.rs` and `popup_bench.rs`.

| Row | before | after |
| --- | --- | --- |
| clock only: RSS, wakeups per 60 s, fds, shm mappings | 4164 kB, 2, 8, 2 | 4164 kB, 2, 8, 2 |
| clock only, pointer resting on the bar: RSS, wakeups per 60 s | 4164 kB, 2 | 4164 kB, 2 |
| volume (popup_bench `idle`, no binding): RSS, wakeups per minute, fds, shm | 4160 kB, 0, 8, 1 | 4164 kB, 0, 8, 1 |
| `push` module with a tooltip, pointer away: RSS, wakeups per 60 s, fds, shm | 4164 kB, 0, 7, 2 | 4168 kB, 0, 7, 2 |
| same, tooltip shown, nothing changing: RSS, wakeups per 60 s, fds, shm | n/a | 4264 kB, 0, 7, 3 |
| stripped binary | 1,905,376 B | 1,905,376 B; `.text` +13,440 B (1,457,416 to 1,470,856, +0.9%), `.rodata` +320 B |

Show and hide, 100 cycles each checked to have mapped and drawn (a screenshot
shows the tooltip, then not), the bar's CPU from `/proc/PID/schedstat`:

| Row | Measured |
| --- | --- |
| CPU per show and hide | 639.5 us (64.0 ms over 100 cycles); popup open and close was 679 us |
| wakeups per show and hide | 5.00 (popup: 4.00) |
| RSS before, shown, after 100 cycles | 4168 kB, 4264 kB (+96), 4232 kB |
| peak RSS, fds, shm mappings after | 4232 kB, 7 and 7, 2 and 2 |

**The binary-size row grew**: `.text` +13,440 B (+0.9%) and `.rodata` +320 B.
The stripped file did not change size only because it is padded in 64 KiB steps
(the popups entry's +65,536 was one step), so `readelf -S` is the finer number.
**Waived by the maintainer on 2026-10-03** (this row only; see
[lightest.md](../lightest.md)); the independent review also judged it within
rule 1's noise margin. The RSS and wakeup rows are within one
buffer of themselves (a 1600 x 40 buffer is 250 kB, and the second one a redraw
racing a release makes or does not decides the mapping count, as in the popups
entry); the clock-only bar, which has no tooltip, is unchanged on every one of
them. The extra shm mapping while shown is the tooltip's buffers (two, made then
and dropped on hide). RSS after 100 cycles is 64 kB above before; it is
reported as measured, one batch, not as allocator retention.

Tests, all on the dev VM against the tree above (`SCOOTBAR_REQUIRE_SCOOT=1
SCOOTBAR_REQUIRE_SWAY=1 SCOOTBAR_REQUIRE_DBUS_DAEMON=1`, this tree's own debug
scoot via `SCOOTBAR_TEST_SCOOT`):

- `cargo nextest run -p scootbar`: 1081 passed, 4 skipped (the `#[ignore]`d
  benchmarks), run before the last integration test (the scale change) was
  added; after it, `--test tooltip --test popup` 6 times (31 tests each, all
  pass) and `cargo test -p scootbar --test tooltip` (15 pass). `cargo test -p
  scootbar` over everything, before that last test: every suite ok (938 unit
  tests). CI's `scootbar-integration` job lists its test files, so
  `--test tooltip` is added there (it had no way to run these otherwise).
- `cargo clippy -p scootbar --all-targets -- -D warnings` for the default build,
  `--no-default-features`, each of the 14 features alone and `popup` with each
  other (CI's matrix): clean. Unit tests (`--bin scootbar`) for `--all-features`,
  none, and each of the 13 modules alone: all pass (413 to 957 tests each).
  `cargo fmt --check -p scootbar`: clean.
- 19 new unit tests: the state machine (no clock read idle, show at the delay,
  leave, move between modules, motion, popup wins, press and scroll,
  `popup_done`, zero is off, an overflowing delay), the wrap (words, newline,
  control characters, long words, multi-byte, the six-line cap, hostile input,
  reused storage), the layout, and `a_warm_hover_and_a_shown_tooltip_allocate_nothing`
  (the state machine, the wrap, the layout and a paint over 300 rounds: zero
  allocations; it failed with two until its spare content was warmed to the
  longest text, so it can fail). Also three config tests, and a contract check
  that a module whose view has a tooltip says `tooltips()`.
- 15 integration tests in `tests/tooltip.rs`: hover shows after the delay
  (never before it) and leave hides it leaving the desktop pixels as they were
  (the destroy-without-grab redraw); 200 crossings that never rest show none,
  then 10 shows and hides leak no fd and no buffer; zero wakeups over 2 s on a
  module with no tooltip, on bare bar, with one shown and after; a click closes
  it and acts, it stays away under a still pointer, comes back after a leave,
  and a scroll dismisses it the same way; no grab, no keyboard, an input region,
  the pointer never enters it, and no `xdg_wm_base` bound until the first shows;
  its text follows in place (one popup made), shorter stays, longer is made
  again; wrapped (five lines of a 250-byte text; the six-line cap is covered by
  unit tests only, since a payload's 256 bytes cannot reach it), flush at the
  right edge; above a bottom bar; a
  reload closes it and takes the new delay (0 and back); `hide`; no tooltip
  under a click popup (volume, with a real PulseAudio-protocol server) and one
  after it closes; **a real `ext_session_lock_v1` client** (`tests/lockclient`)
  locks the session with a tooltip shown: the whole output is the lock's
  color, no tooltip is drawn over it for eight delays with the pointer on the
  module, and after the unlock one works; on sway, a virtual pointer shows
  one and unplugging the output with it shown leaves the bar alive and the
  other output's tooltip working.
- Flake loops: `--test tooltip --test popup` 8 times, before the scale test: 7 runs all 30 pass, one run
  failed `popup::on_sway_it_opens_and_goes_with_its_output` (it timed out at its
  20 s patience under the load of 30 tests in parallel on 6 CPUs); that test alone
  20 of 20, and `--test popup` alone 8 of 8 on the branch point and 8 of 8 on
  the head. Not attributed to either with more evidence than that.
- Screenshots: `/home/dev/tipshots/{tooltip-open,tooltip-wrapped-right-edge,tooltip-under-lock}.png`
  on the dev VM (not committed): the tooltip under the module with its text
  and frame; five wrapped lines, flush with the output's right edge; the solid
  lock color with the tooltip's place covered.

### Not done, deliberately

- **No tooltip for a tray item's own `ToolTip` text** (its icon and text are
  read for shape and dropped): the module's tooltip is the title list it had.
- **No move-restarts-the-delay and no instant re-show between neighbouring
  modules** (GTK shows the second at once after the first): each module's
  delay is the whole delay. A tuning, no harm.
- **A tooltip is made again, not resized, when its text needs more room**:
  `xdg_popup.reposition` would keep the surface, at a protocol version and a
  compositor round trip this does not need yet.
- **Not verified against a real tty session or on a compositor other than
  scoot and sway** (wlroots). KDE and GNOME popups without a grab are not
  available to test here.

Original entry, left as written:

Filed 2026-09-29. Serves **daily-drive**: the full window title, the battery
time, the SSID behind an icon.

A module's `View` already carries an optional `tooltip` string
([module API](module-api-and-clock-done.md)). Showing it is a popup without a grab,
anchored under the module, shown after a delay and dismissed on leave.

## Cost discipline

The delay is a timerfd armed **only while the pointer rests on a module that has
a tooltip**, and disarmed the moment it leaves: no timer at all otherwise.
The surface exists only while shown. No tooltip is ever drawn for a module
with none.

## Details

Position through the popup positioner so it constraint-adjusts at screen edges
(`popup-constraint-adjustment-done.md`); text wraps at a maximum width; a
tooltip never takes the keyboard or steals a click. It also updates in place
if its module's text changes while shown (a clock tooltip), with damage limited
to the popup.

## Done when

Hover shows and leave hides on headless scoot, no timer is armed off a
module, and a tooltip at the screen edge stays on screen.
