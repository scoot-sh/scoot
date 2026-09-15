---
title: "The four protocols `foot` warned about: cursor-shape, xdg-activation, toplevel-icon, text-input/input-method — DONE"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# The four protocols `foot` warned about — DONE

~~Starting `foot` under flexwm prints four warnings about protocols the
compositor does not offer~~ — DONE, all four implemented together (issue #40).

## What landed

- **`wp-cursor-shape-v1`** (version 2). `CursorShapeManagerState`, plus
  `TabletSeatHandler` (which the protocol's dispatch is bounded on whether or
  not a compositor offers `zwp_tablet_manager_v2` — flexwm does not, so no
  client can construct the tablet half). Smithay routes `set_shape` straight
  into `SeatHandler::cursor_image` as a `CursorImageStatus::Named`, the same
  path `wl_pointer.set_cursor` with no surface already took.

  The substance is on flexwm's side: `cursor/shapes.rs` **draws ten shapes
  procedurally** so that naming a shape actually gets you that shape. Without
  it, advertising the protocol would have been a *regression* in a real
  session — a client that had been uploading a proper I-beam from its own
  xcursor theme would switch to cursor-shape and get one blob triangle for
  every name. Each shape is stamped into a coverage mask by a small
  rasterizer (`stroke_line`/`fill_triangle`/`stroke_circle`) and inked with
  an 8-way dilated outline; the arrow keeps `cursor.rs`'s own byte-identical
  generator, because its outline is drawn *inside* a solid body while every
  shape here is line art one or two pixels thick. All ten are built once at
  startup, indexed by an array slot at render time — no allocation on the
  render path, and stable buffer `Id`s for the damage tracker.

- **`xdg-activation-v1`** (version 1). `compositor/activation.rs`. Two policy
  bounds, both on the token: valid for 30s, and at most 64 unredeemed tokens
  at once with expired ones swept first (`get_activation_token` is
  unauthenticated and unlimited, and nothing upstream prunes the table — the
  same resource-exhaustion family as the `wl_shm` pool cap). A redeemed token
  is removed whether honored or not. Activation runs through `State::act`,
  so it inherits the session-lock gate and the core's scroll-into-view.

- **`xdg-toplevel-icon-v1`** (version 1). `compositor/toplevel_icon.rs`.
  Stores nothing: the icon lives in the surface's own double-buffered state,
  and `State::icon_name_of` resolves the *current* one when asked — so it can
  never report an icon the client attached but has not committed, and there
  is no second copy to invalidate on unmap/destroy. Exposed as
  `WindowSnapshot::icon` (optional on the wire, so no `PROTOCOL_VERSION`
  bump). No preferred icon sizes are advertised: flexwm draws no icon, so it
  has no size to prefer, and an empty list is the protocol's own way to say
  so.

- **`text-input-v3` + `input-method-v2`** (version 1 each).
  `compositor/input_method.rs`. Smithay owns the middle of this — text-input
  focus follows keyboard focus inside its own seat, and preedit/commit
  traffic is forwarded between the two protocols. What flexwm owns is the
  IME popup: tracked as a `PopupKind::InputMethod` against whichever surface
  holds the field, which is what makes both `Window` and `LayerSurface`
  render it (and send it frame callbacks) with no render-path change at all.
  `parent_geometry` answers for a layer surface as well as a window, because
  keyboard focus in flexwm can be on a launcher's search field.

## Evidence

Measured against this branch's tree, in the session container (Ubuntu 24.04,
`foot` 1.16.2, a debug build), not narrated:

- `cargo test --workspace`: 465 + 68 + 26 pass, 1 ignored (`shape_art`, which
  prints the shapes as ASCII art for a human and asserts nothing).
  `cargo clippy -p flexwm --all-targets -- -D warnings` and
  `cargo fmt --check -p flexwm` are clean.
- `scripts/smoke-test.sh` (`--headless`, `FLEXWM=target/debug/flexwm`): exit
  0, zero `BUG` lines, including the decoration pixel checks.
- Before/after on the *actual* warnings, same `foot`, same probe (start
  headless, `msg action spawn foot`, grep the log):

  ```
  === BEFORE (main, f0463f9) ===
  warn: wayland.c:1503: no XDG activation support; bell.urgent will fall back to coloring the window margins red
  warn: wayland.c:1512: no server-side cursors available, falling back to client-side cursors
  warn: wayland.c:1523: text input interface not implemented by compositor; IME will be disabled
  === AFTER (this branch) ===
  (none of the four warnings)
  ```

  `foot` 1.16.2 does not warn about `xdg-toplevel-icon` at all (that warning
  comes from the newer `foot` in the dev VM, which is what the issue
  reported); the global is implemented and covered by its own live-client
  tests either way.

### Performance

The only path this change touches that runs per frame is `Cursor::element`
(and only under `--tty`, the one backend that draws a cursor at all). Release
build, 10 000 iterations per run, three runs each side, same container:

| | `main` (f0463f9) | this branch |
| --- | --- | --- |
| `Cursor::element`, warm | 173 / 186 / 208 / 210 ns | 187 / 218 / 233 / 207 ns (default), 190 / 217 / 243 / 196 ns (text) |

Run-to-run spread is ±35 ns on both sides and the two ranges overlap
throughout, so there is no measurable per-frame difference — which matches
what the diff does there: one extra `match` arm, a jump-table lookup
(`Shape::for_icon`) and an array index, replacing a direct field read. For
scale, ~200 ns is 0.001% of a 16.7 ms frame.

What did get more expensive is startup, once, by construction — ten bitmaps
instead of one:

| `Cursor::new` | `main` | this branch |
| --- | --- | --- |
| default 16px | 853 ns | 16.8 µs |
| 48px | 1.5 µs | 118 µs |
| 256px (`MAX_CURSOR_SIZE`) | 49 µs | 4.9 ms |

16 µs at the default is not worth a second thought. The 4.9 ms at the largest
size a config can ask for is recorded rather than hidden: it is one-time, at
startup, only for a session that configures `cursor_size = 256`, and it buys
ten shapes that are then free forever (built once, indexed by array slot, no
allocation on the render path, stable buffer `Id`s for the damage tracker).
Building them lazily would trade that for a first-use allocation while the
pointer is moving, which is the worse moment.

Not covered here, and unchanged by this work: real `--tty` hardware. Nothing
in the four protocols is backend-specific except *drawing* the cursor, which
only `--tty` does — the shapes themselves are asserted pixel-by-pixel in
`cursor/shapes/tests.rs` and rendered end-to-end through a real
`PixmanRenderer` in `cursor/tests.rs`, which is the same renderer `--tty`
scans out.

## What this deliberately leaves open

- **Toplevel icon *buffers*.** Only the icon name is exposed. A client may
  supply raw square `wl_shm` buffers instead, which
  `ToplevelIconCachedState::buffers` holds and nothing reads; handing those
  to an IPC client means re-encoding shm to PNG per query, the way
  `screenshot.rs` does for the screen, and no consumer has asked yet. Such a
  client reads as having no icon.
- **No `XDG_ACTIVATION_TOKEN` when flexwm spawns.** `Action::Spawn` does not
  mint an external token for the child, so a compositor-spawned app cannot
  activate itself the way a launcher-spawned one can. `create_external_token`
  is the upstream hook for it.
- **Cursor shapes beyond the ten.** `help`, `wait`, `progress`, `pointer`,
  `zoom-in`/`zoom-out`, `alias`, `copy` and `context-menu` all draw the
  arrow, deliberately: a hand or an hourglass is not distinguishable as line
  art at the default 16px, and drawing a near-duplicate nobody can read is
  worse than the honest fallback.
