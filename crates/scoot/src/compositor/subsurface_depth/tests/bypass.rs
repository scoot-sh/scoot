//! Every way to put a surface deeper than the cap without any one link
//! being deep: attaching a surface that already has subsurfaces below it.
//! A subsurface role outlives its `wl_subsurface`, and a surface with no
//! role can be given subsurfaces before it is made one, so a tree need not
//! grow at its leaves.

use super::*;
use crate::compositor::popup_parent::tests::still_serving;

/// A tree built below a plain surface first, then attached: admitted when
/// its deepest surface lands exactly at the cap, and drawn to it.
#[test]
fn a_tree_built_below_a_plain_surface_is_admitted_at_the_cap() {
    let mut fixture = Fixture::with_window();
    fixture.batch(vec![
        Op::Sub(SubOp::Surface),
        chain(Node::Surface(0), CAP - 1),
        Op::Sub(SubOp::Attach {
            surface: 0,
            parent: Node::Window(0),
        }),
        Op::Sub(SubOp::Commit(Node::Surface(0))),
    ]);

    let pixels = fixture.render();
    assert_drawn_at(&pixels, window_corner(&fixture, 0), CAP);
}

/// The same tree one level taller is refused when attached, though the link
/// itself is only one level below a window.
#[test]
fn a_tree_built_below_a_plain_surface_is_refused_past_the_cap() {
    let mut fixture = Fixture::with_window();

    let error = fixture.refused(vec![
        Op::Sub(SubOp::Surface),
        chain(Node::Surface(0), CAP),
        Op::Sync,
        Op::Sub(SubOp::Attach {
            surface: 0,
            parent: Node::Window(0),
        }),
    ]);

    assert_too_deep(&error);
    still_serving(&mut fixture);
}

/// The bypass a cap on the new parent's depth alone would miss: pieces
/// capped in size, each attached below the tip of a fresh one, so every
/// link is made near the top of a short chain and the tree still ends up
/// thousands deep -- then attached to a window and drawn. Refused at the
/// first attach that makes it too deep; a check that ignored the height of
/// what is attached lets the whole thing through, and it overflows the
/// stack.
#[test]
fn a_tree_assembled_bottom_up_from_short_pieces_is_refused() {
    const PIECE: usize = 60;
    const PIECES: usize = 50;
    // Each piece is a plain root and a chain of `PIECE` below it.
    let root = |piece: usize| piece * (PIECE + 1);
    let tip = |piece: usize| root(piece) + PIECE;

    let mut fixture = Fixture::with_window();
    let mut ops = Vec::new();
    for piece in 0..PIECES {
        ops.push(Op::Sub(SubOp::Surface));
        ops.push(chain(Node::Surface(root(piece)), PIECE));
        if piece > 0 {
            ops.push(Op::Sub(SubOp::Attach {
                surface: root(piece - 1),
                parent: Node::Surface(tip(piece)),
            }));
            ops.push(Op::Sync);
        }
    }
    ops.push(Op::Sub(SubOp::Attach {
        surface: root(PIECES - 1),
        parent: Node::Window(0),
    }));
    ops.push(Op::Sub(SubOp::Commit(Node::Surface(root(PIECES - 1)))));

    let error = attack(&mut fixture, ops);

    assert_too_deep(&error);
    still_serving(&mut fixture);
}

/// `wl_subsurface.destroy` on a chain's first link: the surface keeps its
/// role and everything below it, and may be made a subsurface again. At the
/// depth it had, that is admitted and drawn...
#[test]
fn a_detached_tree_can_be_reattached_where_it_was() {
    let mut fixture = Fixture::with_window();
    fixture.batch(vec![
        chain(Node::Window(0), CAP),
        Op::Sub(SubOp::Detach(0)),
        Op::Sub(SubOp::Attach {
            surface: 0,
            parent: Node::Window(0),
        }),
        Op::Sub(SubOp::Commit(Node::Surface(0))),
    ]);

    let pixels = fixture.render();
    assert_drawn_at(&pixels, window_corner(&fixture, 0), CAP);
}

/// ...but one level deeper, it is refused.
#[test]
fn a_detached_tree_cannot_be_reattached_deeper() {
    let mut fixture = Fixture::with_window();

    let error = fixture.refused(vec![
        chain(Node::Window(0), CAP),
        chain(Node::Window(0), 1),
        Op::Sub(SubOp::Detach(0)),
        Op::Sync,
        Op::Sub(SubOp::Attach {
            surface: 0,
            parent: Node::Surface(CAP),
        }),
    ]);

    assert_too_deep(&error);
    still_serving(&mut fixture);
}

/// Destroying a subsurface's parent `wl_surface` orphans it, `wl_subsurface`
/// and all, and the pinned Smithay then accepts a second `get_subsurface`
/// for it (the protocol says that is `bad_surface`; Smithay checks for a
/// parent, not for a live `wl_subsurface`). The depth check covers that
/// link like any other.
#[test]
fn a_tree_orphaned_by_its_parents_destruction_cannot_be_reattached_deeper() {
    let mut fixture = Fixture::with_window();

    // Surface 1 is left a root with `CAP - 2` levels below it; attached
    // below the second of a two-deep chain, its deepest would be `CAP + 1`.
    let error = fixture.refused(vec![
        chain(Node::Window(0), CAP),
        chain(Node::Window(0), 2),
        Op::Sub(SubOp::DestroySurface(0)),
        Op::Sync,
        Op::Sub(SubOp::Attach {
            surface: 1,
            parent: Node::Surface(CAP + 1),
        }),
    ]);

    assert_too_deep(&error);
    still_serving(&mut fixture);
}

/// The check's false refusals, pinned so they stay a decision. A surface's
/// recorded height is a bound that is never lowered, so a surface that
/// once had a tall subtree keeps its height after the subtree is gone:
/// surface 0 had 60 levels below it, they are cut off, and it is then
/// attached where 60 levels would not fit, though nothing is below it. No
/// real client nests anywhere near deep enough to meet this (see
/// `subsurface_depth.rs`).
#[test]
fn a_surface_keeps_the_height_of_a_subtree_it_lost() {
    let mut fixture = Fixture::with_window();

    let error = fixture.refused(vec![
        Op::Sub(SubOp::Surface),
        chain(Node::Surface(0), 60),
        Op::Sub(SubOp::DestroySurface(1)),
        chain(Node::Window(0), 4),
        Op::Sync,
        Op::Sub(SubOp::Attach {
            surface: 0,
            parent: Node::Surface(64),
        }),
    ]);

    assert_too_deep(&error);
    assert!(error.contains("up to 65"), "{error}");
    still_serving(&mut fixture);
}

/// ...and passes it up: attaching that surface (0) below another plain
/// surface (61) raises 61's height as if the lost levels were still there
/// -- 61 levels, though 61 has only ever had one surface below it. Then 61
/// attached three levels down is refused as "up to 65", when the tree it
/// would really make is five deep. Conservative only: the refusal message
/// says "up to" because the number is a bound.
#[test]
fn a_lost_subtrees_height_is_carried_up_to_the_surfaces_above_it() {
    let mut fixture = Fixture::with_window();

    let error = fixture.refused(vec![
        Op::Sub(SubOp::Surface),
        chain(Node::Surface(0), 60),
        Op::Sub(SubOp::DestroySurface(1)),
        Op::Sub(SubOp::Surface),
        // Admitted: 0 + 1 + 60 is within the cap.
        Op::Sub(SubOp::Attach {
            surface: 0,
            parent: Node::Surface(61),
        }),
        chain(Node::Window(0), 3),
        Op::Sync,
        Op::Sub(SubOp::Attach {
            surface: 61,
            parent: Node::Surface(64),
        }),
    ]);

    assert_too_deep(&error);
    assert!(error.contains("up to 65"), "{error}");
    still_serving(&mut fixture);
}

/// A parent that is the surface itself, or one of its descendants, is
/// Smithay's refusal, not the depth check's: the client gets the error it
/// always did.
#[test]
fn a_loop_is_still_smithays_refusal() {
    let mut fixture = Fixture::with_window();

    let error = fixture.refused(vec![
        chain(Node::Window(0), 3),
        Op::Sub(SubOp::Detach(0)),
        Op::Sync,
        Op::Sub(SubOp::Attach {
            surface: 0,
            parent: Node::Surface(2),
        }),
    ]);

    assert!(error.contains("wl_subcompositor@"), "{error}");
    assert!(error.contains("Surface already has a role"), "{error}");
    still_serving(&mut fixture);
}

/// The same at the edge of the depth check's walk: the surface is exactly
/// the cap's number of levels above the new parent, which is as far up as
/// the walk goes before it gives up. It still finds the loop there and
/// leaves it to Smithay's `bad_surface`, rather than stopping one short and
/// calling it too deep.
#[test]
fn a_loop_exactly_the_cap_deep_is_still_smithays_refusal() {
    let mut fixture = Fixture::with_window();

    let error = fixture.refused(vec![
        Op::Sub(SubOp::Surface),
        chain(Node::Surface(0), CAP),
        Op::Sync,
        Op::Sub(SubOp::Attach {
            surface: 0,
            parent: Node::Surface(CAP),
        }),
    ]);

    assert!(error.contains("wl_subcompositor@"), "{error}");
    assert!(error.contains("Surface already has a role"), "{error}");
    still_serving(&mut fixture);
}
