---
title: "Pointer-button presses don't activate browser modal buttons, and virtual button hold doesn't persist across IPC calls"
status: "open"
area: "core"
priority: "high"
blocked: null
---

# Pointer-button presses don't activate browser modal buttons, and virtual button hold doesn't persist across IPC calls

Filed 2026-10-05 from the f459 live proof (PR #459 follow-up). Serves
**daily-drive** (a user who cannot click a browser dialog cannot use the
desktop) and **computer use** (an agent driving `scoot msg pointer`
cannot press dialog buttons or drag).

## The gap

Two related pointer-button failures, reproduced live on the Asahi M2 in
a real greetd login (scoot-test, seat0, build `g19iyp4...-scoot-0.1.0`
from `397a49701`), driven both ways the brief names — (a) a uinput
relative mouse (`click.py`, `BTN_LEFT` press/release, the real `--tty`
libinput path) and (b) `scoot msg pointer click` / `pointer button` —
against a probe page logging `pointerdown` with client coordinates,
foot with SGR mouse reporting, and screenshots after every step
(`f459-chrome*.png`, `f459-ff*.png`, `f459-foot*.png`,
`f459-track.png` for the uinput press/release sequences, beside the
f459 report).

What works (so this is not "clicks are broken"):

- (b) into web content is exact: Chrome page logs each click with the
  expected client coordinates (two points fit `client = compositor -
  (857, 159)` exactly); Firefox logs them subpixel
  (`down #2 @124,137.33… btn=0`). Both pages' buttons activate on the
  first click.
- (b) into browser chrome works: Chrome's getDisplayMedia "Entire
  Screen" tab selects on one click (and opens the portal chooser).
- (a) into web content works: the uinput press/release logs
  `down #1 @543,385 btn=0` in the page.
- (a) into foot works: with SGR mouse tracking on, one uinput click
  prints the full pair `^[[<0;46;26M^[[<0;46;26m` (press `M` and
  release `m`).
- (b) double-click into foot works: two rapid `pointer click`s on one
  word select it (`marker` highlighted).
- Moves and keyboard work everywhere throughout.

What fails:

1. **Browser modal buttons never activate.** Chrome's getDisplayMedia
   "Share" button: two `pointer click`s dead-center (proven exact
   coordinates) do nothing; the stream only starts via Tab+Return.
   Firefox's permission-doorhanger "Allow": two `pointer click`s do
   nothing, and a separated `pointer button left press` … screenshot …
   `pointer button left release` also does nothing. Cursor visibly sits
   on the target in the screenshots.
2. **A virtual button hold does not survive across IPC calls.**
   `pointer move` to text, `pointer button left press`, `pointer move`
   to extend, `pointer button left release` (four separate IPC calls)
   selects nothing in foot — while two rapid `pointer click`s select
   the word. Either each `button press` is released inside its own
   dispatch, or the held state resets between IPC requests. Agents
   cannot drag (text selection, sliders, window moves) until this
   holds.

## What to do

- Reproduce headless first: a test client with a popup grab (or the
  synthetic equivalent) asserting button press/release delivery to the
  grabbed surface, and a virtual-pointer test asserting held state
  across two IPC dispatches. Both should FAIL before the fix.
- Trace where a press aimed at a grabbed/modal surface goes today
  (wrong surface? press without release? release without press?) and
  where the virtual button state lives across IPC dispatches; fix at
  that layer.
- Edge cases: press that lands while a grab starts/ends mid-gesture,
  button held when the client disconnects, press/release split across
  a compositor restart of the input path, multi-button holds.

## Not in this ticket

- The `XDG_SESSION_TYPE` export that caused this investigation (PR
  #459 follow-up `397a49701`): fixed and proven separately.
- Chrome's first-run and Firefox's broken default profile for a fresh
  user (worked around live with keyboard and `--profile`): setup
  papercuts, not pointer bugs.
