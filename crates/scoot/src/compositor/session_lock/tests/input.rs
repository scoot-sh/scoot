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
/// unknown-output no-op. Index 0 is likewise the already-active workspace,
/// for the same reason. The response shape is the other discriminator: a
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
        scoot_ipc::Action::FocusOutputWorkspaceIndex {
            output: 1,
            index: 0,
        },
        scoot_ipc::Action::FocusOutputDirection {
            direction: scoot_ipc::Horizontal::Left,
        },
        scoot_ipc::Action::MoveWindowToOutputDirection {
            direction: scoot_ipc::Horizontal::Right,
        },
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
    fixture
        .state
        .act(scoot_core::Action::FocusOutputWorkspaceIndex {
            output: scoot_core::OutputId(1),
            index: 0,
        });
    fixture.state.act(scoot_core::Action::FocusOutputDirection(
        scoot_core::Horizontal::Left,
    ));
    fixture
        .state
        .act(scoot_core::Action::MoveWindowToOutputDirection(
            scoot_core::Horizontal::Right,
        ));
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

/// A `spawn` bind flagged `allow_when_locked` fires while locked: the
/// keystroke is intercepted (the lock client never sees it) and the
/// config-pinned command runs. Volume, brightness and media keys -- the
/// desktop profile opts exactly those in.
#[test]
fn an_allowed_spawn_bind_fires_while_locked() {
    use smithay::backend::input::KeyState;

    use crate::compositor::keybindings::{BindFlags, Bound, Modifiers};

    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.state.keybindings.insert(
        Modifiers::default(),
        crate::compositor::input::keysym_named("XF86AudioRaiseVolume").unwrap(),
        Bound::Action(scoot_core::Action::Spawn(vec!["true".into()])),
        BindFlags {
            repeat: false,
            allow_when_locked: true,
        },
    );
    fixture.run(Step::Lock);
    fixture.run(Step::map_lock_surface(0));
    let before = fixture.report();
    let spawns = fixture.state.spawned_children.len();

    let code = fixture
        .state
        .keycode_for_combo(&KeyCombo {
            modifiers: vec![],
            key: "XF86AudioRaiseVolume".into(),
        })
        .expect("the volume key resolves on the test keymap");
    fixture.state.key(code, KeyState::Pressed);
    assert_eq!(
        fixture.state.spawned_children.len(),
        spawns + 1,
        "the allowed spawn fired while locked"
    );
    fixture.state.key(code, KeyState::Released);
    fixture.settle();

    let after = fixture.report();
    assert_eq!(
        after.keys, before.keys,
        "the allowed bind's keystroke must be intercepted, not forwarded to the lock client"
    );
    assert_eq!(
        after.keyboard_focus,
        Some(Which::Lock(0)),
        "focus must not have moved"
    );
}

/// A `spawn` bind *without* the flag stays refused while locked: the
/// keystroke goes to the lock client like any other, and nothing runs. A
/// terminal from behind the lock screen would be a complete bypass, so the
/// default for every bind is refusal.
#[test]
fn a_spawn_bind_without_the_flag_is_forwarded_while_locked() {
    use smithay::backend::input::KeyState;

    use crate::compositor::keybindings::{BindFlags, Bound, Modifiers};

    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.state.keybindings.insert(
        Modifiers::default(),
        crate::compositor::input::keysym_named("XF86AudioLowerVolume").unwrap(),
        Bound::Action(scoot_core::Action::Spawn(vec!["true".into()])),
        BindFlags::default(),
    );
    fixture.run(Step::Lock);
    fixture.run(Step::map_lock_surface(0));
    let before = fixture.report();
    let spawns = fixture.state.spawned_children.len();

    let code = fixture
        .state
        .keycode_for_combo(&KeyCombo {
            modifiers: vec![],
            key: "XF86AudioLowerVolume".into(),
        })
        .expect("the volume key resolves on the test keymap");
    fixture.state.key(code, KeyState::Pressed);
    fixture.state.key(code, KeyState::Released);
    assert_eq!(
        fixture.state.spawned_children.len(),
        spawns,
        "the unflagged spawn must not run while locked"
    );
    fixture.settle();

    let after = fixture.report();
    assert!(
        after.keys > before.keys,
        "the refused bind's keystroke should have been forwarded to the lock client"
    );
    assert_eq!(
        after.keyboard_focus,
        Some(Which::Lock(0)),
        "focus must not have moved"
    );
}

/// The flag on anything but a `spawn` still refuses while locked: layout,
/// focus, close and quit keep today's refusal even when a table names the
/// flag on them (`config.rs` clears it at load with a warning; `act_bind`
/// re-checks the shape, and this pins the key path end to end).
#[test]
fn an_allowed_flag_on_a_non_spawn_action_still_refuses_while_locked() {
    use smithay::backend::input::KeyState;

    use crate::compositor::keybindings::{BindFlags, Bound, Modifiers};

    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.state.keybindings.insert(
        Modifiers::default(),
        crate::compositor::input::keysym_named("XF86AudioPlay").unwrap(),
        Bound::Action(scoot_core::Action::CloseFocused),
        BindFlags {
            repeat: false,
            allow_when_locked: true,
        },
    );
    fixture.run(Step::Lock);
    fixture.run(Step::map_lock_surface(0));
    let before = fixture.report();

    let code = fixture
        .state
        .keycode_for_combo(&KeyCombo {
            modifiers: vec![],
            key: "XF86AudioPlay".into(),
        })
        .expect("the media key resolves on the test keymap");
    fixture.state.key(code, KeyState::Pressed);
    fixture.state.key(code, KeyState::Released);
    fixture.settle();

    let after = fixture.report();
    assert_eq!(
        after.closes, before.closes,
        "a non-spawn must not act from behind a lock screen, flag or not"
    );
    assert!(
        after.keys > before.keys,
        "the refused bind's keystroke should have been forwarded to the lock client"
    );
}

/// An allowed `repeat` bind keeps stepping while locked: hold volume on
/// the lock screen and it keeps stepping until release.
#[test]
fn an_allowed_repeat_bind_keeps_stepping_while_locked() {
    use smithay::backend::input::KeyState;

    use crate::compositor::bind_repeat::repeat_delay;
    use crate::compositor::keybindings::{BindFlags, Bound, Modifiers};

    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.state.keybindings.insert(
        Modifiers::default(),
        crate::compositor::input::keysym_named("XF86AudioRaiseVolume").unwrap(),
        Bound::Action(scoot_core::Action::Spawn(vec!["true".into()])),
        BindFlags {
            repeat: true,
            allow_when_locked: true,
        },
    );
    fixture.run(Step::Lock);
    fixture.run(Step::map_lock_surface(0));
    let before = fixture.report();

    let code = fixture
        .state
        .keycode_for_combo(&KeyCombo {
            modifiers: vec![],
            key: "XF86AudioRaiseVolume".into(),
        })
        .expect("the volume key resolves on the test keymap");
    fixture.state.key(code, KeyState::Pressed);
    let spawns = fixture.state.spawned_children.len();
    fixture
        .state
        .note_bind_repeat_timeout(std::time::Instant::now() + repeat_delay() * 2);
    assert_eq!(
        fixture.state.spawned_children.len(),
        spawns + 1,
        "the allowed repeat re-fired while locked"
    );
    fixture.state.key(code, KeyState::Released);
    fixture.settle();

    let after = fixture.report();
    assert_eq!(
        after.keys, before.keys,
        "neither the press nor the re-fires may reach the lock client"
    );
}

/// Locking cancels an in-flight repeat: a volume key held across the lock
/// stops stepping the moment the session locks, and the timer after that
/// fires into nothing.
#[test]
fn locking_cancels_an_in_flight_repeat() {
    use smithay::backend::input::KeyState;

    use crate::compositor::bind_repeat::repeat_delay;
    use crate::compositor::keybindings::{BindFlags, Bound, Modifiers};

    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.state.keybindings.insert(
        Modifiers::default(),
        crate::compositor::input::keysym_named("XF86AudioRaiseVolume").unwrap(),
        Bound::Action(scoot_core::Action::Spawn(vec!["true".into()])),
        BindFlags {
            repeat: true,
            allow_when_locked: false,
        },
    );

    let code = fixture
        .state
        .keycode_for_combo(&KeyCombo {
            modifiers: vec![],
            key: "XF86AudioRaiseVolume".into(),
        })
        .expect("the volume key resolves on the test keymap");
    fixture.state.key(code, KeyState::Pressed);
    assert!(
        fixture.state.bind_repeat.is_some(),
        "the press armed a repeat"
    );

    fixture.run(Step::Lock);
    assert!(
        fixture.state.bind_repeat.is_none(),
        "the lock cancelled the in-flight repeat"
    );
    let spawns = fixture.state.spawned_children.len();
    fixture
        .state
        .note_bind_repeat_timeout(std::time::Instant::now() + repeat_delay() * 2);
    assert_eq!(
        fixture.state.spawned_children.len(),
        spawns,
        "nothing re-fires after the lock cancelled the repeat"
    );
    fixture.state.key(code, KeyState::Released);
}

/// An IPC `action` naming a spawn stays refused while locked, even though
/// the same command as a flagged bind would fire: the request carries an
/// arbitrary command from the requester, while a bind can only run its
/// config-pinned command. Allowing IPC spawns would turn "volume keys work
/// on the lock screen" into "anything with socket access runs anything
/// while locked".
#[test]
fn ipc_spawn_actions_are_refused_while_locked() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::Lock);
    let spawns = fixture.state.spawned_children.len();

    let response = fixture
        .state
        .handle_request(Request::Action(scoot_ipc::Action::Spawn {
            command: vec!["true".into()],
        }));
    assert!(
        matches!(response, Response::Error { .. }),
        "an IPC spawn must be refused while locked, got {response:?}"
    );
    fixture.settle();
    assert_eq!(
        fixture.state.spawned_children.len(),
        spawns,
        "the refused spawn must not have run"
    );
}
