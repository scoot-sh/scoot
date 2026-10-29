//! A press outside the `scootlaunch` panel asks it to close.
//!
//! `exclusive` keeps the keyboard until the surface unmaps, so the click
//! cannot take it away; the compositor sends `zwlr_layer_surface_v1.closed`
//! instead (see `State::dismiss_launcher_on_outside_press`). Every assertion
//! is on what the client was actually sent, never on a compositor field. The
//! test client does not obey `closed` on its own, so a test that wants the
//! surface gone destroys it, the way scootlaunch does.

use super::*;

/// The namespace the compositor recognises, spelled out rather than imported:
/// a rename of the constant must fail a test, not follow silently.
const PANEL: &str = "scootlaunch";

fn open_panel(fixture: &mut Fixture) {
    fixture.run(Step::CreateLayer(LayerSpec::launcher(60).named(PANEL)));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    assert_eq!(
        fixture.keyboard().focused,
        Some(Focused::Layer(0)),
        "the panel holds the keyboard before any click"
    );
}

fn closes(fixture: &mut Fixture) -> Vec<u32> {
    fixture.layer_closes()
}

#[test]
fn a_click_on_the_bare_desktop_closes_the_panel() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    open_panel(&mut fixture);
    fixture.click(ON_DESKTOP.0, ON_DESKTOP.1);
    assert_eq!(closes(&mut fixture), vec![1]);
}

/// The press, not the release, decides: one close per click, never two.
#[test]
fn a_press_closes_once_and_a_bare_release_not_at_all() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    open_panel(&mut fixture);
    fixture.state.pointer_move(ON_DESKTOP.0, ON_DESKTOP.1);
    fixture
        .state
        .pointer_button(scoot_ipc::PointerButton::Left, false);
    fixture.settle();
    assert_eq!(
        closes(&mut fixture),
        vec![0],
        "a release alone closes nothing"
    );
    fixture
        .state
        .pointer_button(scoot_ipc::PointerButton::Left, true);
    fixture
        .state
        .pointer_button(scoot_ipc::PointerButton::Left, false);
    fixture.settle();
    assert_eq!(closes(&mut fixture), vec![1]);
}

#[test]
fn a_click_on_the_panel_itself_does_not_close_it() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    open_panel(&mut fixture);
    fixture.click(ON_LAUNCHER.0, ON_LAUNCHER.1);
    assert_eq!(closes(&mut fixture), vec![0]);
    assert_eq!(
        fixture.pointer_focus(),
        Some(Focused::Layer(0)),
        "the click reached the panel, so a row pick still works"
    );
    assert_eq!(fixture.keyboard().focused, Some(Focused::Layer(0)));
}

/// A click on a window closes the panel and still reaches the window; once
/// the panel is gone (the client destroys it on `closed`) the keyboard is the
/// window's.
#[test]
fn a_click_on_a_window_closes_the_panel_and_the_window_gets_the_keyboard() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    open_panel(&mut fixture);
    fixture.click(ON_WINDOW.0, ON_WINDOW.1);
    assert_eq!(closes(&mut fixture), vec![1]);
    assert_eq!(
        fixture.pointer_focus(),
        Some(Focused::Window(0)),
        "the click went on to the window"
    );
    fixture.run(Step::DestroyLayer { index: 0 });
    fixture.settle();
    assert_eq!(fixture.keyboard().focused, Some(Focused::Window(0)));
}

#[test]
fn a_click_on_a_bar_closes_the_panel() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    open_panel(&mut fixture);
    fixture.run(Step::CreateLayer(LayerSpec::bar(30)));
    fixture.run(Step::MapLayer {
        index: 1,
        color: BAR_BGRA,
    });
    fixture.click(100.0, 15.0);
    assert_eq!(closes(&mut fixture), vec![1, 0], "the bar is not closed");
}

/// Any button: a right click outside is as much "I am done here".
#[test]
fn a_right_click_outside_closes_the_panel_too() {
    let mut fixture = Fixture::new();
    open_panel_without_window(&mut fixture);
    fixture.state.pointer_move(ON_DESKTOP.0, ON_DESKTOP.1);
    fixture
        .state
        .pointer_button(scoot_ipc::PointerButton::Right, true);
    fixture
        .state
        .pointer_button(scoot_ipc::PointerButton::Right, false);
    fixture.settle();
    assert_eq!(closes(&mut fixture), vec![1]);
}

/// Zero windows: the panel is the only thing on screen.
fn open_panel_without_window(fixture: &mut Fixture) {
    open_panel(fixture);
}

#[test]
fn with_no_window_a_click_on_the_desktop_closes_the_panel() {
    let mut fixture = Fixture::new();
    open_panel_without_window(&mut fixture);
    fixture.click(ON_DESKTOP.0, ON_DESKTOP.1);
    assert_eq!(closes(&mut fixture), vec![1]);
}

/// The gate: an `exclusive` surface that is not the launcher panel (a
/// locker built on layer-shell, a password prompt) is never closed by a
/// click outside it. Mutation: drop the namespace check and this fails.
#[test]
fn another_exclusive_surface_is_never_closed_by_an_outside_click() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateLayer(
        LayerSpec::launcher(60).named("password-prompt"),
    ));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    fixture.click(ON_DESKTOP.0, ON_DESKTOP.1);
    fixture.click(ON_WINDOW.0, ON_WINDOW.1);
    assert_eq!(closes(&mut fixture), vec![0]);
    assert_eq!(fixture.keyboard().focused, Some(Focused::Layer(0)));
}

/// An `on_demand` panel that a click focused holds the keyboard without
/// `exclusive`: clicking away from it is ordinary click-to-unfocus, not a
/// reason to ask it to close.
#[test]
fn a_click_focused_on_demand_panel_is_left_to_normal_focus() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateLayer(
        LayerSpec::launcher(60)
            .with_keyboard(KeyboardInteractivity::OnDemand)
            .named(PANEL),
    ));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    fixture.click(ON_LAUNCHER.0, ON_LAUNCHER.1);
    assert_eq!(fixture.keyboard().focused, Some(Focused::Layer(0)));
    fixture.click(ON_DESKTOP.0, ON_DESKTOP.1);
    assert_eq!(closes(&mut fixture), vec![0]);
    assert_eq!(fixture.keyboard().focused, Some(Focused::Window(0)));
}

/// A locked session is never touched, whatever is under the pointer.
#[test]
fn a_locked_session_never_closes_the_panel() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    open_panel(&mut fixture);
    fixture.run(Step::LockSession);
    assert!(fixture.state.session_lock.is_locked());
    fixture.click(ON_DESKTOP.0, ON_DESKTOP.1);
    assert_eq!(closes(&mut fixture), vec![0]);
    fixture.disconnect_client();
}

/// Typing never closes it: keyboard-only use is unchanged.
#[test]
fn typing_does_not_close_the_panel() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    open_panel(&mut fixture);
    fixture.state.type_text("firefox").expect("typed");
    fixture.settle();
    assert_eq!(closes(&mut fixture), vec![0]);
}
