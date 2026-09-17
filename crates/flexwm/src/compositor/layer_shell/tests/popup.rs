//! `xdg_popup`: mapping, explicit grabs, and where the keyboard goes while
//! one is up.
//!
//! Shares the real-client harness in the parent module (see its doc for why
//! these drive a live `wayland-client` connection rather than calling
//! handlers directly). Everything here asserts on what the *client* was
//! told -- `wl_keyboard.enter`, `xdg_popup.popup_done` -- or on real
//! rendered pixels, because "who holds the keyboard" is a claim about the
//! wire, not about a field in `State`.

use super::*;
use flexwm_core::{Action, WindowId};
use flexwm_ipc::{Request, Response};

/// An `xdg_popup` gets its initial configure, maps, draws and tears down
/// without taking the compositor with it.
///
/// This is the fix for `docs/backlog/protocols/xdg-popup-never-configured.md`
/// proving itself: it replaces the pinned-gap test that asserted no popup
/// is ever configured (deleted with that entry), and inverts its
/// assertion -- and then goes further,
/// because a configure the client cannot use is no fix. The popup acks,
/// attaches a buffer and maps; its pixels reach the framebuffer (which is
/// what "no popup maps at all" denied); its frame callback completes
/// (`Window::send_frame` covers popup surfaces, and this is the test that
/// would catch it if that ever stopped); exactly one configure arrived
/// (later commits stay quiet, as a non-reactive positioner requires); and
/// destroying the popup leaves the compositor serving.
#[test]
fn an_xdg_popup_configures_maps_draws_and_tears_down() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);

    let before = fixture.render();
    assert!(
        !contains(&before, POPUP_BGRA),
        "the popup color should be absent before any popup exists"
    );

    let Ack::PopupConfigured(configured) = fixture.run(Step::MapPopup {
        parent: PopupParent::Window,
        color: POPUP_BGRA,
        grab: None,
    }) else {
        panic!("the popup step should report whether a configure arrived");
    };
    assert!(
        configured,
        "the popup never got its initial configure -- the fix regressed"
    );

    let pixels = fixture.render();
    assert!(
        contains(&pixels, POPUP_BGRA),
        "the mapped popup's pixels should reach the framebuffer"
    );
    assert_eq!(
        fixture.frames(),
        vec![1],
        "one frame should complete the popup's requested callback"
    );
    assert_eq!(
        fixture.popup_configures(),
        1,
        "the popup should be configured exactly once -- later commits stay quiet"
    );

    fixture.run(Step::DestroyPopup);
    // ...and the compositor survived the whole lifecycle, still serving.
    assert_eq!(fixture.usable(), WHOLE);
    let after = fixture.render();
    assert_eq!(after.len(), (CANVAS * CANVAS * 4) as usize);
    assert!(
        !contains(&after, POPUP_BGRA),
        "the destroyed popup's pixels should be gone"
    );
    fixture.disconnect_client();
}

// -------------------------------------------------------------------------
// Input: grabs, and where the keyboard goes while a menu is up
// -------------------------------------------------------------------------

/// Maps a grabbing popup on the first window, and hands back the point to
/// click to land on the popup itself.
///
/// The shared opening of every grab test below: one real keystroke so the
/// client has an input serial to grab with (a toolkit always does), and the
/// assertion that the grab actually took -- so a test that goes on to check
/// what *ends* the grab cannot pass by never having started one. The caller
/// maps the windows, because how many there are is the test's business.
fn window_with_grabbing_popup(fixture: &mut Fixture) -> (f64, f64) {
    fixture.press_a_key();
    assert!(
        matches!(fixture.keyboard().focused, Some(Focused::Window(_))),
        "a window should hold the keyboard before any popup exists"
    );

    let Ack::PopupConfigured(configured) = fixture.run(Step::MapPopup {
        parent: PopupParent::Window,
        color: POPUP_BGRA,
        grab: Some(GrabSource::Key),
    }) else {
        panic!("the popup step should report whether a configure arrived");
    };
    assert!(configured, "a grabbing popup should still be configured");
    assert!(
        fixture.state.popup_grab.is_some(),
        "the compositor should be holding the grab it granted"
    );

    let pixels = fixture.render();
    find_color(&pixels, POPUP_BGRA).expect("the grabbing popup should have drawn")
}

/// The headline: an explicit grab moves the keyboard onto the popup, and
/// destroying the popup gives it back to the window.
///
/// This is what Escape-to-close in an application menu needs -- not the
/// Escape key itself, which is the client's business, but the `enter` that
/// makes the menu the thing an Escape is delivered to at all.
#[test]
fn a_popup_grab_moves_the_keyboard_onto_the_popup_and_gives_it_back() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    window_with_grabbing_popup(&mut fixture);

    let during = fixture.keyboard();
    assert_eq!(
        during.focused,
        Some(Focused::Popup(0)),
        "the grabbing popup should hold the keyboard"
    );

    // Typed keys really arrive there, which is the point of the focus move:
    // `enter` without delivery would look identical in the focus field.
    let before = during.keys;
    fixture.press_a_key();
    assert_eq!(
        fixture.keyboard().keys,
        before + 2,
        "the press and release should both reach the popup"
    );

    fixture.run(Step::DestroyPopup);
    let after = fixture.keyboard();
    assert_eq!(
        after.focused,
        Some(Focused::Window(0)),
        "destroying the popup should hand the keyboard back to its window"
    );
    assert!(
        fixture.state.popup_grab.is_none(),
        "the ended grab should not be held after the popup is gone"
    );
    fixture.disconnect_client();
}

/// A click over a popup reaches the popup, not the window behind it.
///
/// Pointer hit-testing already walked into popups before any of this landed
/// (`WindowSurfaceType::ALL` in `State::surface_under`); this pins it, since
/// a popup that draws but cannot be clicked is the failure mode the whole
/// feature is about. Asserted with no grab in play, so it is the hit test
/// under test and not the grab's own focus routing.
#[test]
fn a_click_over_a_popup_reaches_the_popup() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::MapPopup {
        parent: PopupParent::Window,
        color: POPUP_BGRA,
        grab: None,
    });

    let pixels = fixture.render();
    let (x, y) = find_color(&pixels, POPUP_BGRA).expect("the popup should have drawn");
    fixture.click(x, y);
    assert_eq!(
        fixture.pointer_focus(),
        Some(Focused::Popup(0)),
        "the pointer should have entered the popup, not the window under it"
    );

    // ...and the window behind it is still reachable once the popup is gone,
    // so the hit test is following the popup rather than swallowing clicks.
    fixture.run(Step::DestroyPopup);
    fixture.click(ON_WINDOW.0, ON_WINDOW.1);
    assert_eq!(fixture.pointer_focus(), Some(Focused::Window(0)));
    fixture.disconnect_client();
}

/// A click outside a grabbing popup dismisses it: the client is sent
/// `popup_done`, and the keyboard comes back to the window.
///
/// The half of "nothing ever dismisses a menu today" that does not need the
/// client to do anything at all.
#[test]
fn a_click_outside_a_grabbing_popup_dismisses_it() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    window_with_grabbing_popup(&mut fixture);
    assert_eq!(fixture.popup_dones(), 0, "nothing has dismissed it yet");

    fixture.click(ON_DESKTOP.0, ON_DESKTOP.1);
    assert_eq!(
        fixture.popup_dones(),
        1,
        "clicking outside the menu should dismiss it"
    );
    assert_eq!(
        fixture.keyboard().focused,
        Some(Focused::Window(0)),
        "and the keyboard should be back on the window"
    );
    assert!(fixture.state.popup_grab.is_none());

    // The dismissed popup is out of the parent's tree immediately, so its
    // pixels go on the next frame rather than when the client gets around to
    // destroying it.
    assert!(
        !contains(&fixture.render(), POPUP_BGRA),
        "a dismissed menu should stop being drawn"
    );
    fixture.disconnect_client();
}

/// A click *inside* a grabbing popup keeps it: the grab is for routing
/// input into the menu, not for closing it on the first click.
#[test]
fn a_click_inside_a_grabbing_popup_keeps_it() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    let (x, y) = window_with_grabbing_popup(&mut fixture);

    fixture.click(x, y);
    assert_eq!(
        fixture.popup_dones(),
        0,
        "clicking the menu itself must not dismiss it"
    );
    assert_eq!(
        fixture.keyboard().focused,
        Some(Focused::Popup(0)),
        "and the menu should still hold the keyboard"
    );
    assert!(fixture.state.popup_grab.is_some());
    fixture.disconnect_client();
}

// -------------------------------------------------------------------------
// Precedence: what a popup grab loses to
// -------------------------------------------------------------------------

/// An `exclusive` layer surface mapping while a menu is open takes the
/// keyboard, and the menu is dismissed rather than left holding input it
/// cannot use.
///
/// `layer_shell.rs` documents `exclusive` on `top`/`overlay` as "takes the
/// keyboard the moment it maps and keeps it until it unmaps". A popup grab
/// swallows `set_focus` while it is live, so without dismissing it here the
/// launcher would be on screen with a text field nothing could type into --
/// the exact "one silently wins forever" state this precedence exists to
/// prevent.
#[test]
fn an_exclusive_layer_surface_pre_empts_an_open_popup_grab() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    window_with_grabbing_popup(&mut fixture);

    fixture.run(Step::CreateLayer(LayerSpec::launcher(60)));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });

    assert_eq!(
        fixture.popup_dones(),
        1,
        "the launcher should have dismissed the menu, not silently outranked it"
    );
    assert_eq!(
        fixture.keyboard().focused,
        Some(Focused::Layer(0)),
        "the launcher should hold the keyboard"
    );
    assert!(fixture.state.popup_grab.is_none());

    // ...and it is really typeable, which is the thing that was at risk.
    let before = fixture.keyboard().keys;
    fixture.press_a_key();
    assert_eq!(
        fixture.keyboard().keys,
        before + 2,
        "keys should reach the launcher"
    );
    fixture.disconnect_client();
}

/// The same rule at grant time: a popup that asks for a grab while a
/// launcher already holds the keyboard is refused and dismissed, rather than
/// granted and revoked a moment later.
///
/// Checked separately because the two paths are genuinely different --
/// `refresh_keyboard_focus` handles "the launcher arrived second", and only
/// `grab_popup` handles "the launcher was already there".
#[test]
fn a_popup_grab_is_refused_while_a_launcher_holds_the_keyboard() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    // Before the launcher: with `exclusive` focus the keys go to the layer
    // surface, and this is only here to give the client a serial to grab
    // with -- which is a fact about the client, not about who has focus.
    fixture.press_a_key();
    fixture.run(Step::CreateLayer(LayerSpec::launcher(60)));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    assert_eq!(fixture.keyboard().focused, Some(Focused::Layer(0)));

    fixture.run(Step::MapPopup {
        parent: PopupParent::Window,
        color: POPUP_BGRA,
        grab: Some(GrabSource::Key),
    });
    assert!(
        fixture.state.popup_grab.is_none(),
        "the grab should have been refused outright"
    );
    assert_eq!(
        fixture.popup_dones(),
        1,
        "and the popup dismissed, not left up with no input"
    );
    assert_eq!(
        fixture.keyboard().focused,
        Some(Focused::Layer(0)),
        "the launcher should never have lost the keyboard"
    );
    fixture.disconnect_client();
}

/// An `exclusive` layer surface's *own* popup grab is accepted: the launcher
/// that opened the menu is not outranked by itself.
///
/// The failure `docs/backlog/protocols/popup-grab-exclusive-self-dismiss.md`
/// files: `popup_grab_outranked()` refused a new grab whenever
/// `layer_keyboard_focus()` reported `exclusive: true`, without checking
/// whether the exclusive surface *is* the grab's own root -- so a dropdown
/// opened by an `exclusive` launcher flashed open and instantly closed,
/// dismissed by the very surface that opened it. A *different* exclusive
/// surface still refuses one; see
/// `a_popup_grab_is_refused_while_a_launcher_holds_the_keyboard`.
#[test]
fn an_exclusive_layer_surfaces_own_popup_grab_is_accepted() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateLayer(LayerSpec::launcher(60)));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    assert_eq!(
        fixture.keyboard().focused,
        Some(Focused::Layer(0)),
        "the launcher should hold the keyboard before its menu exists"
    );
    // The key goes to the launcher, which is what gives the client a serial
    // to grab with -- the same shape every other grab test opens with.
    fixture.press_a_key();

    let Ack::PopupConfigured(configured) = fixture.run(Step::MapPopup {
        parent: PopupParent::Layer(0),
        color: POPUP_BGRA,
        grab: Some(GrabSource::Key),
    }) else {
        panic!("the popup step should report whether a configure arrived");
    };
    assert!(
        configured,
        "the launcher's own menu should still be configured"
    );
    assert!(
        fixture.state.popup_grab.is_some(),
        "the compositor should be holding the launcher's own grab"
    );
    assert_eq!(
        fixture.popup_dones(),
        0,
        "the launcher must not dismiss the menu it just opened"
    );
    assert_eq!(
        fixture.keyboard().focused,
        Some(Focused::Popup(0)),
        "the launcher's menu should hold the keyboard"
    );

    // ...and typed keys really arrive there, which is the point of the menu.
    let before = fixture.keyboard().keys;
    fixture.press_a_key();
    assert_eq!(
        fixture.keyboard().keys,
        before + 2,
        "the press and release should both reach the launcher's menu"
    );
    fixture.disconnect_client();
}

/// An `exclusive` layer surface does not pre-empt its *own* popup grab --
/// including a nested submenu off the same root -- when a later refresh
/// re-derives focus.
///
/// The second half of the same ticket: `refresh_keyboard_focus` dismissed an
/// existing grab whenever an `exclusive` surface was up, even when that
/// surface was the root the grab belonged to. Mapping a window is what forces
/// the refresh here, the way real session churn does. A grab rooted on a
/// window while a *different* exclusive surface is up is still pre-empted;
/// see `an_exclusive_layer_surface_pre_empts_an_open_popup_grab`.
#[test]
fn an_exclusive_layer_surface_does_not_pre_empt_its_own_popup_grab() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateLayer(LayerSpec::launcher(60)));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    fixture.press_a_key();
    let Ack::PopupConfigured(configured) = fixture.run(Step::MapPopup {
        parent: PopupParent::Layer(0),
        color: POPUP_BGRA,
        grab: Some(GrabSource::Key),
    }) else {
        panic!("the popup step should report whether a configure arrived");
    };
    assert!(
        configured,
        "the launcher's own menu should still be configured"
    );

    // A nested submenu off the same root: same chain, same root, so the
    // refresh below must spare it too.
    fixture.run(Step::MapPopup {
        parent: PopupParent::Popup(0),
        color: WALLPAPER_BGRA,
        grab: Some(GrabSource::Key),
    });
    assert_eq!(
        fixture.keyboard().focused,
        Some(Focused::Popup(1)),
        "the submenu should take the keyboard from its parent menu"
    );

    fixture.run(Step::MapWindow);
    assert_eq!(
        fixture.popup_dones(),
        0,
        "re-deriving focus must not dismiss the launcher's own menu chain"
    );
    assert!(
        fixture.state.popup_grab.is_some(),
        "the launcher's own grab should survive the refresh"
    );
    assert_eq!(
        fixture.keyboard().focused,
        Some(Focused::Popup(1)),
        "the submenu should still hold the keyboard"
    );
    fixture.disconnect_client();
}

/// Dismissing a window-rooted menu for a *different* exclusive surface stays
/// dismissed when that surface goes away: the root check narrows dismissal,
/// it never re-grants one.
///
/// The unmap edge of the same ticket: once `popup_done` is sent the popup is
/// out of its parent's tree, and nothing -- including the pre-empting surface
/// unmapping -- may bring the grab back.
#[test]
fn unmapping_the_pre_empting_launcher_does_not_resurrect_a_dismissed_grab() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    window_with_grabbing_popup(&mut fixture);
    fixture.run(Step::CreateLayer(LayerSpec::launcher(60)));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    assert_eq!(
        fixture.popup_dones(),
        1,
        "the launcher should have dismissed the window's menu"
    );
    assert!(fixture.state.popup_grab.is_none());

    fixture.run(Step::UnmapLayer { index: 0 });
    assert_eq!(
        fixture.popup_dones(),
        1,
        "nothing re-grants a dismissed menu"
    );
    assert!(
        fixture.state.popup_grab.is_none(),
        "no grab comes back with the dismissal's cause gone"
    );
    assert_eq!(
        fixture.keyboard().focused,
        Some(Focused::Window(0)),
        "the keyboard goes back to the window, not to a menu"
    );
    fixture.disconnect_client();
}

/// Unmapping the exclusive root mid-grab neither dismisses its own menu
/// nor disturbs it: the unmap path never dismissed a grab (an unmapped
/// surface stops being `exclusive` rather than becoming a third party), and
/// the root check keeps that answer rather than changing it.
///
/// The other half of the unmap edge above: there the pre-empting surface
/// goes away after dismissing someone else's menu, here the menu's own
/// root goes away while the menu is up.
#[test]
fn unmapping_the_exclusive_root_mid_grab_leaves_its_own_menu_up() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateLayer(LayerSpec::launcher(60)));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    fixture.press_a_key();
    let Ack::PopupConfigured(configured) = fixture.run(Step::MapPopup {
        parent: PopupParent::Layer(0),
        color: POPUP_BGRA,
        grab: Some(GrabSource::Key),
    }) else {
        panic!("the popup step should report whether a configure arrived");
    };
    assert!(
        configured,
        "the launcher's own menu should still be configured"
    );

    fixture.run(Step::UnmapLayer { index: 0 });
    assert_eq!(
        fixture.popup_dones(),
        0,
        "unmapping the root must not dismiss its own menu"
    );
    assert!(
        fixture.state.popup_grab.is_some(),
        "the launcher's own grab should survive its root unmapping"
    );
    assert_eq!(
        fixture.keyboard().focused,
        Some(Focused::Popup(0)),
        "the menu should still hold the keyboard"
    );
    fixture.disconnect_client();
}

/// Locking the session takes the keyboard off a menu.
///
/// The security-relevant one: `PopupKeyboardGrab` ignores `set_focus` while
/// it is live, so a grab left installed across a lock would route every
/// keystroke of the user's password to whatever client had a menu open. No
/// lock *surface* is created here on purpose -- with none, the correct
/// keyboard focus while locked is nobody, and "nobody" is exactly as strong
/// an assertion as "the lock client" for this question.
#[test]
fn locking_the_session_takes_the_keyboard_off_a_popup_grab() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    window_with_grabbing_popup(&mut fixture);

    fixture.run(Step::LockSession);
    assert!(
        fixture.state.session_lock.is_locked(),
        "the lock should have been accepted"
    );
    assert_eq!(
        fixture.popup_dones(),
        1,
        "locking should dismiss the menu, not leave it grabbing"
    );
    assert!(fixture.state.popup_grab.is_none());
    assert_eq!(
        fixture.keyboard().focused,
        None,
        "nothing but a lock surface may hold the keyboard while locked"
    );

    // And no keystroke reaches the client that had the menu.
    let before = fixture.keyboard().keys;
    fixture.press_a_key();
    assert_eq!(
        fixture.keyboard().keys,
        before,
        "keys must not reach a client behind the lock screen"
    );

    // Nor can it take the keyboard back by opening a *new* menu: a grab
    // asked for while the session is already locked is refused at grant
    // time, which is a different code path from the pre-emption above and
    // the one an adversarial client would actually reach for.
    fixture.run(Step::MapPopup {
        parent: PopupParent::Window,
        color: POPUP_BGRA,
        grab: Some(GrabSource::Key),
    });
    assert!(
        fixture.state.popup_grab.is_none(),
        "a grab requested while locked must be refused"
    );
    assert_eq!(
        fixture.keyboard().focused,
        None,
        "and the keyboard must stay off it"
    );
    fixture.disconnect_client();
}

/// A *click-focused* `on_demand` layer surface does **not** outrank a popup
/// grab -- the bar's own menu would otherwise dismiss itself the moment it
/// opened.
///
/// The other side of the precedence rule, and the one a bar depends on: the
/// surface that opened the menu is the surface a refresh would hand the
/// keyboard back to, so treating it like an `exclusive` one would make a
/// bar's dropdown impossible to use.
#[test]
fn a_clicked_layer_surface_does_not_pre_empt_a_popup_grab() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.press_a_key();
    fixture.run(Step::CreateLayer(
        LayerSpec::launcher(60).with_keyboard(KeyboardInteractivity::OnDemand),
    ));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    fixture.click(ON_LAUNCHER.0, ON_LAUNCHER.1);
    assert_eq!(fixture.keyboard().focused, Some(Focused::Layer(0)));

    fixture.run(Step::MapPopup {
        parent: PopupParent::Layer(0),
        color: POPUP_BGRA,
        grab: Some(GrabSource::Key),
    });
    assert!(
        fixture.state.popup_grab.is_some(),
        "a bar's own menu should be allowed to grab"
    );
    assert_eq!(
        fixture.popup_dones(),
        0,
        "and must not be dismissed by the surface that opened it"
    );
    assert_eq!(
        fixture.keyboard().focused,
        Some(Focused::Popup(0)),
        "the bar's menu should hold the keyboard"
    );

    // Closing it hands the keyboard back to the bar -- to the *bar*, which
    // is what the click earned, and not merely to whatever Smithay's grab
    // considers the root.
    fixture.run(Step::DestroyPopup);
    assert_eq!(
        fixture.keyboard().focused,
        Some(Focused::Layer(0)),
        "the bar should get its click-earned keyboard focus back"
    );
    fixture.disconnect_client();
}

/// A bar that never wanted the keyboard does not end up holding it when its
/// own menu closes.
///
/// Smithay's grab restores focus to the popup's *root*, which here is a
/// `keyboard_interactivity: none` bar -- a surface `layer_shell.rs`'s policy
/// says may never hold the keyboard. This is the case `settle_popup_grab`
/// exists for, and it would pass just as happily with the wrong answer if it
/// asserted on `State` instead of on what the client was told.
#[test]
fn a_bar_that_wants_no_keyboard_does_not_keep_one_when_its_menu_closes() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.press_a_key();
    fixture.run(Step::CreateLayer(LayerSpec::bar(30)));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    assert_eq!(
        fixture.keyboard().focused,
        Some(Focused::Window(0)),
        "a `none` bar never takes the keyboard"
    );

    fixture.run(Step::MapPopup {
        parent: PopupParent::Layer(0),
        color: POPUP_BGRA,
        grab: Some(GrabSource::Key),
    });
    assert_eq!(
        fixture.keyboard().focused,
        Some(Focused::Popup(0)),
        "the bar's menu holds the keyboard while it is up"
    );

    fixture.run(Step::DestroyPopup);
    assert_eq!(
        fixture.keyboard().focused,
        Some(Focused::Window(0)),
        "focus goes back to the window, not to the bar the grab was rooted on"
    );
    assert!(!fixture.state.keyboard_on_layer);
    fixture.disconnect_client();
}

/// Keybindings keep firing while a menu holds the keyboard.
///
/// The property that makes it safe to let a client take every keystroke at
/// all, and the same one `layer_shell.rs` relies on for an `exclusive`
/// surface: `input.rs`'s `key()` matches bindings in the filter Smithay runs
/// *before* `input_forward`, and only `input_forward` consults the grab. So
/// the VT-switch binds and `quit` stay reachable even if a client wedges
/// with a menu open -- which is the escape hatch a keyboard grab needs.
#[test]
fn keybindings_still_fire_while_a_popup_grabs_the_keyboard() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::MapWindow);
    window_with_grabbing_popup(&mut fixture);

    let before = fixture.keyboard();
    assert_eq!(before.focused, Some(Focused::Popup(0)));
    let focused = fixture.state.focus;
    fixture
        .state
        .press(&flexwm_ipc::KeyCombo {
            key: "h".into(),
            modifiers: vec![flexwm_ipc::Modifier::Super],
        })
        .expect("a pressable combo");
    fixture.settle();

    assert_ne!(
        fixture.state.focus, focused,
        "Super+h should still move window focus with a menu open"
    );
    let after = fixture.keyboard();
    assert_eq!(
        after.focused,
        Some(Focused::Popup(0)),
        "and the menu should keep the keyboard -- a binding is not a focus change"
    );
    assert_eq!(
        after.keys,
        before.keys + 2,
        "only the Super modifier's own press and release reach the popup; \
         the bound `h` is intercepted"
    );
    fixture.disconnect_client();
}

/// A submenu grabs on top of its parent menu, and closing it unwinds to the
/// parent rather than all the way out.
///
/// Nested grabs are a protocol feature (`xdg_popup.grab` on a popup whose
/// parent is the current grab), and the unwind is the part a menu bar
/// depends on: closing a submenu must leave the menu it came from usable.
#[test]
fn a_nested_popup_grab_unwinds_to_its_parent() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    window_with_grabbing_popup(&mut fixture);

    fixture.run(Step::MapPopup {
        parent: PopupParent::Popup(0),
        color: WALLPAPER_BGRA,
        grab: Some(GrabSource::Key),
    });
    assert_eq!(
        fixture.keyboard().focused,
        Some(Focused::Popup(1)),
        "the submenu should take the keyboard from its parent menu"
    );
    assert_eq!(fixture.popup_dones(), 0, "neither has been dismissed");

    // Closing the submenu: the parent menu is still grabbing, so the grab is
    // not over and must not be reaped.
    fixture.run(Step::DestroyPopup);
    assert!(
        fixture.state.popup_grab.is_some(),
        "the parent menu's grab should survive its submenu closing"
    );
    fixture.run(Step::DestroyPopup);
    assert_eq!(
        fixture.keyboard().focused,
        Some(Focused::Window(0)),
        "closing the last menu hands the keyboard back to the window"
    );
    assert!(fixture.state.popup_grab.is_none());
    fixture.disconnect_client();
}

// -------------------------------------------------------------------------
// Teardown edge cases
// -------------------------------------------------------------------------

/// A client that dies while its popup is grabbing the seat leaves nothing
/// behind: no held grab, no keyboard stuck on a dead surface, and a
/// compositor still serving.
#[test]
fn a_client_that_dies_while_grabbing_leaves_nothing_behind() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    window_with_grabbing_popup(&mut fixture);

    fixture.disconnect_client();
    assert!(
        fixture.state.popup_grab.is_none(),
        "a dead client's grab must not be held here"
    );
    assert_eq!(fixture.usable(), WHOLE);
    let pixels = fixture.render();
    assert_eq!(pixels.len(), (CANVAS * CANVAS * 4) as usize);
    assert!(
        !contains(&pixels, POPUP_BGRA),
        "the dead client's menu should be gone"
    );
}

/// A popup parented to a *layer surface* (`zwlr_layer_surface_v1.get_popup`)
/// -- a bar's own dropdown menu or tooltip -- configures, maps and draws.
///
/// The half of popup support that reaches a bar rather than an application.
#[test]
fn a_layer_parented_popup_configures_maps_and_draws() {
    let mut fixture = Fixture::new();
    fixture.run(Step::CreateLayer(LayerSpec::bar(30)));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });

    let before = fixture.render();
    assert!(
        !contains(&before, POPUP_BGRA),
        "the popup color should be absent before any popup exists"
    );

    let Ack::PopupConfigured(configured) = fixture.run(Step::MapPopup {
        parent: PopupParent::Layer(0),
        color: POPUP_BGRA,
        grab: None,
    }) else {
        panic!("the popup step should report whether a configure arrived");
    };
    assert!(
        configured,
        "a layer-surface-parented popup should get its initial configure"
    );

    let pixels = fixture.render();
    assert!(
        contains(&pixels, POPUP_BGRA),
        "a bar's own dropdown should reach the framebuffer"
    );
    assert_eq!(
        fixture.popup_configures(),
        1,
        "exactly one configure, as for a window's popup"
    );
    // The reason `WlrLayerShellHandler::new_popup` stays unimplemented, and
    // the assertion that catches it if someone "fixes" that: the popup is
    // already tracked once, through `XdgShellHandler::new_popup`. Tracking it
    // again there puts a second node for the same surface in the layer
    // surface's `PopupTree`, and `LayerSurface::send_frame` walks that tree
    // -- so one requested callback would complete twice. Measured, not
    // assumed; see the resolution doc.
    assert_eq!(
        fixture.popups_on_first_layer(),
        1,
        "the bar's popup should be tracked exactly once"
    );

    fixture.run(Step::DestroyPopup);
    let after = fixture.render();
    assert!(
        !contains(&after, POPUP_BGRA),
        "the destroyed dropdown's pixels should be gone"
    );
    assert_eq!(
        fixture.usable(),
        Rect::new(0, 30, CANVAS, CANVAS - 30),
        "the bar's own reservation should be untouched by its popup"
    );
    fixture.disconnect_client();
}

// -------------------------------------------------------------------------
// IPC visibility: which window's popup holds the keyboard
// -------------------------------------------------------------------------

/// `flexwm msg windows` reports the popup-grab holder alongside `focused`.
///
/// The divergence
/// `docs/backlog/protocols/popup-grab-survives-window-focus-change.md` files:
/// a focus-changing action moves compositor focus to window B while the
/// seat's real keyboard stays on window A's grabbing popup. An agent that
/// only reads `focused` would inject its next keystroke at B and silently
/// reach A's menu instead; `popup_grab` is the field that tells the two
/// apart, without changing the grab semantics
/// `keybindings_still_fire_while_a_popup_grabs_the_keyboard` pins.
#[test]
fn windows_reports_the_popup_grab_holder_alongside_focus() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    let grabbed: WindowId = fixture
        .state
        .focus
        .expect("mapping the only window focuses it");
    window_with_grabbing_popup(&mut fixture);

    // The ticket's trigger in miniature: window focus moves elsewhere while
    // the grab lives on. An explicit focus action here; a `Super+h`-style
    // keybinding does the same -- see
    // `keybindings_still_fire_while_a_popup_grabs_the_keyboard`.
    fixture.run(Step::MapWindow);
    let other = *fixture
        .state
        .windows
        .keys()
        .find(|id| **id != grabbed)
        .expect("two mapped windows");
    fixture.state.act(Action::FocusWindowId(other));
    fixture.settle();
    assert_eq!(
        fixture.state.focus,
        Some(other),
        "compositor focus should have moved to the other window"
    );

    // ...while the real keyboard never left the menu.
    assert_eq!(
        fixture.keyboard().focused,
        Some(Focused::Popup(0)),
        "the grab should still hold the keyboard after the focus move"
    );

    let Response::Windows { windows } = fixture.state.handle_request(Request::Windows) else {
        panic!("a windows request answers with its windows");
    };
    assert_eq!(windows.len(), 2, "both windows are listed");
    let holder = windows
        .iter()
        .find(|snapshot| snapshot.id == grabbed.0)
        .expect("the grabbing window is listed");
    let focused = windows
        .iter()
        .find(|snapshot| snapshot.id == other.0)
        .expect("the focused window is listed");
    assert!(
        !holder.focused,
        "compositor focus moved to the other window"
    );
    assert!(
        holder.popup_grab,
        "the grabbing window reports the keyboard its menu holds"
    );
    assert!(focused.focused);
    assert!(
        !focused.popup_grab,
        "the focused window claims no grab -- keys sent at it would reach the menu instead"
    );
    fixture.disconnect_client();
}

/// No grab, no report: without an active `xdg_popup.grab` every window reads
/// `popup_grab: false` -- including one with a mapped popup that never
/// grabbed, which is the "only the grabbing one matters" half of the
/// contract. An agent can rely on all-false meaning no window's menu holds
/// the keyboard.
#[test]
fn windows_reports_no_popup_grab_without_an_active_grab() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    // A mapped but non-grabbing popup (a tooltip, say): visible, keyboardless.
    fixture.run(Step::MapPopup {
        parent: PopupParent::Window,
        color: POPUP_BGRA,
        grab: None,
    });
    fixture.run(Step::MapWindow);

    let Response::Windows { windows } = fixture.state.handle_request(Request::Windows) else {
        panic!("a windows request answers with its windows");
    };
    assert_eq!(windows.len(), 2, "both windows are listed");
    assert!(
        windows.iter().all(|snapshot| !snapshot.popup_grab),
        "nothing grabbed the keyboard, so no window may claim the grab"
    );
    fixture.disconnect_client();
}

/// A grab rooted at a layer surface (a bar's own dropdown) holds the keyboard
/// without belonging to any window, so every window honestly reports
/// `popup_grab: false` -- the field names the grabbing *window*, and there
/// is none.
#[test]
fn windows_reports_no_popup_grab_holder_for_a_layer_rooted_grab() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateLayer(LayerSpec::bar(30)));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    fixture.press_a_key();
    let Ack::PopupConfigured(configured) = fixture.run(Step::MapPopup {
        parent: PopupParent::Layer(0),
        color: POPUP_BGRA,
        grab: Some(GrabSource::Key),
    }) else {
        panic!("the popup step should report whether a configure arrived");
    };
    assert!(
        configured,
        "a layer-parented popup should still be configured"
    );
    assert!(
        fixture.state.popup_grab.is_some(),
        "the compositor should be holding the layer-rooted grab -- \
         otherwise this test passes vacuously"
    );

    let Response::Windows { windows } = fixture.state.handle_request(Request::Windows) else {
        panic!("a windows request answers with its windows");
    };
    assert!(
        windows.iter().all(|snapshot| !snapshot.popup_grab),
        "a bar's dropdown holds the keyboard, and no window may claim it"
    );
    fixture.disconnect_client();
}
