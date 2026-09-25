//! Moving and resizing floating windows with the pointer, through real
//! pointer events and a real client: the modifier drag, the client's own
//! `xdg_toplevel.move`/`.resize` (and the serials it may and may not use),
//! every way a drag ends, IPC `move-floating`/`resize-floating`, and a
//! window's dialogs staying drawn above it.
//!
//! The modifier is held with a real key event (`State::key`), so what is
//! read is the seat keyboard's own modifier state, as on a real session.

use scoot_core::{Action, OutputId};
use scoot_ipc::{PointerButton, Request, Response};
use smithay::backend::input::KeyState;
use smithay::input::keyboard::Keycode;

use super::*;
use crate::compositor::headless;

/// `KEY_LEFTMETA` and `KEY_LEFTALT`, as xkb keycodes (evdev + 8).
const SUPER: u32 = 125 + 8;
const ALT: u32 = 56 + 8;

fn hold(fixture: &mut Fixture, keycode: u32, held: bool) {
    let state = if held {
        KeyState::Pressed
    } else {
        KeyState::Released
    };
    fixture.state.key(Keycode::new(keycode), state);
    fixture.settle();
}

/// A tiled window with a floating dialog over it; answers the dialog's
/// index.
fn dialog_scene() -> (Fixture, usize) {
    let mut fixture = Fixture::new();
    fixture.map(Spec::tiled());
    let dialog = fixture.map(Spec::dialog_of(0));
    assert!(fixture.floating(dialog));
    (fixture, dialog)
}

fn press(fixture: &mut Fixture, button: PointerButton) {
    fixture.state.pointer_button(button, true);
    fixture.settle();
}

fn release(fixture: &mut Fixture, button: PointerButton) {
    fixture.state.pointer_button(button, false);
    fixture.settle();
}

fn move_pointer(fixture: &mut Fixture, x: i32, y: i32) {
    fixture.state.pointer_move(f64::from(x), f64::from(y));
    fixture.settle();
}

/// Where the space has the window's element: what is drawn and hit-tested.
fn space_location(fixture: &Fixture, index: usize) -> Option<(i32, i32)> {
    let window = fixture.state.window(fixture.id(index))?.clone();
    fixture
        .state
        .space
        .element_location(&window)
        .map(|at| (at.x, at.y))
}

/// Holds Super and presses `button` in the middle of the dialog: a drag
/// under way. Answers where the dialog was.
fn start_modifier_drag(fixture: &mut Fixture, dialog: usize, button: PointerButton) -> Rect {
    let rect = fixture.placement(dialog).rect;
    let (cx, cy) = centre(rect);
    move_pointer(fixture, cx, cy);
    hold(fixture, SUPER, true);
    press(fixture, button);
    assert_eq!(
        fixture.state.floating_grab_window(),
        Some(fixture.id(dialog)),
        "the drag began"
    );
    rect
}

#[test]
fn a_modifier_left_drag_moves_a_floating_window_and_it_stays() {
    let (mut fixture, dialog) = dialog_scene();
    let before = start_modifier_drag(&mut fixture, dialog, PointerButton::Left);
    let (cx, cy) = centre(before);
    move_pointer(&mut fixture, cx + 30, cy + 20);
    let moved = Rect::new(before.x + 30, before.y + 20, before.w, before.h);
    assert_eq!(fixture.placement(dialog).rect, moved);
    assert_eq!(
        space_location(&fixture, dialog),
        Some((moved.x, moved.y)),
        "drawn where the core has it, between arrangements"
    );
    let pixels = fixture.render();
    let (mx, my) = centre(moved);
    assert_eq!(pixel(&pixels, mx, my), DIALOG_BGRA);
    // The drag shows its own cursor (set after the focus clear, whose
    // `leave` resets it)...
    assert!(
        matches!(
            fixture.state.cursor.status(),
            smithay::input::pointer::CursorImageStatus::Named(
                smithay::input::pointer::CursorIcon::Grabbing
            )
        ),
        "{:?}",
        fixture.state.cursor.status()
    );
    // The client never saw the press, and has no pointer focus mid-drag.
    assert!(fixture.buttons().is_empty());
    assert_eq!(fixture.pointer(), None);
    // What an agent reads is where it went.
    let rect = fixture.snapshot(dialog).rect;
    assert_eq!((rect.x, rect.y), (moved.x, moved.y));
    release(&mut fixture, PointerButton::Left);
    hold(&mut fixture, SUPER, false);
    assert_eq!(fixture.state.floating_grab_window(), None);
    assert!(fixture.buttons().is_empty(), "nor the release");
    assert_eq!(
        fixture.pointer(),
        Some(Entered::Window(dialog)),
        "the pointer re-enters what it is over"
    );
    // Kept by the core: a relayout for something else leaves it there.
    fixture.act(Action::FocusWindowId(fixture.id(0)));
    assert_eq!(fixture.placement(dialog).rect, moved);
    // And the pointer is no longer grabbed: the next click is a click.
    let (tx, ty) = centre(fixture.placement(0).rect);
    fixture.click(f64::from(tx), f64::from(ty - 60));
    assert!(!fixture.buttons().is_empty());
}

#[test]
fn a_modifier_right_drag_resizes_from_the_nearest_corner() {
    let (mut fixture, dialog) = dialog_scene();
    let before = fixture.placement(dialog).rect;
    // Near the bottom-right corner: that corner moves, the top-left stays.
    move_pointer(&mut fixture, before.right() - 3, before.bottom() - 3);
    hold(&mut fixture, SUPER, true);
    press(&mut fixture, PointerButton::Right);
    // The client answers what it has been sent (the press focused it), as a
    // live client does before the drag gets anywhere.
    fixture.done(Step::Draw { window: dialog });
    move_pointer(&mut fixture, before.right() + 27, before.bottom() + 17);
    let asked = fixture.last_configure(dialog);
    assert_eq!((asked.width, asked.height), (before.w + 30, before.h + 20));
    assert!(asked.resizing, "{asked:?}");
    assert!(
        !asked.any_tiled,
        "a floating window is never told it is tiled"
    );
    fixture.done(Step::Draw { window: dialog });
    let resized = fixture.placement(dialog).rect;
    assert_eq!(
        resized,
        Rect::new(before.x, before.y, before.w + 30, before.h + 20)
    );
    assert_eq!(space_location(&fixture, dialog), Some((before.x, before.y)));
    release(&mut fixture, PointerButton::Right);
    hold(&mut fixture, SUPER, false);
    let last = fixture.last_configure(dialog);
    assert!(!last.resizing, "the resize is over: {last:?}");
    assert_eq!((last.width, last.height), (before.w + 30, before.h + 20));
    // The size is kept, asked for from now on.
    assert_eq!(
        fixture.placement(dialog).requested,
        Some(scoot_core::Size::new(before.w + 30, before.h + 20))
    );
}

#[test]
fn a_top_left_resize_keeps_the_bottom_right_corner() {
    let (mut fixture, dialog) = dialog_scene();
    let before = fixture.placement(dialog).rect;
    move_pointer(&mut fixture, before.x + 2, before.y + 2);
    hold(&mut fixture, SUPER, true);
    press(&mut fixture, PointerButton::Right);
    fixture.done(Step::Draw { window: dialog });
    move_pointer(&mut fixture, before.x - 18, before.y - 8);
    fixture.done(Step::Draw { window: dialog });
    let resized = fixture.placement(dialog).rect;
    assert_eq!(
        resized.size(),
        scoot_core::Size::new(before.w + 20, before.h + 10)
    );
    assert_eq!(
        (resized.right(), resized.bottom()),
        (before.right(), before.bottom())
    );
    release(&mut fixture, PointerButton::Right);
    hold(&mut fixture, SUPER, false);
}

/// Configures go out at the rate the client answers them, not the
/// mouse's: while one is unacked, motion only moves the size the core asks
/// for, and the client's next frame brings the newest size.
#[test]
fn a_resize_drag_sends_a_new_size_only_once_the_last_is_answered() {
    let (mut fixture, dialog) = dialog_scene();
    let before = fixture.placement(dialog).rect;
    move_pointer(&mut fixture, before.right() - 3, before.bottom() - 3);
    hold(&mut fixture, SUPER, true);
    press(&mut fixture, PointerButton::Right);
    fixture.done(Step::Draw { window: dialog });
    move_pointer(&mut fixture, before.right() + 7, before.bottom() + 7);
    let sent = fixture.configures(dialog).len();
    let first = fixture.last_configure(dialog);
    assert_eq!((first.width, first.height), (before.w + 10, before.h + 10));
    // Unanswered: more motion sends nothing...
    for step in 1..=5 {
        move_pointer(
            &mut fixture,
            before.right() + 7 + step,
            before.bottom() + 7 + step,
        );
    }
    assert_eq!(
        fixture.configures(dialog).len(),
        sent,
        "a configure per motion"
    );
    // ...until the client draws the first, which brings the newest.
    fixture.done(Step::Draw { window: dialog });
    let newest = fixture.last_configure(dialog);
    assert_eq!(
        (newest.width, newest.height),
        (before.w + 15, before.h + 15)
    );
    assert!(newest.resizing);
    release(&mut fixture, PointerButton::Right);
    hold(&mut fixture, SUPER, false);
}

#[test]
fn without_the_modifier_a_press_on_a_floating_window_is_a_click() {
    let (mut fixture, dialog) = dialog_scene();
    let (cx, cy) = centre(fixture.placement(dialog).rect);
    move_pointer(&mut fixture, cx, cy);
    press(&mut fixture, PointerButton::Left);
    assert_eq!(fixture.state.floating_grab_window(), None);
    move_pointer(&mut fixture, cx + 30, cy + 20);
    assert_eq!(centre(fixture.placement(dialog).rect), (cx, cy));
    release(&mut fixture, PointerButton::Left);
    let buttons = fixture.buttons();
    assert_eq!(buttons.len(), 2, "{buttons:?}");
}

#[test]
fn a_modifier_press_on_a_tiled_window_is_a_click() {
    let (mut fixture, _) = dialog_scene();
    let column = fixture.placement(0).rect;
    move_pointer(&mut fixture, column.x + 5, column.y + 5);
    hold(&mut fixture, SUPER, true);
    press(&mut fixture, PointerButton::Left);
    assert_eq!(fixture.state.floating_grab_window(), None);
    move_pointer(&mut fixture, column.x + 40, column.y + 40);
    assert_eq!(fixture.placement(0).rect, column);
    release(&mut fixture, PointerButton::Left);
    hold(&mut fixture, SUPER, false);
    assert_eq!(fixture.buttons().len(), 2);
}

#[test]
fn the_configured_modifier_is_the_one_that_drags() {
    let (mut fixture, dialog) = dialog_scene();
    fixture.state.floating_modifier = scoot_ipc::Modifier::Alt;
    let (cx, cy) = centre(fixture.placement(dialog).rect);
    move_pointer(&mut fixture, cx, cy);
    hold(&mut fixture, SUPER, true);
    press(&mut fixture, PointerButton::Left);
    assert_eq!(
        fixture.state.floating_grab_window(),
        None,
        "Super no longer drags"
    );
    release(&mut fixture, PointerButton::Left);
    hold(&mut fixture, SUPER, false);
    hold(&mut fixture, ALT, true);
    press(&mut fixture, PointerButton::Left);
    assert_eq!(
        fixture.state.floating_grab_window(),
        Some(fixture.id(dialog))
    );
    release(&mut fixture, PointerButton::Left);
    hold(&mut fixture, ALT, false);
}

/// The swallowed press must not become an activation serial the client
/// could mint a token from: it never received it.
#[test]
fn a_modifier_press_is_not_an_interaction_serial() {
    let (mut fixture, dialog) = dialog_scene();
    let (cx, cy) = centre(fixture.placement(dialog).rect);
    move_pointer(&mut fixture, cx, cy);
    hold(&mut fixture, SUPER, true);
    let before = fixture.state.interaction_serials.latest();
    press(&mut fixture, PointerButton::Left);
    assert_eq!(fixture.state.interaction_serials.latest(), before);
    release(&mut fixture, PointerButton::Left);
    // The release is swallowed too. What the end of the drag does deliver
    // is the pointer's `enter` back onto the dialog, which is recorded as
    // an enter -- one the activation gate never accepts.
    let after = fixture.state.interaction_serials.latest();
    if after != before {
        let (serial, client) = after.expect("an entry");
        assert!(
            !fixture.state.interaction_serials.contains(serial, &client),
            "the end of a drag recorded an input serial"
        );
    }
    hold(&mut fixture, SUPER, false);
}

/// Presses the left button in the middle of `window` (no modifier) and
/// answers the serial the client received with it.
fn client_press(fixture: &mut Fixture, window: usize) -> u32 {
    let (cx, cy) = centre(fixture.placement(window).rect);
    move_pointer(fixture, cx, cy);
    press(fixture, PointerButton::Left);
    let button = *fixture.buttons().last().expect("the client got the press");
    assert!(button.pressed);
    button.serial
}

#[test]
fn a_client_move_request_on_its_held_press_moves_the_window() {
    let (mut fixture, dialog) = dialog_scene();
    let before = fixture.placement(dialog).rect;
    let serial = client_press(&mut fixture, dialog);
    fixture.done(Step::RequestMove {
        window: dialog,
        serial,
    });
    assert_eq!(
        fixture.state.floating_grab_window(),
        Some(fixture.id(dialog))
    );
    let (cx, cy) = centre(before);
    move_pointer(&mut fixture, cx + 25, cy + 15);
    assert_eq!(
        fixture.placement(dialog).rect,
        Rect::new(before.x + 25, before.y + 15, before.w, before.h)
    );
    release(&mut fixture, PointerButton::Left);
    assert_eq!(fixture.state.floating_grab_window(), None);
    move_pointer(&mut fixture, cx, cy);
    assert_eq!(fixture.placement(dialog).rect.x, before.x + 25, "kept");
}

#[test]
fn a_client_move_request_with_a_stale_serial_is_refused() {
    let (mut fixture, dialog) = dialog_scene();
    let before = fixture.placement(dialog).rect;
    let serial = client_press(&mut fixture, dialog);
    release(&mut fixture, PointerButton::Left);
    fixture.done(Step::RequestMove {
        window: dialog,
        serial,
    });
    assert_eq!(fixture.state.floating_grab_window(), None);
    // Nor any other serial while a button is held.
    let held = client_press(&mut fixture, dialog);
    fixture.done(Step::RequestMove {
        window: dialog,
        serial: held.wrapping_sub(1),
    });
    assert_eq!(fixture.state.floating_grab_window(), None);
    let (cx, cy) = centre(before);
    move_pointer(&mut fixture, cx + 25, cy + 15);
    assert_eq!(fixture.placement(dialog).rect, before);
    release(&mut fixture, PointerButton::Left);
}

#[test]
fn a_move_request_on_another_client_s_press_is_refused() {
    let (mut fixture, dialog) = dialog_scene();
    let other = fixture.spawn(run_client);
    // The other client's window floats too (it says it is a dialog), so
    // only whose press it is can refuse the request.
    assert!(matches!(
        fixture.run_on(
            other,
            Step::Map(Spec {
                color: OTHER_BGRA,
                dialog: true,
                ..Spec::tiled()
            })
        ),
        Ack::Done
    ));
    let theirs = fixture.state.windows.len() - 1;
    assert!(fixture.floating(theirs));
    let serial = client_press(&mut fixture, dialog);
    assert!(matches!(
        fixture.run_on(other, Step::RequestMove { window: 0, serial }),
        Ack::Done
    ));
    assert_eq!(fixture.state.floating_grab_window(), None);
    release(&mut fixture, PointerButton::Left);
}

#[test]
fn a_client_resize_request_resizes_within_the_window_s_limits() {
    let mut fixture = Fixture::new();
    fixture.map(Spec::tiled());
    let dialog = fixture.map(Spec {
        min: Some((50, 30)),
        max: Some((100, 60)),
        ..Spec::dialog_of(0)
    });
    let before = fixture.placement(dialog).rect;
    let serial = client_press(&mut fixture, dialog);
    fixture.done(Step::RequestResize {
        window: dialog,
        serial,
        edges: xdg_toplevel::ResizeEdge::BottomRight,
    });
    assert_eq!(
        fixture.state.floating_grab_window(),
        Some(fixture.id(dialog))
    );
    let (cx, cy) = centre(before);
    move_pointer(&mut fixture, cx + 200, cy + 200);
    let asked = fixture.last_configure(dialog);
    assert_eq!((asked.width, asked.height), (100, 60), "the maximum");
    assert!(asked.resizing);
    fixture.done(Step::Draw { window: dialog });
    move_pointer(&mut fixture, cx - 200, cy - 200);
    let asked = fixture.last_configure(dialog);
    assert_eq!((asked.width, asked.height), (50, 30), "the minimum");
    release(&mut fixture, PointerButton::Left);
    assert!(!fixture.last_configure(dialog).resizing);
}

#[test]
fn a_tiled_window_s_move_request_is_ignored() {
    let (mut fixture, _) = dialog_scene();
    let column = fixture.placement(0).rect;
    move_pointer(&mut fixture, column.x + 5, column.y + 5);
    press(&mut fixture, PointerButton::Left);
    let serial = fixture.buttons().last().expect("a press").serial;
    fixture.done(Step::RequestMove { window: 0, serial });
    assert_eq!(fixture.state.floating_grab_window(), None);
    move_pointer(&mut fixture, column.x + 60, column.y + 60);
    assert_eq!(fixture.placement(0).rect, column);
    release(&mut fixture, PointerButton::Left);
    // Its own implicit grab carried on, so its release reached it.
    assert!(fixture.buttons().last().is_some_and(|b| !b.pressed));
}

#[test]
fn closing_the_window_mid_drag_ends_the_drag() {
    let (mut fixture, dialog) = dialog_scene();
    let before = start_modifier_drag(&mut fixture, dialog, PointerButton::Left);
    fixture.done(Step::Destroy { window: dialog });
    assert_eq!(fixture.state.floating_grab_window(), None);
    let (cx, cy) = centre(before);
    move_pointer(&mut fixture, cx + 30, cy + 30);
    release(&mut fixture, PointerButton::Left);
    hold(&mut fixture, SUPER, false);
    assert!(
        !fixture
            .state
            .seat
            .get_pointer()
            .expect("a pointer")
            .is_grabbed()
    );
}

#[test]
fn locking_mid_drag_ends_the_drag() {
    let (mut fixture, dialog) = dialog_scene();
    let before = start_modifier_drag(&mut fixture, dialog, PointerButton::Left);
    fixture.done(Step::LockSession);
    assert!(fixture.state.session_lock.is_locked());
    assert_eq!(fixture.state.floating_grab_window(), None);
    let (cx, cy) = centre(before);
    move_pointer(&mut fixture, cx + 30, cy + 30);
    assert_eq!(
        fixture.placement(dialog).rect,
        before,
        "nothing moves behind the lock"
    );
    release(&mut fixture, PointerButton::Left);
    hold(&mut fixture, SUPER, false);
}

#[test]
fn another_button_ends_the_drag() {
    let (mut fixture, dialog) = dialog_scene();
    let before = start_modifier_drag(&mut fixture, dialog, PointerButton::Left);
    let (cx, cy) = centre(before);
    move_pointer(&mut fixture, cx + 10, cy + 10);
    press(&mut fixture, PointerButton::Right);
    assert_eq!(fixture.state.floating_grab_window(), None);
    let stopped = fixture.placement(dialog).rect;
    move_pointer(&mut fixture, cx + 40, cy + 40);
    assert_eq!(fixture.placement(dialog).rect, stopped);
    release(&mut fixture, PointerButton::Right);
    release(&mut fixture, PointerButton::Left);
    hold(&mut fixture, SUPER, false);
}

#[test]
fn a_workspace_switch_mid_drag_ends_the_drag() {
    let (mut fixture, dialog) = dialog_scene();
    let before = start_modifier_drag(&mut fixture, dialog, PointerButton::Left);
    fixture.act(Action::FocusWorkspace(scoot_core::Vertical::Down));
    let (cx, cy) = centre(before);
    move_pointer(&mut fixture, cx + 30, cy + 30);
    assert_eq!(fixture.state.floating_grab_window(), None);
    release(&mut fixture, PointerButton::Left);
    hold(&mut fixture, SUPER, false);
    fixture.act(Action::FocusWorkspace(scoot_core::Vertical::Up));
    assert_eq!(fixture.placement(dialog).rect, before);
}

#[test]
fn un_floating_mid_drag_ends_the_drag() {
    let (mut fixture, dialog) = dialog_scene();
    let before = start_modifier_drag(&mut fixture, dialog, PointerButton::Left);
    // The press focused the dialog, so the toggle takes it into the strip.
    fixture.act(Action::ToggleFloating);
    assert!(!fixture.floating(dialog));
    let (cx, cy) = centre(before);
    move_pointer(&mut fixture, cx + 30, cy + 30);
    assert_eq!(fixture.state.floating_grab_window(), None);
    release(&mut fixture, PointerButton::Left);
    hold(&mut fixture, SUPER, false);
}

#[test]
fn an_output_resize_mid_drag_ends_the_drag() {
    let (mut fixture, dialog) = dialog_scene();
    start_modifier_drag(&mut fixture, dialog, PointerButton::Right);
    assert!(fixture.state.resize_output(CANVAS + 40, CANVAS));
    fixture.settle();
    assert_eq!(fixture.state.floating_grab_window(), None);
    assert!(
        !fixture.last_configure(dialog).resizing,
        "the resize state went with the drag"
    );
    release(&mut fixture, PointerButton::Right);
    hold(&mut fixture, SUPER, false);
}

#[test]
fn a_drag_onto_another_output_carries_the_window_there() {
    let (mut fixture, dialog) = dialog_scene();
    headless::add_output(&mut fixture.state, "headless-2", CANVAS, CANVAS)
        .expect("a second output");
    fixture.settle();
    let before = start_modifier_drag(&mut fixture, dialog, PointerButton::Left);
    let (cx, cy) = centre(before);
    move_pointer(&mut fixture, cx + CANVAS, cy);
    let placed = fixture.placement(dialog);
    assert_eq!(placed.output, OutputId(2));
    assert_eq!(
        placed.rect,
        Rect::new(before.x + CANVAS, before.y, before.w, before.h)
    );
    assert!(placed.visible);
    assert_eq!(
        space_location(&fixture, dialog),
        Some((placed.rect.x, placed.rect.y))
    );
    assert_eq!(fixture.state.world.focused_output(), Some(OutputId(2)));
    assert_eq!(
        fixture.state.floating_grab_window(),
        Some(fixture.id(dialog)),
        "the drag carries on across the edge"
    );
    release(&mut fixture, PointerButton::Left);
    hold(&mut fixture, SUPER, false);
    let pixels = fixture.pixels_of(OutputId(2));
    let (mx, my) = centre(placed.rect);
    assert_eq!(pixel(&pixels, mx - CANVAS, my), DIALOG_BGRA);
}

#[test]
fn ipc_moves_and_resizes_a_floating_window() {
    let (mut fixture, dialog) = dialog_scene();
    let id = fixture.id(dialog).0;
    let before = fixture.placement(dialog).rect;
    let ok = |response: Response| assert!(matches!(response, Response::Ok { .. }), "{response:?}");
    ok(fixture
        .state
        .handle_request(Request::Action(scoot_ipc::Action::MoveFloating {
            id,
            x: 10,
            y: 12,
        })));
    fixture.settle();
    let rect = fixture.snapshot(dialog).rect;
    assert_eq!(
        (rect.x, rect.y, rect.width, rect.height),
        (10, 12, before.w, before.h)
    );
    ok(fixture
        .state
        .handle_request(Request::Action(scoot_ipc::Action::ResizeFloating {
            id,
            width: 80,
            height: 50,
        })));
    fixture.settle();
    let asked = fixture.last_configure(dialog);
    assert_eq!((asked.width, asked.height), (80, 50));
    assert!(
        !asked.resizing,
        "a size by number is not an interactive resize"
    );
    fixture.done(Step::Draw { window: dialog });
    let rect = fixture.snapshot(dialog).rect;
    assert_eq!((rect.x, rect.y, rect.width, rect.height), (10, 12, 80, 50));
    // Clamped: far off the output lands inside it.
    ok(fixture
        .state
        .handle_request(Request::Action(scoot_ipc::Action::MoveFloating {
            id,
            x: i32::MAX,
            y: i32::MIN,
        })));
    fixture.settle();
    let rect = fixture.snapshot(dialog).rect;
    assert_eq!((rect.x, rect.y), (CANVAS - 80, 0));
    // A tiled window, or no window, is left alone (and answered `ok`, like
    // every by-id action).
    let column = fixture.placement(0).rect;
    ok(fixture
        .state
        .handle_request(Request::Action(scoot_ipc::Action::MoveFloating {
            id: fixture.id(0).0,
            x: 0,
            y: 0,
        })));
    ok(fixture
        .state
        .handle_request(Request::Action(scoot_ipc::Action::ResizeFloating {
            id: 999,
            width: 10,
            height: 10,
        })));
    fixture.settle();
    assert_eq!(fixture.placement(0).rect, column);
}

/// IPC `resize-floating` clamps to the limits the window has *now*: they
/// arrive at its first commit, after the core took its copy.
#[test]
fn ipc_resize_respects_the_window_s_own_limits() {
    let mut fixture = Fixture::new();
    fixture.map(Spec::tiled());
    let dialog = fixture.map(Spec {
        min: Some((50, 30)),
        max: Some((100, 60)),
        ..Spec::dialog_of(0)
    });
    let id = fixture.id(dialog).0;
    for ((width, height), expected) in [((500, 500), (100, 60)), ((5, 5), (50, 30))] {
        let response =
            fixture
                .state
                .handle_request(Request::Action(scoot_ipc::Action::ResizeFloating {
                    id,
                    width,
                    height,
                }));
        assert!(matches!(response, Response::Ok { .. }), "{response:?}");
        fixture.settle();
        let asked = fixture.last_configure(dialog);
        assert_eq!((asked.width, asked.height), expected);
    }
}

/// PR #242's carried note, on screen: a floating parent clicked above its
/// dialog keeps the dialog drawn over it.
#[test]
fn a_floating_parent_clicked_keeps_its_dialog_drawn_above_it() {
    let mut fixture = Fixture::new();
    fixture.map(Spec::tiled());
    fixture.act(Action::ToggleFloating);
    assert!(fixture.floating(0));
    let dialog = fixture.map(Spec::dialog_of(0));
    let (dx, dy) = centre(fixture.placement(dialog).rect);
    assert!(
        fixture
            .placement(0)
            .rect
            .contains(scoot_core::Point::new(dx, dy)),
        "the dialog sits over its parent"
    );
    // A click on the parent, away from the dialog: the parent is focused
    // (and on top of the stack), its dialog still drawn over it.
    let parent = fixture.placement(0).rect;
    fixture.click(f64::from(parent.x + 4), f64::from(parent.y + 4));
    assert_eq!(fixture.state.world.focused_window(), Some(fixture.id(0)));
    let pixels = fixture.render();
    assert_eq!(pixel(&pixels, dx, dy), DIALOG_BGRA);
}
