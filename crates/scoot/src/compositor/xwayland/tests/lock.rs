//! The session lock against X windows: nothing of an X client is drawn
//! behind the lock, and nothing of the user's input reaches one -- neither
//! a managed window that held the keyboard nor an override-redirect menu
//! left open. Each assertion here was first run against a build with the
//! corresponding lock branch removed (see the PR's fail-first record).

use smithay::backend::input::KeyState;
use smithay::input::keyboard::Keycode;
use x11rb::protocol::Event as XEvent;

use super::live::{BLUE, BLUE_BGRA, RED, RED_BGRA, live};
use super::peer::{Ack, Step};
use super::x11::{Props, eventually};
use crate::compositor::keyboard_focus::KeyboardFocus;

const KEY_A: u32 = 30 + 8;

#[test]
fn the_lock_blanks_x_windows_and_refuses_them_input() {
    let Some(mut live) = live("the_lock_blanks_x_windows_and_refuses_them_input") else {
        return;
    };
    let xid = live.x.map(&Props::new(RED));
    let id = live.managed(xid);
    let mut menu = Props::new(BLUE);
    menu.rect = (20, 20, 50, 40);
    menu.override_redirect = true;
    let menu = live.x.map(&menu);
    eventually(&mut live.fixture, "the menu drawn", |fixture| {
        fixture
            .state
            .x11_unmanaged
            .iter()
            .any(|known| known.window_id() == menu && known.wl_surface().is_some())
    });
    let rect = live.placement(id).rect;
    let window_point = (rect.x + rect.w - 10, rect.y + rect.h - 10);
    // Both visible, and the X window holds the keyboard, before the lock.
    assert_eq!(live.pixel_at(window_point.0, window_point.1), RED_BGRA);
    assert_eq!(live.pixel_at(45, 40), BLUE_BGRA);
    live.drain();
    assert_eq!(live.x.input_focus(), xid);

    assert!(matches!(live.fixture.run(Step::Lock), Ack::Done));
    eventually(&mut live.fixture, "the session locking", |fixture| {
        fixture.state.session_lock.is_locked()
    });
    live.drain();

    // Blanked: neither the window nor the menu is in the frame.
    let pixels = live.fixture.render();
    assert!(
        !crate::compositor::test_support::contains(&pixels, RED_BGRA),
        "the X window is drawn behind the lock"
    );
    assert!(
        !crate::compositor::test_support::contains(&pixels, BLUE_BGRA),
        "the override-redirect menu is drawn behind the lock"
    );

    // The keyboard: off the X window, Wayland-side and X-side.
    assert!(
        !matches!(live.keyboard(), Some(KeyboardFocus::X11 { .. })),
        "the X window kept the keyboard through the lock"
    );
    assert_ne!(
        live.x.input_focus(),
        xid,
        "the X server still sends keys to the X window under the lock"
    );
    live.x.drain();
    live.fixture
        .state
        .key(Keycode::new(KEY_A), KeyState::Pressed);
    live.fixture
        .state
        .key(Keycode::new(KEY_A), KeyState::Released);
    live.drain();
    let leaked = live
        .x
        .drain()
        .into_iter()
        .any(|event| matches!(event, XEvent::KeyPress(_)));
    assert!(
        !leaked,
        "a key typed at the lock screen reached an X client"
    );

    // The pointer: over the window and over the menu, it finds neither.
    for (x, y) in [window_point, (45, 40)] {
        assert!(
            live.fixture
                .state
                .surface_under((f64::from(x), f64::from(y)).into())
                .is_none(),
            "the pointer finds an X surface at ({x}, {y}) under the lock"
        );
        live.fixture.state.pointer_move(f64::from(x), f64::from(y));
        assert!(
            live.fixture
                .state
                .seat
                .get_pointer()
                .and_then(|pointer| pointer.current_focus())
                .is_none(),
            "the pointer entered an X surface at ({x}, {y}) under the lock"
        );
    }
    // And the window keeps its place for the unlock.
    assert_eq!(super::live::id_of_xid(&live.fixture.state, xid), Some(id));
}

/// An override-redirect window mapped *during* the lock -- a tooltip or an
/// X client's fake prompt -- is neither drawn nor pointed at.
#[test]
fn an_override_redirect_window_mapped_during_the_lock_is_neither_drawn_nor_hit() {
    let Some(mut live) =
        live("an_override_redirect_window_mapped_during_the_lock_is_neither_drawn_nor_hit")
    else {
        return;
    };
    assert!(matches!(live.fixture.run(Step::Lock), Ack::Done));
    eventually(&mut live.fixture, "the session locking", |fixture| {
        fixture.state.session_lock.is_locked()
    });
    let mut props = Props::new(BLUE);
    props.rect = (20, 20, 50, 40);
    props.override_redirect = true;
    let overlay = live.x.map(&props);
    eventually(&mut live.fixture, "the overlay mapped", |fixture| {
        fixture
            .state
            .x11_unmanaged
            .iter()
            .any(|known| known.window_id() == overlay && known.wl_surface().is_some())
    });
    live.drain();
    let pixels = live.fixture.render();
    assert!(
        !crate::compositor::test_support::contains(&pixels, BLUE_BGRA),
        "an override-redirect window mapped under the lock is drawn"
    );
    assert!(
        live.fixture
            .state
            .surface_under((45.0, 40.0).into())
            .is_none(),
        "the pointer finds an override-redirect window mapped under the lock"
    );
    live.fixture.state.pointer_move(45.0, 40.0);
    assert!(
        live.fixture
            .state
            .seat
            .get_pointer()
            .and_then(|pointer| pointer.current_focus())
            .is_none(),
        "the pointer entered an override-redirect window mapped under the lock"
    );
}
