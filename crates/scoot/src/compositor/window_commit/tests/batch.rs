//! One batch of 1-deep sibling subsurfaces, sent in one flush: the
//! coalescing pin (one recompute, not one per commit) and the correctness
//! pin (exactly the state sequential dispatches produce, nothing dropped or
//! reordered).
//!
//! The shape is the ticket's probe
//! (`docs/backlog/core/subsurface-count-quadratic.md`): `N` desynchronized
//! 4x4 siblings plus the window commit. `SIBLINGS` stays small: the scaling
//! itself is pinned by `scripts/subsurface-flood/run.sh`, not by a
//! wall-clock threshold here -- a threshold would be a flake, not a
//! guarantee.

use scoot_core::WindowId;

use crate::compositor::popup_parent::tests::{
    Fixture, MARKED_BGRA, Node, Op, STEP, SUB_SIZE, SubOp, pixel,
};

/// Siblings per batch.
const SIBLINGS: usize = 200;

/// `n` 1-deep drawn desynchronized siblings of window 0, plus the window
/// commit -- the ticket's probe shape.
fn desync_batch(n: usize) -> Vec<Op> {
    let mut ops: Vec<Op> = std::iter::repeat_n(
        Op::Sub(SubOp::Chain {
            parent: Node::Window(0),
            len: 1,
            sync: false,
            draw: true,
        }),
        n,
    )
    .collect();
    ops.push(Op::Sub(SubOp::Commit(Node::Window(0))));
    ops
}

/// The same siblings synchronized, shown by the parent commit that ends the
/// batch. The cheap path: each child commit is cached, so even before the
/// fix only the window commit recomputed.
fn sync_batch(n: usize) -> Vec<Op> {
    let mut ops: Vec<Op> = std::iter::repeat_n(
        Op::Sub(SubOp::Chain {
            parent: Node::Window(0),
            len: 1,
            sync: true,
            draw: true,
        }),
        n,
    )
    .collect();
    ops.push(Op::Sub(SubOp::Commit(Node::Window(0))));
    ops
}

/// The core id of the one window, in creation order (the harness's own
/// [`Fixture::id`] is private).
fn only_window(fixture: &Fixture) -> WindowId {
    let mut ids: Vec<WindowId> = fixture.state.windows.keys().copied().collect();
    ids.sort();
    assert_eq!(ids.len(), 1, "one window, got {ids:?}");
    ids[0]
}

/// The committed size, i.e. the recomputed bbox: what the coalesced
/// recompute actually wrote.
fn committed_size(fixture: &Fixture) -> (i32, i32) {
    let size = fixture
        .state
        .windows
        .get(&only_window(fixture))
        .expect("the window")
        .geometry()
        .size;
    (size.w, size.h)
}

/// The topmost sibling is drawn: every sibling sits at one [`STEP`] right
/// and down of the window's corner, each chain's last (here: only) surface
/// in [`MARKED_BGRA`], so they all stack on the same spot with marked on
/// top.
fn assert_siblings_drawn(fixture: &mut Fixture) {
    let rect = fixture.rect_of(0);
    let centre = (rect.x + STEP + SUB_SIZE / 2, rect.y + STEP + SUB_SIZE / 2);
    let pixels = fixture.render();
    assert_eq!(pixel(&pixels, centre), MARKED_BGRA);
}

#[test]
fn desync_batch_recomputes_once() {
    let mut fixture = Fixture::with_window();
    fixture.batch(desync_batch(SIBLINGS));
    assert!(
        fixture.state.pending_window_commits.is_empty(),
        "the batch's recompute was not flushed"
    );
    assert_eq!(
        fixture.state.last_window_commit_flush, 1,
        "one batch against one window must recompute once, not once per commit"
    );
    assert_siblings_drawn(&mut fixture);
}

#[test]
fn desync_batch_matches_sequential_commits_exactly() {
    let mut batched = Fixture::with_window();
    batched.batch(desync_batch(SIBLINGS));

    // The same commits, one dispatch each: every commit recomputed on its
    // own, the way code before the fix did even within a batch.
    let mut sequential = Fixture::with_window();
    for _ in 0..SIBLINGS {
        sequential.batch(vec![Op::Sub(SubOp::Chain {
            parent: Node::Window(0),
            len: 1,
            sync: false,
            draw: true,
        })]);
    }
    sequential.batch(vec![Op::Sub(SubOp::Commit(Node::Window(0)))]);

    // Identical resulting state: the same bbox, and the same pixels.
    // Coalescing must not drop or reorder commits.
    assert_eq!(
        committed_size(&batched),
        committed_size(&sequential),
        "a batched flood must land the same bbox as sequential commits"
    );
    assert_eq!(
        batched.render(),
        sequential.render(),
        "a batched flood must draw the same frame as sequential commits"
    );
}

#[test]
fn sync_batch_still_applies_on_the_parent_commit() {
    let mut fixture = Fixture::with_window();
    fixture.batch(sync_batch(SIBLINGS));
    assert_eq!(
        fixture.state.last_window_commit_flush, 1,
        "the synchronized path must stay a single recompute"
    );
    let mut desync = Fixture::with_window();
    desync.batch(desync_batch(SIBLINGS));
    // Both modes show every sibling: same bbox, same frame.
    assert_eq!(committed_size(&fixture), committed_size(&desync));
    assert_eq!(fixture.render(), desync.render());
    assert_siblings_drawn(&mut fixture);
}
