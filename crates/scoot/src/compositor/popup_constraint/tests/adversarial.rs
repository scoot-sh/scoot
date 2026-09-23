//! Positioners and parent chains a client can build that the constraint
//! pass must survive: coordinates at the edge of `i32`, and an ancestor
//! popup that has no parent at all.

use super::*;

/// A positioner whose unadjusted geometry is in range but whose far edge is
/// not: `x = i32::MAX - 10` plus a 60px width. Smithay's own creation-time
/// arithmetic survives it; the constraint pass's (`loc + size`, and a flip's
/// recompute) would overflow -- a panic in a debug build. Beyond
/// `COORDINATE_LIMIT` the popup is left unconstrained instead, exactly where
/// it asked to be, and the compositor keeps serving.
#[test]
fn a_positioner_near_the_edge_of_i32_is_left_unconstrained() {
    let mut fixture = Fixture::one_output();
    let window = fixture.window();
    let spec = Spec {
        offset: (i32::MAX - 10, 0),
        ..Spec::menu_at(0, 0, 60, 40)
    }
    .adjust(Adjust::all());

    let geometry = fixture.popup(Parent::Window(window), spec);

    assert_eq!(geometry, (i32::MAX - 10, 0, 60, 40));
    // ...and the compositor still draws, and still serves.
    let pixels = fixture.frame(1);
    assert!(!test_support::contains(&pixels, POPUP_BGRA));
    assert_eq!(
        fixture.popup(Parent::Window(window), Spec::menu_at(10, 10, 20, 20)),
        (10, 10, 20, 20)
    );
}

/// The same on the negative side, through a reposition.
#[test]
fn a_reposition_near_the_edge_of_i32_is_left_unconstrained() {
    let mut fixture = Fixture::one_output();
    let window = fixture.window();
    fixture.popup(Parent::Window(window), Spec::menu_at(10, 20, 60, 40));
    let spec = Spec {
        offset: (0, i32::MIN + 100),
        gravity: Gravity::TopRight,
        ..Spec::menu_at(0, 0, 60, 40)
    }
    .adjust(Adjust::all());

    let Ack::Repositioned { geometry, .. } = fixture.run(Step::Reposition {
        popup: 0,
        spec,
        token: 1,
    }) else {
        panic!("expected a repositioned popup");
    };

    // Gravity top: the positioner's own geometry puts its top 40 above the
    // offset.
    assert_eq!(geometry, (0, i32::MIN + 100 - 40, 60, 40));
}

/// A popup created parentless (the layer-shell path) that no layer surface
/// ever adopts, with a submenu of its own: the submenu's parent is set, so
/// its commit is legal, but walking up from it finds an ancestor with no
/// parent. Smithay's `get_popup_toplevel_coords` would `unwrap` that
/// parent. The submenu never gets an initial configure (it is in no popup
/// tree, so `send_popup_initial_configure` cannot find it -- unchanged
/// here), but a reposition reaches the constraint pass regardless, and it
/// must answer with the positioner's own geometry rather than panic.
#[test]
fn a_popup_whose_ancestor_has_no_parent_is_left_unconstrained() {
    let mut fixture = Fixture::one_output();
    fixture.window();
    fixture.done(Step::OrphanPopup);
    fixture.done(Step::CommitPopup {
        parent: Parent::Popup(0),
        spec: Spec::menu_at(0, 0, 40, 40),
    });

    let spec = Spec::menu_at(CANVAS - 10, 20, 60, 40).adjust(Adjust::all());
    let Ack::Repositioned { geometry, token } = fixture.run(Step::Reposition {
        popup: 1,
        spec,
        token: 3,
    }) else {
        panic!("expected a repositioned popup");
    };

    assert_eq!(token, 3);
    assert_eq!(geometry, (CANVAS - 10, 20, 60, 40));
}

/// A popup whose parent is its own `xdg_surface`: nothing at the pinned
/// Smithay rev refuses the request, and tracking the popup ran Smithay's
/// walk up its parent chain forever -- one request froze the compositor.
/// It is refused before tracking, with the client disconnected, and the
/// compositor keeps serving other clients.
#[test]
fn a_popup_parented_to_itself_is_refused_not_a_hang() {
    let mut fixture = Fixture::one_output();
    fixture.window();

    let error = fixture.run_expecting_disconnect(Step::SelfParentPopup);

    assert!(error.contains("invalid_popup_parent"), "{error}");
    still_serving(&mut fixture);
}

/// The same loop, two popups long: a popup of a bare `xdg_surface`, then
/// that `xdg_surface` made a popup of its own child. The loop can no longer
/// even be built: its first popup is refused, because a parent with no live
/// role object is (see `popup_parent.rs`) -- with the same error, and the
/// same outcome for everyone else.
#[test]
fn a_two_popup_parent_loop_is_refused_not_a_hang() {
    let mut fixture = Fixture::one_output();
    fixture.window();

    let error = fixture.run_expecting_disconnect(Step::LoopingPopups);

    assert!(error.contains("invalid_popup_parent"), "{error}");
    still_serving(&mut fixture);
}

/// A second client connects, maps a window and opens a constrained menu:
/// the compositor survived whatever the first one did.
fn still_serving(fixture: &mut Fixture) {
    let second = fixture.spawn(run_client);
    let ack = fixture.run_on(second, Step::MapWindow);
    assert!(matches!(ack, Ack::Done), "{ack:?}");
    let window = fixture.state.windows.len() - 1;
    let rect = fixture.rect_of(window);
    let spec = Spec::menu_at(CANVAS - rect.x - 10, 20, 60, 40).adjust(Adjust::SlideX);
    let Ack::Popup(geometry) = fixture.run_on(
        second,
        Step::Popup {
            parent: Parent::Window(0),
            spec,
        },
    ) else {
        panic!("expected a popup configure");
    };
    assert_eq!(geometry, (CANVAS - rect.x - 60, 20, 60, 40));
}
