//! An IME candidate window over a lock screen's password field.
//!
//! The popup is tracked and positioned but -- before this work -- never
//! rendered: the locked render path replaces the element list wholesale with
//! the lock surfaces plus backdrop, and sends frame callbacks to the lock
//! surfaces only. Someone whose passphrase needs an IME gets no candidate
//! window. Composition itself works, so every claim here is about *pixels*
//! and *frame callbacks*, never about composed text.
//!
//! The trust half is as load-bearing as the rendering half (see the module
//! doc this file's tests pin): whatever the locked path gathers beyond the
//! lock surfaces must be placeable there by nobody but the lock client
//! itself and the compositor's own IME parenting. So each inclusion test
//! has an exclusion twin, and the exclusion twin matters more.
//!
//! Harness notes: one client plays the locker (lock + lock surface + the
//! text field and IME objects -- the text input must belong to the same
//! client as the surface it enables, exactly as `input_method/tests.rs`
//! documents), while a second client owns the background window whose
//! popups must stay invisible. The IME popup's screen position is the text
//! cursor rectangle: the lock surface fills the output from the origin, and
//! `parent_geometry` answers the default rectangle for it, so the candidate
//! lands at the raw surface-local caret.

use super::*;

/// Where the test password field's caret sits, surface-local -- and, by the
/// paragraph above, on screen.
const CURSOR: (i32, i32) = (30, 40);

/// The lock half of the setup every test here shares: this client's input
/// method exists before focus lands, the session locks, its surface maps,
/// and its text field enables against the focused lock surface.
fn lock_with_ime_field(fixture: &mut Fixture, locker: usize) {
    fixture.run_on(locker, Step::SetupIme);
    fixture.run_on(locker, Step::Lock);
    fixture.run_on(locker, Step::map_lock_surface(0));
    fixture.run_on(locker, Step::EnableTextInput);
    fixture.run_on(
        locker,
        Step::SetCursorRectangle {
            x: CURSOR.0,
            y: CURSOR.1,
            w: 2,
            h: 18,
        },
    );
}

fn ime_status(fixture: &mut Fixture, client: usize) -> ImeReport {
    let Ack::ImeStatus(report) = fixture.run_on(client, Step::ImeStatus) else {
        panic!("the IME status step should report IME state");
    };
    report
}

/// The headline: an IME candidate popup parented to the focused lock surface
/// is drawn -- at the caret -- and its frame callbacks arrive, so an
/// animated IME does not stall behind the lock screen.
#[test]
fn ime_popup_on_the_focused_lock_surface_is_drawn_and_gets_frames() {
    let mut fixture = Fixture::new();
    lock_with_ime_field(&mut fixture, 0);
    fixture.run(Step::CreateImePopup);
    assert!(
        ime_status(&mut fixture, 0).activated,
        "the input method never activated against the lock screen's text field"
    );

    fixture.run(Step::RequestImeFrame);
    let pixels = fixture.render();
    assert!(
        contains(&pixels, IME_BGRA),
        "the IME candidate window is not on screen over the lock screen"
    );
    assert_pixel(
        &pixels,
        CANVAS,
        CURSOR.0 + 2,
        CURSOR.1 + 2,
        IME_BGRA,
        "the IME candidate window is not at the password field's caret",
    );
    assert!(
        ime_status(&mut fixture, 0).ime_frame,
        "the IME popup asked for a frame and never got one: an animated IME stalls"
    );
}

/// The other end of the lifecycle while locked: the password field goes
/// away, the input method deactivates, and the candidate window must stop
/// being drawn -- a popup left tracked would keep rendering over the lock
/// screen after its field is gone.
#[test]
fn disabling_the_lock_screens_text_field_dismisses_its_popup() {
    let mut fixture = Fixture::new();
    lock_with_ime_field(&mut fixture, 0);
    fixture.run(Step::CreateImePopup);
    assert!(
        contains(&fixture.render(), IME_BGRA),
        "the IME candidate window should be on screen before the field is disabled"
    );

    fixture.run(Step::DisableTextInput);
    let pixels = fixture.render();
    assert!(
        !contains(&pixels, IME_BGRA),
        "the IME popup is still drawn over the lock screen after its field was disabled"
    );
    assert!(
        !ime_status(&mut fixture, 0).activated,
        "the input method is still active after its lock-screen field was disabled"
    );
}

/// A background window's `xdg_popup` -- mapped, real pixels, no grab so the
/// lock's own grab dismissal does not take it down -- is still tracked
/// against its window while locked, and must still not be drawn. This is the
/// PR #44 password guarantee extended to the lock path's new element source:
/// whatever was left open keeps no pixels and gets no callbacks.
#[test]
fn a_background_windows_xdg_popup_is_not_drawn_over_the_lock_screen() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::MapXdgPopup { window: 0 });
    assert!(
        contains(&fixture.render(), XDG_POPUP_BGRA),
        "the background popup should be on screen before the lock"
    );

    let locker = fixture.connect();
    fixture.run_on(locker, Step::Lock);
    fixture.run_on(locker, Step::map_lock_surface(0));
    // Armed while locked: a callback that arrives is a client being told to
    // draw, which is what "stop rendering normal clients" forbids.
    fixture.run_on(0, Step::RequestXdgPopupFrame { popup: 0 });
    let pixels = fixture.render();
    assert_whole_screen_is(
        &pixels,
        LOCK_BGRA,
        "a background window's popup leaked over the lock screen",
    );
    assert_eq!(
        ime_status(&mut fixture, 0).xdg_frames,
        vec![0],
        "a background popup got a frame callback while locked"
    );
}

/// Same, for an IME popup parented to a background window: mapped and
/// animated before the lock, gone -- pixels and callbacks -- after it. The
/// lock moves keyboard (and with it text-input) focus onto the lock surface,
/// which deactivates the IME against the window and dismisses its popup; the
/// compositor never re-parents it anywhere the locked path gathers.
#[test]
fn a_background_windows_ime_popup_is_not_drawn_over_the_lock_screen() {
    let mut fixture = Fixture::new();
    fixture.run(Step::SetupIme);
    fixture.run(Step::MapWindow);
    fixture.run(Step::EnableTextInput);
    fixture.run(Step::SetCursorRectangle {
        x: 8,
        y: 8,
        w: 2,
        h: 10,
    });
    fixture.run(Step::CreateImePopup);
    fixture.run(Step::RequestImeFrame);
    assert!(
        contains(&fixture.render(), IME_BGRA),
        "the background IME popup should be on screen before the lock"
    );
    assert!(
        ime_status(&mut fixture, 0).ime_frame,
        "the background IME popup should get frames before the lock"
    );

    let locker = fixture.connect();
    fixture.run_on(locker, Step::Lock);
    fixture.run_on(locker, Step::map_lock_surface(0));
    fixture.run_on(0, Step::RequestImeFrame);
    let pixels = fixture.render();
    assert!(
        !contains(&pixels, IME_BGRA),
        "a background window's IME popup leaked over the lock screen"
    );
    assert!(
        !ime_status(&mut fixture, 0).ime_frame,
        "a background IME popup got a frame callback while locked"
    );
}

/// Unlocking hands the screen back: the background popup that the lock hid
/// draws again and its callbacks resume. No stuck state -- nothing the lock
/// tore down (grab, focus, tracking) may leave the popup unmapped-but-alive
/// or callback-starved once the session is back.
#[test]
fn unlocking_restores_popup_rendering_and_frames() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::MapXdgPopup { window: 0 });

    let locker = fixture.connect();
    // Twice, back to back: a rapid lock/unlock cycle must leave neither the
    // hidden-while-locked nor the drawn-after-unlock state stuck.
    for n in 0..2 {
        fixture.run_on(locker, Step::Lock);
        fixture.run_on(locker, Step::map_lock_surface(n));
        assert_whole_screen_is(
            &fixture.render(),
            LOCK_BGRA,
            "a background window's popup leaked over the lock screen",
        );

        fixture.run_on(locker, Step::Unlock { lock: n });
        fixture.run_on(0, Step::RequestXdgPopupFrame { popup: 0 });
        let pixels = fixture.render();
        assert!(
            contains(&pixels, XDG_POPUP_BGRA),
            "the background popup did not come back after unlock"
        );
        assert_eq!(
            ime_status(&mut fixture, 0).xdg_frames,
            vec![1],
            "the background popup got no frame callback after unlock"
        );
    }
}
