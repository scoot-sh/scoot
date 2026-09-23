//! How deep a chain may nest: [`CAP`] popups are drawn to the deepest, one
//! more is refused, and a chain far past the cap is refused at it rather
//! than taking the compositor down.

use super::*;

/// A chain exactly [`CAP`] popups long is admitted in full, every popup in
/// it is configured, and the deepest is drawn -- on top, where it belongs.
#[test]
fn a_chain_at_the_cap_is_drawn_to_its_deepest_popup() {
    let mut fixture = Fixture::with_window();
    fixture.batch(vec![Op::Chain {
        parent: Parent::Window(0),
        len: CAP,
    }]);
    fixture.map(0..CAP, Some(CAP - 1));

    let pixels = fixture.render();
    let (x, y) = fixture.chain_corner(0, CAP);
    let centre = (x + POPUP_SIZE / 2, y + POPUP_SIZE / 2);
    assert_eq!(pixel(&pixels, centre), MARKED_BGRA);
}

/// One popup past the cap is refused with `invalid_popup_parent` -- in the
/// same flush as a `reposition` of it, which is one more walk up its chain
/// and must never be read -- and the compositor keeps serving everyone
/// else.
#[test]
fn one_popup_past_the_cap_is_refused() {
    let mut fixture = Fixture::with_window();

    let error = fixture.refused(vec![
        Op::Chain {
            parent: Parent::Window(0),
            len: CAP,
        },
        Op::Sync,
        Op::Popup(Parent::Popup(CAP - 1)),
        Op::Reposition(CAP),
    ]);

    assert!(error.contains("invalid_popup_parent"), "{error}");
    assert!(error.contains("xdg_popup@"), "{error}");
    still_serving(&mut fixture);
}

/// The attack itself: a chain thousands deep, sent in one go. Before the
/// cap, a chain this deep overflowed the compositor's stack while it was
/// being built or drawn (see the module doc of `popup_parent.rs`). Now the
/// client is cut off at the cap -- whatever it sees first, the protocol
/// error or the closed socket under the rest of its requests -- and the
/// compositor draws a frame and serves the next client as if nothing
/// happened.
#[test]
fn a_chain_thousands_deep_is_cut_off_at_the_cap_not_a_crash() {
    let mut fixture = Fixture::with_window();

    fixture.refused(vec![Op::Chain {
        parent: Parent::Window(0),
        len: 3000,
    }]);

    fixture.render();
    still_serving(&mut fixture);
}

/// A bar's dropdown -- created parentless and adopted through
/// `zwlr_layer_surface_v1.get_popup` -- nests to the cap like any other
/// chain, and no further. Adoption gives the dropdown a layer surface for a
/// parent, which is not a popup, so its chain is as long before adoption as
/// after: there is no depth for adoption itself to add, and nothing for it
/// to check.
#[test]
fn a_bars_dropdown_nests_to_the_cap_and_no_further() {
    let mut fixture = Harness::headless(appearance(), CANVAS);
    fixture.spawn(run_client);
    fixture.run(Step::MapBar);
    fixture.batch(vec![
        Op::Popup(Parent::Layer(0)),
        Op::Chain {
            parent: Parent::Popup(0),
            len: CAP - 1,
        },
    ]);
    fixture.map(0..CAP, Some(CAP - 1));

    // The bar sits at the output's corner, so its chain's deepest popup is
    // `CAP` steps in from there.
    let pixels = fixture.render();
    let corner = i32::try_from(CAP).expect("a small cap") * STEP;
    let centre = (corner + POPUP_SIZE / 2, corner + POPUP_SIZE / 2);
    assert_eq!(pixel(&pixels, centre), MARKED_BGRA);

    let error = fixture.refused(vec![Op::Popup(Parent::Popup(CAP - 1))]);
    assert!(error.contains("invalid_popup_parent"), "{error}");
    still_serving(&mut fixture);
}
