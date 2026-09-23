//! `zwlr_layer_surface_v1.get_popup`: a bar adopting its own dropdown works
//! as it always did, and adopting anything else is refused.
//!
//! The pinned Smithay overwrites the popup's parent with the layer surface
//! whatever the popup is. Adopting one that already sits deep in a tree
//! cuts its chain to one while its node stays where it was, so the next
//! 63 popups admitted under it by the short chain land 63 levels deeper
//! in the tree -- round after round, past any cap on chains (see
//! `popup_parent.rs`'s module doc).

use super::*;

/// The attack, as the review of #226 found it: a bar's 64-deep dropdown
/// chain, its tip adopted by the bar (chain length one again, node still 64
/// deep), 63 more under it, and again. Forty rounds is a tree about 2600
/// deep, which overflowed the compositor's stack in a debug build. Refused
/// at the first adoption.
#[test]
fn readopting_the_tip_of_a_bars_chain_into_the_bar_is_refused() {
    let mut fixture = Harness::headless(appearance(), CANVAS);
    fixture.spawn(run_client);
    fixture.run(Step::MapBar);
    let mut ops = vec![
        Op::Popup(Parent::Layer(0)),
        Op::Chain {
            parent: Parent::Popup(0),
            len: CAP - 1,
        },
        Op::Adopt {
            popup: CAP - 1,
            layer: 0,
        },
        Op::Sync,
    ];
    let mut tip = CAP - 1;
    for _ in 0..40 {
        ops.push(Op::Chain {
            parent: Parent::Popup(tip),
            len: CAP - 1,
        });
        tip += CAP - 1;
        ops.push(Op::Adopt {
            popup: tip,
            layer: 0,
        });
    }

    let error = fixture.refused(ops);

    assert!(error.contains("invalid_popup_parent"), "{error}");
    assert!(error.contains("zwlr_layer_surface_v1.get_popup"), "{error}");
    still_serving(&mut fixture);
}

/// A window's menu handed to a bar: it was created with a parent, which the
/// protocol does not allow adopting.
#[test]
fn adopting_a_windows_menu_into_a_bar_is_refused() {
    let mut fixture = Fixture::with_window();
    fixture.run(Step::MapBar);
    fixture.batch(vec![Op::Popup(Parent::Window(0))]);

    let error = fixture.refused(vec![Op::Adopt { popup: 0, layer: 0 }]);

    assert!(error.contains("invalid_popup_parent"), "{error}");
    still_serving(&mut fixture);
}

/// The same before the popup's first commit: a popup made with a parent is
/// tracked, and placed in its tree, the moment it is made, so it is just as
/// deep uncommitted -- here the 64th of a window's chain. Only "made with a
/// null parent" rules this one out; "not committed yet" does not.
#[test]
fn adopting_an_uncommitted_popup_that_has_a_parent_is_refused() {
    let mut fixture = Fixture::with_window();
    fixture.run(Step::MapBar);
    fixture.batch(vec![
        Op::Chain {
            parent: Parent::Window(0),
            len: CAP - 1,
        },
        Op::Uncommitted(Parent::Popup(CAP - 2)),
    ]);

    let error = fixture.refused(vec![Op::Adopt {
        popup: CAP - 1,
        layer: 0,
    }]);

    assert!(error.contains("invalid_popup_parent"), "{error}");
    still_serving(&mut fixture);
}

/// A bar adopting its own dropdown a second time, after it has been
/// committed and mapped: the protocol requires adoption before the initial
/// commit.
#[test]
fn adopting_a_committed_dropdown_again_is_refused() {
    let mut fixture = Harness::headless(appearance(), CANVAS);
    fixture.spawn(run_client);
    fixture.run(Step::MapBar);
    fixture.batch(vec![Op::Popup(Parent::Layer(0))]);
    fixture.map([0], None);

    let error = fixture.refused(vec![Op::Adopt { popup: 0, layer: 0 }]);

    assert!(error.contains("invalid_popup_parent"), "{error}");
    still_serving(&mut fixture);
}

/// The ordinary flow, as a bar sends it: a popup with a null parent,
/// adopted and committed; a submenu of it; both closed innermost first; and
/// the dropdown opened again -- this time with the creation, the adoption
/// and the first commit each in a flush of their own, which is just as
/// legal.
#[test]
fn a_bars_dropdown_and_submenu_open_close_and_reopen() {
    let mut fixture = Harness::headless(appearance(), CANVAS);
    fixture.spawn(run_client);
    fixture.run(Step::MapBar);
    fixture.batch(vec![
        Op::Popup(Parent::Layer(0)),
        Op::Popup(Parent::Popup(0)),
    ]);
    fixture.map([0, 1], Some(1));

    // The bar sits at the output's corner: its dropdown is one step in,
    // the submenu two.
    let submenu = (2 * STEP + POPUP_SIZE / 2, 2 * STEP + POPUP_SIZE / 2);
    assert_eq!(pixel(&fixture.render(), submenu), MARKED_BGRA);

    fixture.batch(vec![Op::Destroy(1), Op::Destroy(0)]);
    fixture.batch(vec![Op::Parentless]);
    fixture.batch(vec![Op::Adopt { popup: 2, layer: 0 }]);
    fixture.batch(vec![Op::Commit(2)]);
    fixture.map([2], Some(2));

    let dropdown = (STEP + POPUP_SIZE / 2, STEP + POPUP_SIZE / 2);
    let pixels = fixture.render();
    assert_eq!(pixel(&pixels, dropdown), MARKED_BGRA);
    // The old submenu's far corner, which the new dropdown does not cover:
    // closed, and not drawn.
    let corner = 2 * STEP + POPUP_SIZE - 1;
    assert_ne!(pixel(&pixels, (corner, corner)), MARKED_BGRA);
}
