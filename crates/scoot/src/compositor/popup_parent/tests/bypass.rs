//! Every way a client could try to lengthen a chain after its popups were
//! admitted -- each refused -- and the well-behaved sequences a toolkit
//! actually sends, each left alone.
//!
//! A cap that counts only a new popup's ancestors is bypassed by moving
//! something that already has children under something deep. Smithay at
//! the pinned rev lets a client do that three ways; the first three groups
//! below are those, each once in isolation and once as the attack at full
//! depth (two [`CAP`]-long chains that would be one twice as long).

use super::*;

/// What the compositor logs when it refuses a destroy that leaves child
/// popups behind.
const TOPMOST_WARNING: &str = "still has child popups";

// --- `xdg_surface.already_constructed` --------------------------------------

/// `get_popup` a second time on a live popup's own `xdg_surface`: Smithay
/// hands out the same role again and overwrites the live popup's parent.
#[test]
fn a_second_get_popup_on_a_live_popups_xdg_surface_is_already_constructed() {
    let mut fixture = Fixture::with_window();
    fixture.batch(vec![Op::Popup(Parent::Window(0))]);

    let error = fixture.refused(vec![Op::GetPopupAgain {
        popup: 0,
        parent: Parent::Window(0),
    }]);

    assert!(error.contains("already_constructed"), "{error}");
    assert!(error.contains("xdg_surface@"), "{error}");
    still_serving(&mut fixture);
}

/// The same through a second `xdg_surface` for the popup's `wl_surface`,
/// which nothing at the pinned rev refuses either: the check is per
/// `wl_surface`, not per `xdg_surface`.
#[test]
fn a_second_xdg_surface_for_a_live_popups_surface_is_already_constructed() {
    let mut fixture = Fixture::with_window();
    fixture.batch(vec![Op::Popup(Parent::Window(0))]);

    let error = fixture.refused(vec![Op::SecondXdgSurface {
        popup: 0,
        parent: Parent::Window(0),
    }]);

    assert!(error.contains("already_constructed"), "{error}");
    still_serving(&mut fixture);
}

/// The attack: a second [`CAP`]-long chain grafted onto the tip of the
/// first by a second `get_popup` on its root, which would make its tip
/// twice the cap deep.
#[test]
fn grafting_a_chain_onto_another_by_a_second_get_popup_is_refused() {
    let mut fixture = Fixture::with_window();
    let chain = Op::Chain {
        parent: Parent::Window(0),
        len: CAP,
    };
    fixture.batch(vec![chain, chain]);

    let error = fixture.refused(vec![Op::GetPopupAgain {
        popup: CAP,
        parent: Parent::Popup(CAP - 1),
    }]);

    assert!(error.contains("already_constructed"), "{error}");
    still_serving(&mut fixture);
}

// --- `xdg_wm_base.not_the_topmost_popup` ------------------------------------

/// Destroying a popup that still has a live child: its surface could then be
/// made a popup again anywhere, children and all.
#[test]
fn destroying_a_popup_with_a_live_child_is_not_the_topmost_popup() {
    // The whole run inside the capture (see `capture_logs`).
    let (error, logs) = test_support::capture_logs(|| {
        let mut fixture = Fixture::with_window();
        fixture.batch(vec![
            Op::Popup(Parent::Window(0)),
            Op::Popup(Parent::Popup(0)),
        ]);
        let error = fixture.refused(vec![Op::Destroy(0)]);
        still_serving(&mut fixture);
        error
    });

    assert!(error.contains("not_the_topmost_popup"), "{error}");
    // The warning the teardown test below checks is *not* logged -- so it
    // can be told apart from a capture that saw nothing at all.
    assert!(logs.contains(TOPMOST_WARNING), "{logs}");
}

/// The attack: the root of a second [`CAP`]-long chain destroyed and made a
/// popup again on the tip of the first, in one flush.
#[test]
fn grafting_a_chain_onto_another_by_recreating_its_root_is_refused() {
    let mut fixture = Fixture::with_window();
    let chain = Op::Chain {
        parent: Parent::Window(0),
        len: CAP,
    };
    fixture.batch(vec![chain, chain]);

    let error = fixture.refused(vec![Op::Reincarnate {
        popup: CAP,
        parent: Parent::Popup(CAP - 1),
    }]);

    assert!(error.contains("not_the_topmost_popup"), "{error}");
    still_serving(&mut fixture);
}

// --- a parent with no live role object --------------------------------------

/// A popup of a bare `xdg_surface` that has no role yet.
#[test]
fn a_popup_of_a_bare_xdg_surface_is_refused() {
    let mut fixture = Fixture::with_window();

    let error = fixture.refused(vec![Op::Bare, Op::Popup(Parent::Bare(0))]);

    assert!(error.contains("invalid_popup_parent"), "{error}");
    still_serving(&mut fixture);
}

/// A popup of a surface whose own `xdg_popup` has been destroyed: the
/// surface keeps the popup role, and could be given a new popup somewhere
/// deeper later.
#[test]
fn a_popup_of_a_destroyed_popups_surface_is_refused() {
    let mut fixture = Fixture::with_window();
    fixture.batch(vec![Op::Popup(Parent::Window(0))]);
    fixture.batch(vec![Op::Destroy(0)]);

    let error = fixture.refused(vec![Op::Popup(Parent::Popup(0))]);

    assert!(error.contains("invalid_popup_parent"), "{error}");
    still_serving(&mut fixture);
}

/// The attack: a [`CAP`]-long chain grown under a bare `xdg_surface`, which
/// is then made a popup itself, under a menu of the window. Each step is
/// within the cap on its own -- the bare surface's new popup is only two
/// deep -- but the chain under it would be `CAP + 2`. Refused at the chain's
/// very first popup, before it can grow.
#[test]
fn a_chain_grown_under_a_bare_xdg_surface_is_refused_at_its_first_popup() {
    let mut fixture = Fixture::with_window();

    let error = fixture.refused(vec![
        Op::Bare,
        Op::Popup(Parent::Bare(0)),
        Op::Sync,
        Op::Chain {
            parent: Parent::Popup(0),
            len: CAP - 1,
        },
        Op::Popup(Parent::Window(0)),
        Op::PopupOnBare {
            bare: 0,
            parent: Parent::Popup(CAP),
        },
    ]);

    assert!(error.contains("invalid_popup_parent"), "{error}");
    still_serving(&mut fixture);
}

// --- a surface made a popup again -------------------------------------------

/// A menu re-shown on the `wl_surface` it had before (GTK does this when it
/// keeps the surface), with a submenu opened on it in the same flush. The
/// old popup's node is still in the window's popup tree then, and the tree
/// finds a parent by surface alone: the submenu went under the *dead* node,
/// where it was never drawn and never configured, and Smithay's own
/// `not_the_topmost_popup` check disconnected the client at the end of the
/// dispatch. Repeated in one flush, it nested the tree as deep as a client
/// liked, whatever each chain's own length.
#[test]
fn a_menu_reshown_on_its_old_surface_draws_its_submenu() {
    let mut fixture = Fixture::with_window();
    fixture.batch(vec![
        Op::Popup(Parent::Window(0)),
        Op::Popup(Parent::Popup(0)),
    ]);
    fixture.map([0, 1], None);

    // Close the submenu, re-show its surface as a menu of the window, and
    // open a submenu of *that*, all at once.
    fixture.batch(vec![
        Op::Reincarnate {
            popup: 1,
            parent: Parent::Window(0),
        },
        Op::Popup(Parent::Popup(1)),
    ]);
    fixture.map([1, 2], Some(2));

    let pixels = fixture.render();
    let (x, y) = fixture.chain_corner(0, 2);
    let centre = (x + POPUP_SIZE / 2, y + POPUP_SIZE / 2);
    assert_eq!(pixel(&pixels, centre), MARKED_BGRA);
}

// --- what toolkits actually send --------------------------------------------

/// Closing a menu with its submenus open, innermost first -- the order GTK3
/// was measured sending in every case tried (click-outside, Escape, picking
/// an item, switching submenus, switching menubar menus) -- then opening it
/// again, is all fine.
#[test]
fn closing_submenus_innermost_first_and_reopening_is_fine() {
    let mut fixture = Fixture::with_window();
    let chain = Op::Chain {
        parent: Parent::Window(0),
        len: 3,
    };
    fixture.batch(vec![chain]);
    fixture.map(0..3, None);

    fixture.batch(vec![Op::Destroy(2), Op::Destroy(1), Op::Destroy(0)]);
    fixture.batch(vec![chain]);
    fixture.map(3..6, Some(5));

    let pixels = fixture.render();
    let (x, y) = fixture.chain_corner(0, 3);
    let centre = (x + POPUP_SIZE / 2, y + POPUP_SIZE / 2);
    assert_eq!(pixel(&pixels, centre), MARKED_BGRA);
}

/// A client that disconnects with a full-depth chain open has its popups
/// destroyed in no particular order, parents before children as often as
/// not. That is teardown, not a `not_the_topmost_popup` -- nothing is
/// posted, nothing is logged -- and the compositor carries on.
#[test]
fn a_client_disconnecting_with_submenus_open_is_torn_down_cleanly() {
    // The whole run inside the capture (see `capture_logs`).
    let ((), logs) = test_support::capture_logs(|| {
        let mut fixture = Fixture::with_window();
        fixture.batch(vec![Op::Chain {
            parent: Parent::Window(0),
            len: CAP,
        }]);
        fixture.map(0..CAP, None);
        fixture.disconnect(0);
        fixture.render();
        still_serving(&mut fixture);
    });

    assert!(!logs.contains(TOPMOST_WARNING), "{logs}");
}
