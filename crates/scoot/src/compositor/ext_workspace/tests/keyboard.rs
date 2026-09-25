//! A workspace switch over `ext-workspace-v1` spends a clicked `on_demand`
//! layer surface's keyboard.
//!
//! The suspected bug this pins: `commit_workspace_requests` ended in
//! `act(FocusWorkspaceIndex)` without first clearing `State::clicked_layer`,
//! so a workspace-switching panel that stayed mapped kept every keystroke
//! after handing window focus away -- while `scoot msg windows` named the
//! new window. The same shape PR #50 fixed in
//! `foreign_toplevel_management.rs` and PR #53 fixed in `activation.rs` and
//! `ipc.rs`. Unlike those paths this one was never proven: it is possible
//! (though unlikely) that workspace switching re-derives focus without
//! consulting `clicked_layer`, in which case these tests pass either way and
//! the entry closes as not-a-bug.
//!
//! The weak shape of this test (hand-setting `clicked_layer` with no real
//! layer surface) passes either way and proves nothing; like
//! `foreign_toplevel_management`'s `taskbar_holding_the_keyboard`, this runs
//! a real mapped `on_demand` taskbar, a real pointer click through the input
//! path (the only writer of `clicked_layer`), then a real workspace
//! `activate` + `commit` over the wire, asserting on the seat's actual
//! keyboard focus surface.
//!
//! Three tests, one per question the entry asks:
//!
//! - switching to another workspace moves the keyboard with window focus
//!   (fails unfixed: `layer_keyboard_focus` re-derives the still-mapped
//!   taskbar);
//! - re-activating the already-active workspace spends the click too -- the
//!   same gesture over IPC (`FocusWorkspaceIndex`, PR #53) spends it
//!   unconditionally, so this path must agree rather than differ by
//!   transport (fails unfixed: the early return touches nothing at all);
//! - a switch refused behind the session lock spends neither focus nor the
//!   click, and unlocking hands the keyboard back to the taskbar -- moving
//!   the clear above the lock check fails it while leaving the other two
//!   green.

use super::*;
use scoot_core::WindowId;
use smithay::desktop::{LayerSurface, Window};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;

/// A left click at a point, press and release, the way a user makes one --
/// the same helper `foreign_toplevel_management/tests` uses, for the same
/// reason: `State::clicked_layer` is only ever written by a click that
/// really went through the pointer.
fn click(fixture: &mut Fixture, x: f64, y: f64) {
    fixture.state.pointer_move(x, y);
    fixture
        .state
        .pointer_button(scoot_ipc::PointerButton::Left, true);
    fixture
        .state
        .pointer_button(scoot_ipc::PointerButton::Left, false);
    fixture.settle();
}

/// The `wl_surface` the seat's keyboard focus is actually on, which is the
/// only unambiguous answer to "where do keystrokes go".
fn keyboard_surface(fixture: &Fixture) -> Option<WlSurface> {
    fixture
        .state
        .seat
        .get_keyboard()
        .expect("a keyboard")
        .current_focus()
        .map(WlSurface::from)
}

/// The `wl_surface` of the `id`-th window's toplevel.
fn window_surface(fixture: &Fixture, id: WindowId) -> WlSurface {
    fixture
        .state
        .windows
        .get(&id)
        .and_then(Window::toplevel)
        .expect("a live window")
        .wl_surface()
        .clone()
}

/// The `wl_surface` of the clicked layer surface itself, so the precondition
/// can assert the keyboard is on exactly that surface rather than merely off
/// the window.
fn clicked_surface(fixture: &Fixture) -> WlSurface {
    fixture
        .state
        .clicked_layer
        .as_ref()
        .expect("a clicked layer surface")
        .wl_surface()
        .clone()
}

/// Two windows on two workspaces, a taskbar mapped by a second client and
/// clicked so it holds the keyboard: window 1 on workspace 1, window 2 on
/// the active workspace 2, the keyboard on the taskbar, window focus
/// unmoved. Asserts the whole arrangement really happened, so a later
/// failure can only be about the switch, not the setup.
fn drive_with_taskbar() -> Fixture {
    let mut fixture = Fixture::new();
    let taskbar_client = fixture.spawn(run_client);
    fixture.run(Step::BindOutput);
    fixture.run(Step::BindManager);
    fixture.take_log();
    fixture.run(Step::MapWindow); // window 1 on workspace 1
    fixture.take_log();
    // Onto the trailing empty workspace before mapping the second window,
    // so the two windows end up on different workspaces.
    fixture.act(Action::FocusWorkspace(Vertical::Down));
    fixture.run(Step::MapWindow); // window 2 on workspace 2, focused
    fixture.take_log();
    assert_eq!(
        fixture.workspaces(),
        (3, 1),
        "the setup should leave three workspaces with the second active"
    );
    assert_eq!(
        fixture.state.focus,
        Some(WindowId(2)),
        "the second window should have focus before anything is clicked"
    );

    let Ack::TaskbarMapped = fixture.run_on(taskbar_client, Step::MapTaskbar) else {
        panic!("the taskbar client never mapped its layer surface");
    };
    // The ack only proves the *client* finished its own round trip, not that
    // the compositor has dispatched the buffer commit yet -- without the
    // settle inside `run_on` a click can run against a layer map that does
    // not have the taskbar in it yet and land on whatever is behind it
    // instead.
    click(&mut fixture, TASKBAR_POINT.0, TASKBAR_POINT.1);
    assert!(
        fixture.state.clicked_layer.is_some(),
        "the click never reached the taskbar -- check TASKBAR_POINT against the layout"
    );
    assert!(
        fixture.state.keyboard_on_layer,
        "an on_demand layer surface should hold the keyboard once clicked"
    );
    assert_eq!(
        keyboard_surface(&fixture),
        Some(clicked_surface(&fixture)),
        "the keyboard is not on the taskbar the click landed on"
    );
    assert_eq!(
        fixture.state.focus,
        Some(WindowId(2)),
        "clicking a bar must not move *window* focus"
    );
    fixture
}

#[test]
fn activating_another_workspace_takes_the_keyboard_back_from_a_clicked_taskbar() {
    // A workspace-switching panel that stays mapped: clicked (it takes the
    // keyboard), then it hands focus over with `ext-workspace-v1`. Window
    // focus must move *and* the keyboard must follow it -- without a
    // `clicked_layer = None` before the `act`, `layer_keyboard_focus`
    // re-derives the still-mapped taskbar and only the first half happens,
    // which is worse than either end of it: the user cannot see where their
    // typing is going.
    let mut fixture = drive_with_taskbar();

    fixture.run(Step::Activate(0));
    fixture.run(Step::Commit(0));

    assert_eq!(
        fixture.state.focus,
        Some(WindowId(1)),
        "the workspace switch did not move window focus"
    );
    assert_eq!(
        keyboard_surface(&fixture),
        Some(window_surface(&fixture, WindowId(1))),
        "switching workspace left the keyboard on the taskbar"
    );
    assert!(!fixture.state.keyboard_on_layer);
    assert!(
        fixture.state.clicked_layer.is_none(),
        "the taskbar's click was not spent"
    );
}

#[test]
fn activating_the_already_active_workspace_spends_the_click_too() {
    // The early-return path: the client asked for the workspace it is
    // already on, so there is no layout work to do -- but the gesture
    // happened regardless, and the same request over IPC
    // (`FocusWorkspaceIndex`, PR #53) spends the click unconditionally. This
    // path must agree with that one rather than differ by transport, the way
    // `wlr_toplevel_activate`'s already-focused fast path still spends the
    // click and runs the keyboard half.
    let mut fixture = drive_with_taskbar();

    fixture.run(Step::Activate(1));
    fixture.run(Step::Commit(0));

    assert_eq!(
        fixture.workspaces(),
        (3, 1),
        "re-activating the current workspace should change nothing"
    );
    assert_eq!(
        fixture.state.focus,
        Some(WindowId(2)),
        "re-activating the current workspace moved window focus"
    );
    assert_eq!(
        keyboard_surface(&fixture),
        Some(window_surface(&fixture, WindowId(2))),
        "re-activating the current workspace left the keyboard on the taskbar"
    );
    assert!(
        fixture.state.clicked_layer.is_none(),
        "the taskbar's click was not spent"
    );
}

#[test]
fn a_workspace_activate_while_locked_spends_neither_focus_nor_the_taskbars_click() {
    // The lock gate's ordering, pinned: a refused switch must disturb
    // nothing, because what follows a honored one spends the panel's click
    // -- and the session has to come back as the user left it. Moving the
    // clear above the `is_locked()` check keeps the other two tests green
    // while silently spending the taskbar's click here: on unlock the
    // keyboard would be on the window instead of the taskbar.
    let mut fixture = drive_with_taskbar();
    let taskbar: LayerSurface = fixture
        .state
        .clicked_layer
        .clone()
        .expect("the taskbar holds the click");

    let Ack::Locked = fixture.run(Step::LockSession) else {
        panic!("the client never took the session lock");
    };
    fixture.take_log();
    assert!(fixture.state.session_lock.is_locked());
    // The precondition this test is named for: locking itself must not move
    // window focus, or the refusal below would exercise nothing.
    assert_eq!(
        fixture.state.focus,
        Some(WindowId(2)),
        "locking the session moved window focus"
    );

    fixture.run(Step::Activate(0));
    fixture.run(Step::Commit(0));

    assert!(
        fixture.state.session_lock.is_locked(),
        "the session came unlocked on its own mid-test"
    );
    assert_eq!(
        fixture.state.focus,
        Some(WindowId(2)),
        "a locked session's focus was moved by a workspace activate"
    );
    assert_eq!(
        fixture.state.clicked_layer.as_ref(),
        Some(&taskbar),
        "a refused activate spent the taskbar's click anyway"
    );
    assert_ne!(
        keyboard_surface(&fixture),
        Some(window_surface(&fixture, WindowId(1))),
        "a locked session gave the keyboard to a window on a client's request"
    );
    // Deliberately not asserting the keyboard is still on the taskbar
    // *while* locked: with no lock surface mapped the seat has no focus at
    // all, and the lock guarantees no window does -- which is the half that
    // matters here. The click's survival is what brings the keyboard back
    // below.

    // Unlock the way a lock screen does after auth, and the session must
    // come back as the user left it: same window focus, same click, and the
    // keyboard back on the taskbar it was clicked onto.
    let Ack::Unlocked = fixture.run(Step::Unlock) else {
        panic!("the client never released the session lock");
    };
    assert!(!fixture.state.session_lock.is_locked());
    assert_eq!(
        fixture.state.focus,
        Some(WindowId(2)),
        "unlocking moved window focus"
    );
    assert_eq!(
        fixture.state.clicked_layer.as_ref(),
        Some(&taskbar),
        "the taskbar's click did not survive the lock"
    );
    assert_eq!(
        keyboard_surface(&fixture),
        Some(clicked_surface(&fixture)),
        "unlocking did not hand the keyboard back to the clicked taskbar"
    );
}
