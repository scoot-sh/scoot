//! Where input goes while the session is locked -- and where it must not.
//!
//! Every focus assertion here is made from what the *client* was told
//! (`wl_keyboard.enter`, `wl_pointer.button`, `xdg_toplevel.close`), never
//! from a field inside the compositor: "the window did not receive that
//! keystroke" is a claim about the wire.

use super::*;

/// Keyboard focus moves to the lock surface and every keystroke goes there,
/// not to the window that had focus a moment earlier.
#[test]
fn the_keyboard_reaches_only_the_lock_surface() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    assert_eq!(
        fixture.report().keyboard_focus,
        Some(Which::Window(0)),
        "the window should have the keyboard before the lock"
    );

    fixture.run(Step::Lock);
    assert_eq!(
        fixture.report().keyboard_focus,
        None,
        "the window must lose the keyboard the moment the session locks, \
         even before a lock surface exists"
    );

    fixture.run(Step::map_lock_surface(0));
    let before = fixture.report();
    assert_eq!(before.keyboard_focus, Some(Which::Lock(0)));

    fixture.state.type_text("hello").expect("typed text");
    fixture.settle();
    let after = fixture.report();
    assert_eq!(
        after.keyboard_focus,
        Some(Which::Lock(0)),
        "focus must not have moved"
    );
    assert!(
        after.keys > before.keys,
        "the lock surface should have received the keystrokes"
    );
}

/// Pointer focus has to be moved *at* the lock, not merely hit-tested
/// afterwards: `wl_pointer.button` goes to whatever the pointer last entered,
/// so a click after locking would otherwise land in the window underneath.
#[test]
fn a_click_while_locked_does_not_reach_the_window() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    // Over the window's own buffer, which starts at the placement's top-left
    // corner (gap 12, ring 3).
    fixture.state.pointer_move(30.0, 30.0);
    fixture.settle();
    assert_eq!(
        fixture.report().pointer_focus,
        Some(Which::Window(0)),
        "the pointer should be over the window before the lock"
    );

    fixture.run(Step::Lock);
    let locked = fixture.report();
    assert_eq!(
        locked.pointer_focus, None,
        "the pointer must leave the window when the session locks"
    );

    fixture.state.pointer_button(PointerButton::Left, true);
    fixture.state.pointer_button(PointerButton::Left, false);
    fixture.settle();
    let clicked = fixture.report();
    assert_eq!(
        clicked.buttons, locked.buttons,
        "no button event may reach a client while nothing but the backdrop is up"
    );
    assert_eq!(clicked.pointer_focus, None);

    // ...and once a lock surface is up, the same click reaches *it*.
    fixture.run(Step::map_lock_surface(0));
    fixture.state.pointer_move(30.0, 30.0);
    fixture.state.pointer_button(PointerButton::Left, true);
    fixture.state.pointer_button(PointerButton::Left, false);
    fixture.settle();
    let on_lock = fixture.report();
    assert_eq!(on_lock.pointer_focus, Some(Which::Lock(0)));
    assert_eq!(
        on_lock.buttons,
        clicked.buttons + 2,
        "the click should reach the lock surface"
    );
}

/// A keybinding that runs an `Action` must not fire while locked: `Super+Q`
/// is `CloseFocused` by default, and a window being told to close from behind
/// a lock screen is exactly the bypass this gate exists for. The keystroke is
/// forwarded to the lock client instead of being swallowed.
#[test]
fn an_action_keybinding_does_not_fire_while_locked() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::Lock);
    fixture.run(Step::map_lock_surface(0));
    let before = fixture.report();

    let combo = KeyCombo {
        modifiers: vec![Modifier::Super],
        key: "q".into(),
    };
    fixture.state.press(&combo).expect("a pressed combo");
    fixture.settle();
    let after = fixture.report();
    assert_eq!(
        after.closes, before.closes,
        "the window must not be told to close from behind a lock screen"
    );
    assert!(
        after.keys > before.keys,
        "the keystroke should have been forwarded to the lock client"
    );

    // The same combo works normally once unlocked, so this is a gate rather
    // than a broken binding.
    fixture.run(Step::Unlock { lock: 0 });
    fixture.state.press(&combo).expect("a pressed combo");
    fixture.settle();
    assert_eq!(
        fixture.report().closes,
        before.closes + 1,
        "the binding should work again after unlocking"
    );
}

/// Every IPC success carries the session-lock state it was built under,
/// so an agent learns an unlock landed from the very next reply: `true`
/// while locked (injected input is still served, reaching only the lock
/// screen), `false` once unlocked.
#[test]
fn ok_replies_carry_the_locked_state() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::Lock);

    let response = fixture
        .state
        .handle_request(Request::Type { text: "x".into() });
    assert!(
        matches!(response, Response::Ok { locked: true }),
        "input served while locked should say so, got {response:?}"
    );

    fixture.run(Step::Unlock { lock: 0 });
    let response = fixture
        .state
        .handle_request(Request::Type { text: "x".into() });
    assert!(
        matches!(response, Response::Ok { locked: false }),
        "input served unlocked should say so, got {response:?}"
    );
}

/// An IPC `action` bypasses input entirely, so it is refused outright while
/// locked -- and works again afterwards.
#[test]
fn ipc_actions_are_refused_while_locked() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::Lock);

    let response = fixture
        .state
        .handle_request(Request::Action(scoot_ipc::Action::CloseFocused));
    assert!(
        matches!(response, Response::Error { .. }),
        "an IPC action must be refused while locked, got {response:?}"
    );
    fixture.settle();
    assert_eq!(
        fixture.report().closes,
        0,
        "the refused action must not have reached the window"
    );

    fixture.run(Step::Unlock { lock: 0 });
    let response = fixture
        .state
        .handle_request(Request::Action(scoot_ipc::Action::CloseFocused));
    assert!(
        matches!(response, Response::Ok { locked: false }),
        "the same action should work once unlocked, got {response:?}"
    );
    fixture.settle();
    assert_eq!(fixture.report().closes, 1);
}

/// The backstop behind both of the above, and the one thing standing between
/// an `ext-workspace-v1` client and rearranging the session from behind the
/// lock screen: `State::act` itself refuses while locked, so a caller added
/// later is safe without having to remember.
#[test]
fn the_action_path_itself_is_closed_while_locked() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::Lock);

    fixture.state.act(scoot_core::Action::CloseFocused);
    fixture.settle();
    assert_eq!(
        fixture.report().closes,
        0,
        "an action must not reach a window from behind a lock screen, \
         whichever caller asked for it"
    );

    fixture.run(Step::Unlock { lock: 0 });
    fixture.state.act(scoot_core::Action::CloseFocused);
    fixture.settle();
    assert_eq!(fixture.report().closes, 1);
}

/// The cross-output actions under lock (milestone 19, phase F): neither the
/// IPC gate nor the `act` backstop may let them through, and neither may
/// wedge the session -- the arrangement before and after is identical.
///
/// Output 1 is named deliberately: it is a *valid* output here, so an
/// unchanged arrangement proves the refusal did it, not a same-output or
/// unknown-output no-op. The response shape is the other discriminator: a
/// refusal is an `Error`, never an `Ok`.
#[test]
fn the_cross_output_actions_are_refused_while_locked() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::Lock);
    let before = fixture.state.world.arrange();

    for action in [
        scoot_ipc::Action::MoveFocusedWindowToOutput { output: 1 },
        scoot_ipc::Action::FocusOutput { output: 1 },
    ] {
        let response = fixture.state.handle_request(Request::Action(action));
        assert!(
            matches!(response, Response::Error { .. }),
            "a cross-output action must be refused while locked, got {response:?}"
        );
    }
    // And through the backstop directly, for the caller-added-later shape.
    fixture
        .state
        .act(scoot_core::Action::MoveFocusedWindowToOutput(
            scoot_core::OutputId(1),
        ));
    fixture
        .state
        .act(scoot_core::Action::FocusOutput(scoot_core::OutputId(1)));
    fixture.settle();
    assert_eq!(
        fixture.state.world.arrange(),
        before,
        "a refused cross-output action moved something while locked"
    );
    assert_eq!(
        fixture.report().closes,
        0,
        "a refused cross-output action reached a window"
    );
}
