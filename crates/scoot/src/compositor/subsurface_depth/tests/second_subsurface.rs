//! A second `wl_subsurface` for one surface: refused while the first role
//! object lives on, admitted once it is properly destroyed.
//!
//! The pinned Smithay refuses a second `get_subsurface` only when the
//! surface already has a *parent* (`set_parent`), so destroying the parent
//! `wl_surface` reopens it: the orphan keeps its `wl_subsurface` and a
//! second `get_subsurface` is accepted. scoot tracks the live role object
//! per surface (`subsurface_role.rs`) and refuses with `bad_surface`
//! instead. The depth suite's orphan test stays `bad_parent` -- that link is
//! also too deep, and the depth guard runs first -- so the orphan here is
//! re-attached shallow, where only this guard can refuse it.

use super::*;
use crate::compositor::popup_parent::tests::still_serving;

/// Asserts that `error` is the second-role refusal: `bad_surface`, on the
/// `wl_subcompositor`, posted by scoot's own guard rather than Smithay's
/// "already has a role".
fn assert_second_role(error: &str) {
    assert!(error.contains("wl_subcompositor@"), "{error}");
    assert!(error.contains("bad_surface"), "{error}");
    assert!(error.contains("already has a wl_subsurface"), "{error}");
}

/// Destroying a subsurface's parent `wl_surface` orphans it with its
/// `wl_subsurface` alive, and the pinned Smithay accepts a second
/// `get_subsurface` for it. scoot refuses it instead, and stays serving.
#[test]
fn a_second_subsurface_for_an_orphan_is_bad_surface() {
    let mut fixture = Fixture::with_window();

    // Surface 1 hangs below surface 0; destroying 0 orphans it, role and
    // all. Attaching it under the window is shallow, so the depth guard
    // admits the link -- only the second-role guard refuses it.
    let error = fixture.refused(vec![
        Op::Sub(SubOp::Surface),
        Op::Sub(SubOp::Chain {
            parent: Node::Surface(0),
            len: 1,
            sync: false,
            draw: false,
        }),
        Op::Sub(SubOp::DestroySurface(0)),
        Op::Sync,
        Op::Sub(SubOp::Attach {
            surface: 1,
            parent: Node::Window(0),
        }),
    ]);

    assert_second_role(&error);
    still_serving(&mut fixture);
}

/// A surface properly detached may be made a subsurface again -- twice over
/// in one flush, so rapid re-attach sequences keep working.
#[test]
fn a_surface_detached_twice_can_be_reattached_twice() {
    let mut fixture = Fixture::with_window();
    fixture.batch(vec![
        Op::Sub(SubOp::Surface),
        Op::Sub(SubOp::Attach {
            surface: 0,
            parent: Node::Window(0),
        }),
        Op::Sub(SubOp::Detach(0)),
        Op::Sub(SubOp::Attach {
            surface: 0,
            parent: Node::Window(0),
        }),
        Op::Sub(SubOp::Detach(0)),
        Op::Sub(SubOp::Attach {
            surface: 0,
            parent: Node::Window(0),
        }),
        Op::Sub(SubOp::Commit(Node::Surface(0))),
    ]);
    still_serving(&mut fixture);
}

/// An orphaned subsurface properly detached may be made a subsurface again:
/// destroying the old role object forgets the tracking, so the re-attach is
/// admitted. Without that forget this would be refused as second.
#[test]
fn an_orphan_detached_can_be_reattached() {
    let mut fixture = Fixture::with_window();
    fixture.batch(vec![
        Op::Sub(SubOp::Surface),
        Op::Sub(SubOp::Chain {
            parent: Node::Surface(0),
            len: 1,
            sync: false,
            draw: false,
        }),
        Op::Sub(SubOp::DestroySurface(0)),
        Op::Sub(SubOp::Detach(1)),
        Op::Sub(SubOp::Attach {
            surface: 1,
            parent: Node::Window(0),
        }),
        Op::Sub(SubOp::Commit(Node::Surface(1))),
    ]);
    still_serving(&mut fixture);
}
