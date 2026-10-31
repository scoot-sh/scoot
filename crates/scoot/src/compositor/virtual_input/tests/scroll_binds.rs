//! Scroll binds from a virtual pointer (wayvnc's shape): the same
//! `[virtual_input] binds` gate virtual keyboards sit behind, with the
//! modifier held on a virtual keyboard -- so these also pin that a virtual
//! Super reaches the seat's modifier state the scroll filter reads.

use scoot_core::Action;

use super::*;
use crate::compositor::keybindings::{ScrollDir, Trigger};

/// Maps a window, binds `super+scroll-down` to fullscreen, holds a virtual
/// Super, and scrolls one wheel click down over the window the way wayvnc
/// does (wheel source, a discrete click, framed). Returns what the client
/// heard.
fn scroll_down_with_super(fixture: &mut Fixture) -> Vec<Seen> {
    map_focused_window(fixture);
    fixture.state.keybindings.insert_trigger(
        SUPER,
        Trigger::Scroll(ScrollDir::Down),
        Bound::Action(Action::ToggleFullscreen),
        BindFlags::default(),
    );
    fixture.run(Step::CreatePointer { with_output: false });
    fixture.run(Step::CreateKeyboard);
    fixture.run(Step::UploadKeymap {
        layout: "us".to_owned(),
    });
    let (x, y) = window_point(fixture);
    move_to(fixture, x, y);
    fixture.take_log();
    press_chord(fixture, "us", &[("Super_L", true)]);
    fixture.run(Step::AxisSource { source: 0 });
    fixture.run(Step::AxisDiscrete {
        axis: 0,
        value: 15.0,
        discrete: 1,
    });
    fixture.run(Step::Frame);
    let log = fixture.take_log();
    press_chord(fixture, "us", &[("Super_L", false)]);
    log
}

#[test]
fn a_virtual_scroll_runs_the_bind_when_binds_are_on() {
    let mut fixture = Fixture::with_binds();
    let log = scroll_down_with_super(&mut fixture);
    assert!(
        is_fullscreen(&fixture),
        "a virtual Super + wheel click must run the scroll bind with binds on"
    );
    let heard_scroll = log.iter().any(|seen| {
        matches!(
            seen,
            Seen::PointerAxis { .. } | Seen::PointerAxisDiscrete { .. }
        )
    });
    assert!(
        !heard_scroll,
        "the client must not hear a scroll a bind took: {log:?}"
    );
}

#[test]
fn a_virtual_scroll_reaches_the_client_whole_when_binds_are_off() {
    let mut fixture = Fixture::enabled();
    assert!(!fixture.state.virtual_input_binds);
    let log = scroll_down_with_super(&mut fixture);
    assert!(
        !is_fullscreen(&fixture),
        "binds are off: a virtual scroll must not run a scroll bind"
    );
    assert!(
        log.contains(&Seen::PointerAxisDiscrete {
            axis: 0,
            discrete: 1
        }),
        "the click must reach the client: {log:?}"
    );
}
