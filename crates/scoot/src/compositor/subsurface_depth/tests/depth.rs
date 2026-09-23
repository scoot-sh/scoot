//! How deep a chain may nest: [`CAP`] levels are drawn to the deepest,
//! synchronized or not, one more is refused, and a chain far past the cap
//! is refused at it rather than taking the compositor down.

use super::*;
use crate::compositor::popup_parent::tests::still_serving;

/// A desynchronized chain exactly [`CAP`] levels deep under a window is
/// admitted in full, and its deepest surface is drawn, on top.
#[test]
fn a_chain_at_the_cap_is_drawn_to_its_deepest_surface() {
    let mut fixture = Fixture::with_window();
    fixture.batch(vec![chain(Node::Window(0), CAP)]);

    let pixels = fixture.render();
    assert_drawn_at(&pixels, window_corner(&fixture, 0), CAP);
}

/// The same, synchronized: nothing shows until the window commits, and that
/// commit applies the whole chain through Smithay's
/// `commit_sync_surface_tree`, which recurses once per level -- a walk a
/// desynchronized chain never takes.
#[test]
fn a_synchronized_chain_at_the_cap_is_drawn_to_its_deepest_surface() {
    let mut fixture = Fixture::with_window();
    fixture.batch(vec![
        Op::Sub(SubOp::Chain {
            parent: Node::Window(0),
            len: CAP,
            sync: true,
            draw: true,
        }),
        Op::Sub(SubOp::Commit(Node::Window(0))),
    ]);

    let pixels = fixture.render();
    assert_drawn_at(&pixels, window_corner(&fixture, 0), CAP);
}

/// One level past the cap is refused with `bad_parent` on the
/// `wl_subcompositor`, and the compositor keeps serving everyone else.
#[test]
fn one_level_past_the_cap_is_refused() {
    let mut fixture = Fixture::with_window();

    let error = fixture.refused(vec![
        chain(Node::Window(0), CAP),
        Op::Sync,
        chain(Node::Surface(CAP - 1), 1),
    ]);

    assert_too_deep(&error);
    still_serving(&mut fixture);
}

/// The attack itself: a desynchronized chain thousands deep, each level
/// drawn, sent in one go. Before the cap this overflowed the compositor's
/// stack (see `subsurface_depth.rs`'s module doc). Now the client is cut off
/// at the cap -- whatever it sees first, the protocol error or the closed
/// socket under the rest of its requests -- and the compositor draws a frame
/// and serves the next client.
#[test]
fn a_chain_thousands_deep_is_cut_off_at_the_cap_not_a_crash() {
    let mut fixture = Fixture::with_window();

    attack(&mut fixture, vec![chain(Node::Window(0), 3000)]);

    still_serving(&mut fixture);
}

/// The same, synchronized, and applied by a commit of the window: the walk
/// that would have overflowed is `commit_sync_surface_tree`.
#[test]
fn a_synchronized_chain_thousands_deep_is_cut_off_at_the_cap_not_a_crash() {
    let mut fixture = Fixture::with_window();

    attack(
        &mut fixture,
        vec![
            Op::Sub(SubOp::Chain {
                parent: Node::Window(0),
                len: 3000,
                sync: true,
                draw: true,
            }),
            Op::Sub(SubOp::Commit(Node::Window(0))),
        ],
    );

    still_serving(&mut fixture);
}

/// A chain under a plain surface, which has no role and so is never drawn,
/// whose surfaces are never given a buffer, and which is synchronized, so
/// no commit walks up it either: the one walk left is Smithay's own `is_ancestor`, which every
/// `get_subsurface` runs up the new parent's chain, recursively, *before*
/// anything scoot sees in `CompositorHandler`. The depth check runs ahead of
/// it, so it never walks more than the cap.
#[test]
fn a_chain_nothing_draws_cannot_drive_the_ancestor_check_deep() {
    let mut fixture = Fixture::with_window();

    attack(
        &mut fixture,
        vec![
            Op::Sub(SubOp::Surface),
            Op::Sub(SubOp::Chain {
                parent: Node::Surface(0),
                len: ANCESTOR_ATTACK,
                sync: true,
                draw: false,
            }),
        ],
    );

    still_serving(&mut fixture);
}

/// How deep [`a_chain_nothing_draws_cannot_drive_the_ancestor_check_deep`]
/// builds: past where `is_ancestor` alone overflowed the 2 MB test-thread
/// stack in a debug build before the cap.
const ANCESTOR_ATTACK: usize = 20_000;
