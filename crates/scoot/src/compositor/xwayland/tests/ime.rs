//! Phase 4: input methods. X clients compose text through XIM, which scoot
//! does not provide (XWayland binds no `text-input-v3`), so the question
//! here is only precedence: an input method's keyboard grab
//! (`input-method-v2`, fcitx5 or an on-screen keyboard) pre-empts an X
//! window's keyboard focus exactly as it does a Wayland window's -- the
//! keys go to the input method, and none reach the X window.

use smithay::backend::input::KeyState;
use smithay::input::keyboard::Keycode;
use x11rb::protocol::Event as XEvent;

use super::live::{RED, live};
use super::peer::{Ack, ImeStep, Step};
use super::x11::Props;
use crate::compositor::keyboard_focus::KeyboardFocus;

/// evdev `KEY_A`, and the XKB keycode Smithay's seat takes for it.
const KEY_A_EVDEV: u32 = 30;
const KEY_A: u32 = KEY_A_EVDEV + 8;

fn type_a(live: &mut super::live::Live) {
    live.fixture
        .state
        .key(Keycode::new(KEY_A), KeyState::Pressed);
    live.fixture
        .state
        .key(Keycode::new(KEY_A), KeyState::Released);
    live.drain();
}

fn x_got_a(live: &super::live::Live, xid: u32) -> bool {
    live.x.drain().into_iter().any(|event| {
        matches!(event, XEvent::KeyPress(press) if press.event == xid && u32::from(press.detail) == KEY_A)
    })
}

/// With an X window focused, an input method taking the keyboard grab gets
/// every key; the X window gets none. Before the grab, the same key reaches
/// the X window -- so the test is about the grab, not a dead keyboard.
#[test]
fn an_input_method_grab_pre_empts_an_x_windows_keyboard() {
    let Some(mut live) = live("an_input_method_grab_pre_empts_an_x_windows_keyboard") else {
        return;
    };
    let xid = live.x.map(&Props::new(RED));
    let id = live.managed(xid);
    live.drain();
    assert_eq!(live.fixture.state.focus, Some(id));
    assert!(matches!(live.keyboard(), Some(KeyboardFocus::X11 { .. })));
    live.x.drain();
    type_a(&mut live);
    assert!(
        x_got_a(&live, xid),
        "without a grab the key should reach the X window"
    );

    assert!(matches!(
        live.fixture.run(Step::Ime(ImeStep::Grab)),
        Ack::Done
    ));
    live.x.drain();
    type_a(&mut live);
    let Ack::Keys(keys) = live.fixture.run(Step::Ime(ImeStep::Keys)) else {
        panic!("expected keys");
    };
    assert_eq!(
        keys,
        vec![KEY_A_EVDEV],
        "the input method's grab did not get the key"
    );
    assert!(
        !x_got_a(&live, xid),
        "a key reached the X window through an input method's keyboard grab"
    );
    // Still focused: the grab pre-empts where keys go, not which window
    // holds focus.
    assert_eq!(live.fixture.state.focus, Some(id));
}
