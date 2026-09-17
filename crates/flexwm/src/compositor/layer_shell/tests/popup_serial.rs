//! `xdg_popup.grab` serial validation: which serials buy a grab, and which
//! are refused.
//!
//! Shares the real-client harness in the parent module (see its doc): every
//! grab here passes an explicit serial through [`GrabSource::Serial`], naming
//! exactly the serial under test -- a real key/button/enter serial read back
//! through [`Step::ReportSerials`], a serial consumed from `SERIAL_COUNTER`
//! that was never delivered anywhere, or another client's serial. Everything
//! asserts on what the *client* was told (`wl_keyboard.enter`,
//! `xdg_popup.popup_done`), because "who holds the keyboard" is a claim
//! about the wire.
//!
//! Window-parented popups only, deliberately: the gate reads the requesting
//! client and the serial, neither of which depends on what the popup hangs
//! off.

use super::*;
use smithay::utils::SERIAL_COUNTER;

/// Maps a popup grabbing with exactly `serial`, reporting whether the
/// compositor ever configured it.
fn grab_with(fixture: &mut Fixture, serial: u32) -> bool {
    let Ack::PopupConfigured(configured) = fixture.run(Step::MapPopup {
        parent: PopupParent::Window,
        color: POPUP_BGRA,
        grab: Some(GrabSource::Serial(serial)),
    }) else {
        panic!("the popup step should report whether a configure arrived");
    };
    configured
}

/// A serial nothing was ever sent with buys no grab: the popup is dismissed
/// rather than granted the keyboard.
///
/// The fabricated serial is consumed from the shared counter, which is what
/// makes it airtight: the counter only moves forward, so a consumed serial
/// was never delivered to any client, and every remembered entry was issued
/// earlier, so it matches none of them either.
#[test]
fn a_grab_with_a_serial_nothing_was_sent_with_is_refused() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    assert_eq!(
        fixture.keyboard().focused,
        Some(Focused::Window(0)),
        "a window should hold the keyboard before any popup exists"
    );

    let serial = u32::from(SERIAL_COUNTER.next_serial());
    assert!(
        !grab_with(&mut fixture, serial),
        "a grab with a fabricated serial should never be configured"
    );
    assert!(
        fixture.state.popup_grab.is_none(),
        "the compositor should be holding no grab it refused"
    );
    assert_eq!(
        fixture.popup_dones(),
        1,
        "the popup should be dismissed, not left up with no input"
    );
    assert_eq!(
        fixture.keyboard().focused,
        Some(Focused::Window(0)),
        "the keyboard should never have left the window"
    );
    fixture.disconnect_client();
}

/// A grab with a real key serial is accepted: the strict half of the gate,
/// and the shape every keyboard-opened menu takes.
#[test]
fn a_grab_with_a_real_key_serial_is_accepted() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.press_a_key();
    let key = fixture
        .serials()
        .key
        .expect("the client should have been sent a key");

    assert!(
        grab_with(&mut fixture, key),
        "a grab with the key serial just delivered should be configured"
    );
    assert!(
        fixture.state.popup_grab.is_some(),
        "the compositor should be holding the grab it granted"
    );
    assert_eq!(
        fixture.keyboard().focused,
        Some(Focused::Popup(0)),
        "the grabbing popup should hold the keyboard"
    );
    assert_eq!(fixture.popup_dones(), 0, "nothing has dismissed it yet");
    fixture.disconnect_client();
}

/// A grab minted from the button *release* is accepted: which of the two a
/// client spends is its own choice (a GTK button activates on release, not
/// press), and a gate that only knew presses would refuse the legitimate
/// half of that -- the same reason `interaction_serials` records both.
#[test]
fn a_grab_with_a_real_button_release_serial_is_accepted() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    // A click, press and release: the last button event the client saw is
    // the release, which is the serial under test.
    fixture.click(ON_WINDOW.0, ON_WINDOW.1);
    let report = fixture.serials();
    assert_eq!(
        report.key, None,
        "no key should have been involved -- this is the button half"
    );
    let release = report
        .button
        .expect("the client should have been sent a button event");

    assert!(
        grab_with(&mut fixture, release),
        "a grab with the button release serial just delivered should be configured"
    );
    assert_eq!(
        fixture.keyboard().focused,
        Some(Focused::Popup(0)),
        "the grabbing popup should hold the keyboard"
    );
    fixture.disconnect_client();
}

/// A grab with a pointer-`enter` serial is accepted: the Qt shape.
///
/// Qt passes its input device's last-seen serial to `grab`, and that device
/// updates on `pointer_enter` -- so a menu opened by hovering, which has had
/// no button or key event to draw a fresh serial from, names an enter. A
/// strict key-and-button gate refuses that legitimate menu, silently (the
/// protocol posts no error), which is the trap this test pins: it fails
/// against a strict gate and passes against the looser one.
#[test]
fn a_grab_with_a_pointer_enter_serial_is_accepted() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    // Pointer onto the window, and nothing else: no click, no key. The only
    // newer serial this client has seen is the enter itself.
    fixture.state.pointer_move(ON_WINDOW.0, ON_WINDOW.1);
    fixture.settle();
    let report = fixture.serials();
    assert_eq!(
        (report.key, report.button),
        (None, None),
        "no key or button should have been involved -- this is the enter half"
    );
    let entered = report
        .pointer_enter
        .expect("moving onto the window should deliver a pointer enter");

    assert!(
        grab_with(&mut fixture, entered),
        "a grab with the pointer-enter serial just delivered should be configured"
    );
    assert_eq!(
        fixture.keyboard().focused,
        Some(Focused::Popup(0)),
        "the grabbing popup should hold the keyboard"
    );
    fixture.disconnect_client();
}

/// A grab with a keyboard-`enter` serial is accepted: a freshly mapped and
/// focused window whose toolkit opens a menu before anything is typed or
/// clicked has no newer serial either.
#[test]
fn a_grab_with_a_keyboard_enter_serial_is_accepted() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    // No input at all: the focus the mapping itself earned is the newest
    // thing this client has seen.
    let report = fixture.serials();
    assert_eq!(
        (report.key, report.button, report.pointer_enter),
        (None, None, None),
        "no key, button or pointer enter should have been involved"
    );
    let entered = report
        .keyboard_enter
        .expect("mapping focused should deliver a keyboard enter");

    assert!(
        grab_with(&mut fixture, entered),
        "a grab with the keyboard-enter serial just delivered should be configured"
    );
    assert_eq!(
        fixture.keyboard().focused,
        Some(Focused::Popup(0)),
        "the grabbing popup should hold the keyboard"
    );
    fixture.disconnect_client();
}

/// Another client's serial buys nothing, however right the number is: the
/// check is on the (serial, recipient) pair, which is what makes it a check
/// on interaction rather than on arithmetic -- a client that was never
/// focused has nothing to guess *with*.
#[test]
fn another_clients_serial_buys_nothing() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.press_a_key();
    let key = fixture
        .serials()
        .key
        .expect("the first client should have been sent a key");

    let second = fixture.spawn(run_client);
    fixture.run_on(second, Step::MapWindow);
    let Ack::PopupConfigured(configured) = fixture.run_on(
        second,
        Step::MapPopup {
            parent: PopupParent::Window,
            color: POPUP_BGRA,
            grab: Some(GrabSource::Serial(key)),
        },
    ) else {
        panic!("the popup step should report whether a configure arrived");
    };
    assert!(
        !configured,
        "a grab with another client's serial should never be configured"
    );
    assert!(
        fixture.state.popup_grab.is_none(),
        "the compositor should be holding no grab it refused"
    );
    let Ack::PopupDone(dones) = fixture.run_on(second, Step::ReportPopupDone) else {
        panic!("the popup-done probe should report what the client saw");
    };
    assert_eq!(
        dones, 1,
        "the popup should be dismissed, not left up with no input"
    );
    fixture.disconnect(second);
    fixture.disconnect_client();
}

/// A nested grab with a stale serial is accepted while the parent menu is
/// open: the request continues the client's own live session, and the
/// keyboard was already its -- so the serial's age is beside the point.
///
/// Fail-first: with no session rule this is refused like any other aged-out
/// serial, which is what breaks a submenu hovered after reading its parent
/// for a minute (measured against a real Qt client, which reuses the opening
/// serial for the submenu grab).
#[test]
fn a_nested_grab_with_a_stale_serial_is_accepted_while_its_parent_is_open() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.press_a_key();
    let key = fixture
        .serials()
        .key
        .expect("the client should have been sent a key");
    assert!(
        grab_with(&mut fixture, key),
        "the parent menu should grab with its fresh serial"
    );

    // Age out everything, parent menu still open and holding the keyboard.
    fixture
        .state
        .interaction_serials
        .backdate(std::time::Duration::from_secs(3600));
    assert_eq!(
        fixture.keyboard().focused,
        Some(Focused::Popup(0)),
        "the parent menu should still hold the keyboard"
    );

    let Ack::PopupConfigured(configured) = fixture.run(Step::MapPopup {
        parent: PopupParent::Popup(0),
        color: POPUP_BGRA,
        grab: Some(GrabSource::Serial(key)),
    }) else {
        panic!("the popup step should report whether a configure arrived");
    };
    assert!(
        configured,
        "a nested grab on the live session should not need a fresh serial"
    );
    assert_eq!(
        fixture.keyboard().focused,
        Some(Focused::Popup(1)),
        "the submenu should take the keyboard from its parent menu"
    );
    assert_eq!(fixture.popup_dones(), 0, "neither has been dismissed");
    fixture.disconnect_client();
}

/// Replacing a menu reuses its serial within the grace: toolkits destroy the
/// old popup before grabbing the new one in the same input handler, so the
/// replacement never nests -- and without this a menubar hover-switch a
/// minute in reads as an attack (measured against a real Qt client).
#[test]
fn replacing_a_menu_reuses_its_serial_within_the_grace() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.press_a_key();
    let key = fixture
        .serials()
        .key
        .expect("the client should have been sent a key");
    assert!(
        grab_with(&mut fixture, key),
        "the first menu should grab with its fresh serial"
    );

    fixture.run(Step::DestroyPopup);
    assert!(
        fixture.state.popup_grab.is_none(),
        "the destroyed menu should have been reaped"
    );
    // Age out everything *after* the destroy, so only the ended session --
    // not a remembered serial -- can accept the replacement.
    fixture
        .state
        .interaction_serials
        .backdate(std::time::Duration::from_secs(3600));

    assert!(
        grab_with(&mut fixture, key),
        "a replacement menu within the grace should reuse the ended grab's serial"
    );
    // `Popup(1)`, not `Popup(0)`: the destroyed popup's surface keeps its
    // index (`popup_surfaces` is append-only, so a destroy cannot renumber
    // what is still mapped).
    assert_eq!(
        fixture.keyboard().focused,
        Some(Focused::Popup(1)),
        "the replacement menu should hold the keyboard"
    );
    fixture.disconnect_client();
}

/// Replacing a menu *in the same flush* reuses its serial: the destroy and
/// the new grab are dispatched adjacently with no reap in between, which is
/// exactly how a toolkit switches menus -- and what a test that destroys in
/// one step and re-grabs in the next cannot reproduce, since the reap files
/// the session in between.
///
/// Fail-first against the reap-only session rule: without the destroy-time
/// filing, the replacement arrives before anything recorded the old grab's
/// end, and is refused on its stale serial.
#[test]
fn replacing_a_menu_in_the_same_flush_reuses_its_serial() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.press_a_key();
    let key = fixture
        .serials()
        .key
        .expect("the client should have been sent a key");
    assert!(
        grab_with(&mut fixture, key),
        "the first menu should grab with its fresh serial"
    );

    // Age out everything while the menu is still open, so only the session
    // -- filed synchronously at the destroy below, not a remembered serial
    // -- can accept the replacement.
    fixture
        .state
        .interaction_serials
        .backdate(std::time::Duration::from_secs(3600));

    let Ack::PopupConfigured(configured) = fixture.run(Step::ReplacePopup {
        color: POPUP_BGRA,
        serial: key,
    }) else {
        panic!("the popup step should report whether a configure arrived");
    };
    assert!(
        configured,
        "a same-flush replacement should reuse the ended grab's serial"
    );
    assert_eq!(
        fixture.popup_dones(),
        0,
        "neither the old menu's destroy nor the replacement should count as a dismissal"
    );
    assert!(
        fixture.state.popup_grab.is_some(),
        "the compositor should be holding the replacement grab"
    );
    fixture.disconnect_client();
}

/// ...but after the grace lapses a fresh serial is needed again: the ended
/// session is not a standing permit.
///
/// The session record is backdated directly (rather than sleeping out the
/// grace), so this pins the bound instead of the clock.
#[test]
fn a_replacement_grab_after_the_grace_needs_a_fresh_serial() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.press_a_key();
    let key = fixture
        .serials()
        .key
        .expect("the client should have been sent a key");
    assert!(
        grab_with(&mut fixture, key),
        "the first menu should grab with its fresh serial"
    );

    fixture.run(Step::DestroyPopup);
    assert!(
        fixture.state.popup_grab.is_none(),
        "the destroyed menu should have been reaped"
    );
    // The same holder, but ended long ago -- and every serial aged out too,
    // so nothing at all can accept the replacement.
    assert!(
        fixture.state.last_popup_grab.is_some(),
        "the ended grab should have filed its session"
    );
    fixture.state.last_popup_grab = fixture.state.last_popup_grab.clone().map(|(holder, _)| {
        (
            holder,
            std::time::Instant::now() - std::time::Duration::from_secs(3600),
        )
    });
    fixture
        .state
        .interaction_serials
        .backdate(std::time::Duration::from_secs(3600));

    assert!(
        !grab_with(&mut fixture, key),
        "a replacement menu after the grace should be refused on its stale serial"
    );
    assert_eq!(
        fixture.keyboard().focused,
        Some(Focused::Window(0)),
        "the keyboard should never have left the window"
    );
    fixture.disconnect_client();
}

/// A menu dismissed by clicking outside cannot be reopened on its stale
/// serial: an explicit dismiss ends the session rather than lending it.
///
/// The counterpart to the grace tests above, and the line the narrowed
/// filing draws: only a destroy the holder itself performs files a session
/// (`handlers.rs`'s `destroyed`). A click-outside dismiss reaps through
/// `settle_popup_grab` without filing, so the immediate re-grab below faces
/// the serial check alone -- and loses it, the serial having aged out.
#[test]
fn a_menu_dismissed_by_clicking_outside_cannot_be_reopened_on_its_stale_serial() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.press_a_key();
    let key = fixture
        .serials()
        .key
        .expect("the client should have been sent a key");
    assert!(
        grab_with(&mut fixture, key),
        "the menu should grab with its fresh serial"
    );

    // Age out the opening serial while the menu is still open.
    fixture
        .state
        .interaction_serials
        .backdate(std::time::Duration::from_secs(3600));

    fixture.click(ON_DESKTOP.0, ON_DESKTOP.1);
    assert_eq!(
        fixture.popup_dones(),
        1,
        "clicking outside the menu should dismiss it"
    );
    assert!(
        fixture.state.popup_grab.is_none(),
        "the dismissed menu should have been reaped"
    );
    assert!(
        fixture.state.last_popup_grab.is_none(),
        "a dismissed session must file nothing to reopen from"
    );

    assert!(
        !grab_with(&mut fixture, key),
        "a dismissed menu must not reopen on its stale serial"
    );
    assert_eq!(
        fixture.popup_dones(),
        2,
        "the refused replacement should be dismissed, not left up with no input"
    );
    assert_eq!(
        fixture.keyboard().focused,
        Some(Focused::Window(0)),
        "the keyboard should never have left the window"
    );
    fixture.disconnect_client();
}

/// An enter from long ago spends nothing: the weakening that recording
/// enters buys is bounded by the same age window as every other entry, so
/// this morning's hover is not tonight's keyboard.
#[test]
fn a_stale_enter_serial_is_refused() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    let entered = fixture
        .serials()
        .keyboard_enter
        .expect("mapping focused should deliver a keyboard enter");

    fixture
        .state
        .interaction_serials
        .backdate(std::time::Duration::from_secs(3600));
    assert!(
        !grab_with(&mut fixture, entered),
        "a grab with an aged-out enter serial should never be configured"
    );
    assert!(
        fixture.state.popup_grab.is_none(),
        "the compositor should be holding no grab it refused"
    );
    assert_eq!(
        fixture.keyboard().focused,
        Some(Focused::Window(0)),
        "the keyboard should never have left the window"
    );
    fixture.disconnect_client();
}
