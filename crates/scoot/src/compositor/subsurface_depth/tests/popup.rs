//! Subsurfaces below popups: the two caps do not multiply on the stack,
//! because a popup's surface tree is walked on its own, never from inside
//! the walk of its parent's popups -- so both at once are drawn, and past
//! either one is still refused.

use super::*;
use crate::compositor::popup_parent::tests::{CAP as POPUP_CAP, Parent, still_serving};

/// A popup chain at the popup cap, and below its deepest popup a subsurface
/// chain at the subsurface cap: drawn, to the deepest subsurface, and one
/// more subsurface is refused.
#[test]
fn a_subsurface_chain_at_the_cap_below_a_popup_chain_at_its_cap_is_drawn() {
    let mut fixture = Fixture::with_window();
    fixture.batch(vec![Op::Chain {
        parent: Parent::Window(0),
        len: POPUP_CAP,
    }]);
    fixture.map(0..POPUP_CAP, None);
    fixture.batch(vec![chain(Node::Popup(POPUP_CAP - 1), CAP)]);

    let pixels = fixture.render();
    assert_drawn_at(&pixels, fixture.chain_corner(0, POPUP_CAP), CAP);

    let error = fixture.refused(vec![chain(Node::Surface(CAP - 1), 1)]);
    assert_too_deep(&error);
    still_serving(&mut fixture);
}
