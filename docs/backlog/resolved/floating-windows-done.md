---
title: "Floating windows: dialogs and chosen apps float above the scrolling layout, and move and resize with the pointer — RESOLVED"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Floating windows

**RESOLVED (2026-09-25) in two PRs.** PR 1 (#242) is the floating layer,
map-time auto-float, window rules, the toggle and the focus switch, and
IPC. PR 2 is moving and resizing: the modifier drag, the client's own
`xdg_toplevel.move`/`resize`, IPC `move-floating`/`resize-floating`, and
the two notes carried from PR 1's re-review. PR 2's record comes first;
PR 1's design record and the original entry follow.

## PR 2 record: moving and resizing

### Decisions

- **Core state: an anchor, not a centre.** `Floating::centre` became
  `Floating::anchor`: a point (relative to the output's area origin, as
  before) and, per axis, which point of the window sits on it (start,
  middle, end). Placement is `floating_move.rs`'s `floating_rect`, the only
  placement arithmetic for floating windows (`arrange` and the new
  allocation-free `World::floating_geometry` both use it). A window keeps
  the point it is held by when it draws a new size: its middle for a
  centred or moved window (PR 1's rule, unchanged), the edge a resize did
  not move for a resized one -- so a terminal rounding to whole cells, or a
  client refusing a size, never moves the edge the user did not drag.
- **Two core actions**, platform-independent and property-tested:
  `Action::MoveFloating { id, x, y }` (the top-left corner; clamped into
  the usable area; the window keeps its alignment) and
  `Action::ResizeFloating { id, size, edges }` (the edges named move, the
  others hold; per resized axis the size is clamped to the window's
  `SizeHints` -- `max` is new, the minimum wins where they disagree -- to
  the room between the fixed edge and the far side of the usable area, and
  to at least 1; an axis with no moving edge keeps its placed size). A
  resize holds the anchor the window already has when it already holds that
  edge from inside the usable area, so the resizes of one drag never drift
  even when the client over-draws and is shifted in by the clamp (a core
  test fails if the pin is recomputed from the placed rect).
- **Crossing outputs is cheap, so it is done.** A move whose asked-for
  rect has its middle over another output (with a usable area) carries the
  window to that output's active workspace, on top, focused there when it
  was the focused window (focus follows it); a middle over no output keeps
  it on its own output, clamped. It is the one allocating step of a move
  (`normalize`, `fix_view`), once per crossing, and the shell answers it
  with one full `apply()`.
- **The carried stacking note, fixed in general.** A window's own floating
  dialogs (parent chain, as `descends_from`) are drawn above it whatever
  the stack says (`floating_order.rs`): the drawing order is a walk of a
  forest whose parent links are "nearest ancestor above me in the stack".
  Clicking a floating parent (or a fullscreen game) still raises and
  focuses it; its dialog stays drawn over it. This covers the re-review's
  case (game clicked above its dialog, then another window focused: the
  dialog showed under the game) and the non-fullscreen one it implies (a
  floated app clicked above its modal dialog hid it completely, the same
  "looks like a hang" harm). The common case -- no floating window has a
  floating ancestor, which includes every dialog of a tiled window -- is
  allocation-free (a parent-chain walk per floating window); a workspace
  with nested floating windows builds the order in three `Vec`s, O(n log n).
- **Tiled windows.** A modifier press on a tiled window is an ordinary
  click, and a tiled window's `xdg_toplevel.move`/`resize` is ignored
  (debug log): tiled windows are placed by the strip, and a toolkit whose
  request goes unanswered stays in its own drag with nothing moving. The
  niri-style "drag a tiled window out to float it" was considered and not
  taken: it changes a window's layer as a side effect of a titlebar drag,
  which a user reaching for a GTK headerbar in the strip does not expect.
- **The modifier.** `[floating] modifier`, Super by default (every default
  binding's modifier), any single modifier name a `[binds]` combo accepts;
  a bad value warns and falls back. Reloadable. It exists mainly for
  `--nested`, where the host usually keeps Super.
- **One grab** (`scoot/src/compositor/floating/grab.rs`), started by either
  path. The client request is honoured only if `pointer.has_grab(serial)`
  (the serial is the press serial of the live implicit grab: the button is
  still held) and that grab's focus is the requesting client's surface; a
  modifier drag's own grab has no focus, so no client can take it over.
  The grab clears pointer focus (the client gets `leave`), swallows the
  modifier press and every button, shows the grab/resize cursor (set after
  the focus clear, whose `leave` resets the cursor -- found on the `--tty`
  screenshot, fixed, pinned by test), and per motion calls the core and
  moves the element with `Space::relocate_element` (no `apply()`).
- **Lock discipline.** Every grab callback runs inside Smithay's pointer
  mutex; `apply()` can reach the pointer. What needs a full `apply()` (the
  end, a crossing) sets `State::floating_grab_resync`, drained by
  `settle_floating_grab` right after the pointer call returns (in
  `move_absolute` and `pointer_button`), and cleared by any `apply()`.
- **Focus back through the arrival path.** The grab's own ends unset
  without Smithay's focus restore; settling re-derives pointer focus
  through `move_absolute`, so the window under the pointer gets its
  `enter` the ordinary way -- and a persistent pointer lock on a dragged
  window re-arms, as after a session unlock (a relative-pointer test fails
  with Smithay's restore instead).
- **Resize configures are paced to the client.** A configure carries
  `resizing` and the size asked for, and goes out only when that size
  changed and the client has acked every configure before it; the
  client's resized frame (`observe_frame` -> `apply()`) and the next motion
  after an ack send the newest. A 1000Hz mouse no longer queues sizes a
  60Hz client must skip, and Smithay's per-configure allocations (11,
  measured) happen at the client's rate. The drag's end drops `resizing`.
  Limits are re-read into the core when a resize drag starts (the core's
  copy of `min` is otherwise refreshed only on a title/app-id/parent
  change; that staleness is PR 1's and unchanged for tiled windows).
- **Ending.** Release of the button that started it; any press (a second
  button, or the same one after a lost release); the window closing (at
  once, from `remove_window`), un-floating, going fullscreen or being
  hidden by a workspace switch (checked per motion); the session locking
  (the lock transition's `drop_input_grabs`); a VT switch
  (`session_event`'s pause: its release would never arrive); an output
  changing size (the geometry it started from no longer holds). An output
  being *removed* cannot happen in scoot today (nothing sends
  `OutputRemoved`); the per-motion check would end the grab if the window
  left the space.
- **IPC.** `move-floating ID X Y` and `resize-floating ID W H` (unsigned;
  keeps the top-left corner, i.e. `Edges::BOTTOM_RIGHT`), additive, no
  `PROTOCOL_VERSION` bump; `windows`' `rect` reports the result.
- **The carried doc note.** `protocols.md` no longer says direct scanout
  composites under a dialog: the frame stays eligible and Smithay
  composites only when the dialog cannot get a plane.
- **Not changed:** `focus-column left|right` with a floating window
  focused still returns to the strip (PR 1 said to revisit once windows
  can move); geometric floating focus is a design of its own and nobody has
  asked for it. X11 move/resize stays refused (XWayland Phase 1).

### Evidence

In the PR description: core property tests (`world/tests/floating_move.rs`,
the randomized invariants extended with both actions, wild values,
cross-output moves and a drawn-above-ancestors invariant), 22 harness
tests (`floating/tests/drag.rs`) plus a relative-pointer test, mutation
checks for each guard, the headless and `--tty` live runs
(`~/evidence/fpg/`), the pointer-motion bench before/after and the
allocation probe (0 allocations per motion on the no-drag and move paths
and per unanswered resize motion; a resize allocates only for the
configures it sends, ~11 each, about one per client ack).

## PR 1 design record

### Prior art (behaviour only; no code or config read or copied)

- **niri** (GPL-3.0; its wiki, `Floating-Windows`): a floating layout per
  workspace/monitor, always above the tiled one; windows with a parent or a
  fixed size float automatically; an `open-floating true|false` window rule
  forces it or disables the heuristic; `switch-focus-between-floating-and-tiling`
  moves focus; with a floating window focused the directional binds act on
  the floating layout; `toggle-window-floating`; move by IPC
  (`move-floating-window`) and by dragging.
- **sway**: `for_window [criteria] floating enable|disable`, criteria
  matched by regex; transient windows (a parent) and fixed-size windows
  (min == max) float by default; `floating toggle` on `Mod+Shift+space`,
  `focus mode_toggle` on `Mod+space`; floating windows are centred.
- **Hyprland**: `windowrule = float, <class/title regex>`; dialogs float by
  default.

All three agree on the heuristics (parent, fixed size) and on rules as the
override in both directions; scoot adds the one standard signal they
predate or use too (`xdg-dialog-v1`), takes sway's keys (the most widely
known for exactly these two actions), and differs on matching (globs, below).

### Decisions

- **Core state** (`scoot-core/src/world/floating.rs`, `tree.rs`): each
  workspace has a floating stack (bottom first; the most recently focused on
  top) and a flag for whether its focus is on that layer or its strip; each
  floating window carries its centre (relative to its output's area
  origin, so a bar appearing does not move it), the size the core asks
  for (usually none), and the width preset it had as a column. The strip is
  computed as if the layer did not exist; the randomized invariant test
  checks every tiled rect is unchanged across floating-only steps.
- **Placement**: centred once, when it floats, on the part of its parent
  inside the usable area, when the parent is on the same output and
  workspace, else on the usable area; then kept as a centre point, so a
  window that resizes itself grows around it. Always inside the usable area
  (bars excluded). Re-centred (on the parent, or the output) when carried to
  another output, when its output changes size, when adopted from an
  unplugged output, and when it waited for an output with none left --
  review round 1 measured a 4K->1080p change clamping a dialog into a corner.
  `arrange` does no parent walk except for the dialogs of a covering
  fullscreen window.
- **Pointer re-entry** when what floats changes under a still pointer:
  accepted in review. What is drawn under the pointer is what a click
  reaches (the same rule fullscreen covering already follows); the cost is
  that a click aimed at the window beneath in the instant a dialog appears
  lands on the dialog, which is the window the user is looking at.
- **Menus** from tiled windows draw below floating windows (a popup draws
  with its window): accepted as ordinary stacking.
- **Globs have no escape**: a literal `*` or `?` cannot be matched as
  itself; `?` (any one character) stands in. Documented.
- **Size**: the client's choice (configure `0x0`), placed at what it drew
  (`FrameObserved.actual`); a rule's `size`, or a window that drew larger
  than the usable area, makes the core ask for a clamped size from then on
  (sticky, so asking to fit does not oscillate). Invisible until it has
  drawn, unless it was asked for a size.
- **Focus**: `toggle-floating-focus` (`Super+Space`) switches layer. A
  workspace's floating layer has focus when its flag says so *or* its strip
  is empty; a column going into an empty strip without focus (un-floating
  by id, a window opening unfocused) sets the flag, so the floating window
  that had focus by default keeps it -- review round 1 found `set-floating
  ID off` moving focus there, contradicting its own contract. The randomized
  test now asserts both never move the focused window. With a
  floating window focused: `focus-column left|right` returns to the strip's
  focused column without stepping (floating windows are all centred in
  PR 1, so geometric left/right has nothing to go on -- revisit in PR 2
  once they can be moved); `focus-window up|down` cycles the stack (down
  raises the bottom-most, up sends the top to the bottom); the strip-only
  actions do nothing. A focused floating window closing hands focus to its
  parent on the same workspace (decided before the removal, so a dialog
  alone on an inactive workspace cannot refocus the workspace that slides
  into its index; and not to a parent stacked behind a fullscreen sibling,
  which would end that fullscreen -- both found in review, pinned by core
  tests), else the next floating window, else the strip.
- **Floating and un-floating**: floating takes the window out of its
  column; strip focus moves to the column on its left, and a window that
  floats before it ever drew (the map-time case) also puts the strip's
  scroll back, so the strip is exactly as it was. Un-floating inserts the
  window as a column right of the strip's focused column (the likely
  answer) at its old width -- so a focused column floated and un-floated
  comes back right of its left neighbour, where it was, except the leftmost
  column (it comes back second: `[1,2]` -> `[2,1]`) and a window floated out
  of a stacked column (it comes back as a column of its own).
- **Fullscreen**: a floating window can go fullscreen, and leaving returns
  it to floating (`0x0`, no tiled state). One set of rules for a fullscreen
  window in either layer (review round 1 found the floating case hid the
  window and let the strip show through): focused, it covers, and every
  other floating window hides except **its own dialogs** (parent chain), which
  stay up, placed above it -- review found that clicking a fullscreen app hid
  its modal dialog, which looks like a hang; under a focused floating window
  it stays in place, full size (a floating one then hides the strip), and
  nothing covers (the `top` layer shows, direct scanout pauses); focused
  elsewhere, it is not in front. Floating or un-floating a fullscreen window
  ends the fullscreen first.
- **Auto-float, decided once at the first commit** (`scoot/src/compositor/floating.rs`):
  `xdg-dialog-v1` is implemented by the pinned Smithay fork (`dialog.rs`),
  so it is advertised (no fork change); then a parent; then a fixed size;
  all behind `[floating] auto` (default on). A later hint, parent or title
  change re-decides nothing; a re-map keeps the state. The window has
  already been sent a tiled configure at creation (`add_window`), so the
  decision adds a second configure in the same flush; on the wire every
  client measured drew its first frame at its own size.
- **Rules** (`window_rules.rs`): `[[window_rule]]` with `match_app_id` /
  `match_title` **globs** (`*`, `?`, whole string, case-sensitive) rather
  than regexes: the common rules are exact app ids (a regex needs anchors,
  and `foot` unanchored matches `footclient`) and title fragments; no new
  dependency and a bounded matcher; nothing needs captures. `float` and an
  optional `size`; later matching rules override earlier ones per field;
  a rule needs a matcher and must set something, else it is skipped with a
  warning (and refused by name on reload). Reloadable; map-time only.
- **Wire**: floating windows get no tiled states (the configure says so in
  the same configure as the size); rings are drawn directly under their own
  window so they show over the strip; pointer focus is re-derived when what
  floats on screen changes; IPC `windows` gains `floating`; actions
  `toggle-floating`, `set-floating ID on|off`, `toggle-floating-focus`
  (additive; no `PROTOCOL_VERSION` bump). Popups use PR #225's target
  unchanged (the window's output usable area); output clip (#224) and
  captures unchanged; primary-direct needs nothing (a floating fullscreen
  window covers like any other, and nothing covers while a floating window
  above a fullscreen one has focus).
- **X11 later**: the core takes `WindowInfo::parent` and
  `Event::FloatingRequested` from any platform, so XWayland Phase 2 maps
  `WM_TRANSIENT_FOR` and window types onto the same two inputs.

### Evidence

In the PR description: core property tests, 22 harness tests
(`floating/tests/`), the live dev VM run (`~/evidence/float/live/`), and
benchmarks.

---

The original entry follows.

User request, 2026-09-24: "we should investigate and implement a way to allow
specified windows to float. Like some other tiling ones do for things like
pop up confirmations or settings pop ups." Serves **daily-drive** first (a
file-save dialog or a settings window squeezed into a full-height column is
the most common "this compositor feels wrong" moment) and **computer use**
(an agent expects a confirmation dialog on top and centred, not a new column
off-screen).

## What is wrong today

Every `xdg_toplevel` is an independent column entry, dialogs included
(`resolved/wlr-foreign-toplevel-management-done.md`); `scoot-core` has no
floating concept. PR #240 made a short dialog at least draw and ring
correctly in its slot, but it still takes a column and scrolls the strip.

## What to decide (design first; the prior-art survey is part of the work)

- **Prior art to survey** (behaviour only — niri is GPL-3.0, OmniWM
  GPL-2.0: read for design, copy no code; sway/Hyprland similarly for
  behaviour): niri's floating layer + `window-rule { open-floating true }`
  and its default floating of dialogs; sway `for_window [...] floating
  enable` and its dialog heuristics; Hyprland `windowrule = float, ...`.
- **Which windows float automatically**, in order of signal strength:
  - `xdg-dialog-v1` (`xdg_wm_dialog_v1.get_xdg_dialog`, modal hint) —
    the standard for this; check Smithay's support at the pinned fork
    (`scoot-sh/smithay` `43f50eb`) and follow `CLAUDE.md`'s "standard
    protocol first" rule;
  - an `xdg_toplevel` with a parent (`set_parent`) — transient windows;
  - fixed size (min == max size set) — non-resizable dialogs;
  - decide whether the heuristics are on by default and configurable.
- **Window rules** in config (`[[window_rule]]` matching `app_id` /
  `title`, regex or glob, with `float = true` and perhaps an initial size /
  position) — the "specified windows" the user asked for. Reloadable
  (`scootctl reload`, see config-reload work).
- **Toggle**: an IPC action (`scootctl action toggle-floating`, and by-id
  `set-floating ID on|off` like fullscreen's pair) and a default bind.
- **Layout semantics in `scoot-core`** (platform-independent, fuzzed;
  `CLAUDE.md` says touch only for a real reason — this is one): a
  per-workspace floating set above the strip; position (centred on its
  parent / output, clamped to the usable area), size (client's preferred,
  clamped), stacking order and raise-on-focus, focus movement between
  floating and tiled (a keybind to switch), what `focus-column` does with
  a floating window focused, workspace moves, multi-output, fullscreen
  interplay (#223), what un-floating does (insert as a column where?).
- **Move/resize**: pointer move (Mod+drag) and resize for floating windows,
  or IPC-only first; `xdg_toplevel.move`/`resize` requests from CSD
  clients.
- **Knock-ons**: no tiled states for floating windows (PR #240 sends them
  to layout windows only — floating must not get them, so foot etc.
  cell-round again, which is correct), rounded clip + ring around floating
  windows, output clipping (#224), hit-testing and z-order in
  `window_under`, IPC `windows` (a `floating` field; `rect` is the real
  position), foreign-toplevel protocols, screenshots/primary-direct.

## Evidence expected

Core property/fuzz tests for the floating set; harness tests for auto-float
(dialog protocol, parent, fixed size), rules, toggle, focus and stacking;
live runs with real dialogs (GTK file chooser via `zenity --file-selection`,
a GTK settings/about dialog, `foot` via rule) with screenshots; docs for the
config syntax, binds and IPC actions (README gets a short user-facing line —
it is for users and prospective users).

## Carried into PR 2 from the PR #242 re-review (2026-09-25) -- both fixed in PR 2

- **A floating fullscreen game can end up above its own dialog.** Clicking
  the game raises it above its dialog; if an unrelated floating window then
  takes focus, the game shows behind it but its dialog stays hidden beneath
  the game. Clicking the game brings the dialog back. Rare; fix by keeping a
  window's own floating dialogs stacked above it whenever both show.
- **Doc precision (`docs/protocols.md`):** "With a dialog above it, direct
  scanout composites" overstates it. The frame stays eligible and Smithay
  only composites when the dialog cannot get a plane of its own; on
  overlay-capable hardware it may still go direct, which is correct. Reword.
