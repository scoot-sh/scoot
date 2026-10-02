//! Per-surface overlay-tranche dma-buf feedback, as a real client receives
//! it (`dmabuf/scanout.rs`).
//!
//! The mirror of `scanout_feedback.rs` for the overlay-candidate window: a
//! headless `State` has no DRM plane, so each test installs an overlay
//! feedback built from overlay plane format lists it chooses, through the
//! same `OverlayFeedbacks::refresh` the GPU scanout tier runs with its
//! planes' real lists; the steering is then driven through the same
//! `State::steer_overlay_feedback` the tier's frame runs, with the window
//! the frame marked. What is pinned is what reaches the client over the
//! wire -- which feedback, when, and how often -- and that nothing is sent
//! on a frame that changes nothing.

use std::time::{Duration, Instant};

use smithay::backend::allocator::format::FormatSet;
use smithay::backend::allocator::{Format, Fourcc, Modifier};

use super::scanout_feedback::{DEVICE, SCANOUT, SeenFeedback, pair};
use super::*;
use crate::compositor::dmabuf::scanout::{FormatsKey, REVERT_HOLD, Steer};

/// The overlay plane these wire tests steer with: `LINEAR` `AR24` only.
/// Deliberately minimal rather than `apple,dcp`'s full eight-fourcc list, so
/// the tranche over it is exactly one pair under every test renderer (the
/// richer shapes -- more fourccs, two overlays, lost modifiers -- are pinned
/// without a client in `dmabuf/scanout/tests.rs`).
fn overlay_plane() -> FormatSet {
    std::iter::once(Format {
        code: Fourcc::Argb8888,
        modifier: Modifier::Linear,
    })
    .collect()
}

impl Fixture {
    fn install_overlay(&mut self, overlays: &[&FormatSet], planes: u64) {
        self.state
            .install_overlay_feedback(overlays, DEVICE, FormatsKey { planes, lost: 0 });
    }

    /// One frame's overlay steering for `window`'s surface, then flushed to
    /// the client. `window: None` is a frame that marked nothing.
    fn steer_marked(&mut self, window: Option<usize>, now: Instant) -> Steer {
        let output = self.state.outputs.primary_id().expect("a harness output");
        let candidate = window.map(|index| self.id(index));
        let steer = self.state.steer_overlay_feedback(output, candidate, || now);
        self.settle();
        steer
    }

    fn steer_overlay(&mut self) -> Steer {
        self.steer_marked(Some(0), Instant::now())
    }
}

/// A mapped tiled window that asked for surface feedback, and the
/// overlay-shaped feedback installed: the starting point of most tests here.
fn steerable() -> Fixture {
    let mut fixture = Fixture::new();
    fixture.map(WINDOW_BGRA);
    fixture.surface_feedback(0);
    let plane = overlay_plane();
    fixture.install_overlay(&[&plane], 0);
    fixture
}

/// Asserts `feedback` is the overlay feedback built over `default`: the same
/// table and main device, a first tranche flagged `scanout` on [`DEVICE`]
/// holding `overlay`, and then exactly the default's own tranche.
fn assert_overlay(feedback: &SeenFeedback, default: &SeenFeedback, overlay: &[(u32, u64)]) {
    assert_eq!(feedback.table, default.table, "the default's format table");
    assert_eq!(feedback.main_device, default.main_device);
    assert_eq!(feedback.tranches.len(), 2, "{feedback:?}");
    let first = &feedback.tranches[0];
    assert_eq!(first.flags, SCANOUT);
    assert_eq!(first.target_device, DEVICE.to_ne_bytes().to_vec());
    assert_eq!(first.formats, overlay);
    assert_eq!(
        feedback.tranches[1], default.tranches[0],
        "then the main tranche"
    );
}

#[test]
fn a_marked_tiled_window_is_sent_an_overlay_tranche_once() {
    let mut fixture = steerable();
    let seen = fixture.feedbacks(0);
    assert_eq!(seen.len(), 1, "the default, on request");
    let default = seen[0].clone();
    assert_eq!(default.tranches.len(), 1);
    assert_eq!(default.tranches[0].flags, 0, "no scanout flag by default");

    // Nothing marked: nothing to steer.
    assert_eq!(fixture.steer_marked(None, Instant::now()), Steer::Idle);
    assert_eq!(fixture.steer_overlay(), Steer::Sent);
    let seen = fixture.feedbacks(0);
    assert_eq!(seen.len(), 2);
    assert_overlay(
        &seen[1],
        &default,
        &[pair(Fourcc::Argb8888, Modifier::Linear)],
    );

    // Frame after frame: nothing more.
    for _ in 0..5 {
        assert_eq!(fixture.steer_overlay(), Steer::Kept);
    }
    assert_eq!(fixture.feedbacks(0).len(), 2, "nothing is sent per frame");
}

#[test]
fn every_overlay_pair_is_one_the_default_offers_and_the_renderer_imports() {
    // The promise with teeth, overlay arm: a client that allocates from the
    // overlay tranche and then composites is imported like any other client.
    let mut fixture = steerable();
    assert_eq!(fixture.steer_overlay(), Steer::Sent);
    let seen = fixture.feedbacks(0);
    let (default, overlay) = (&seen[0], &seen[1]);
    let tranche = &overlay.tranches[0].formats;
    assert!(!tranche.is_empty(), "the test's own premise");
    for (code, modifier) in tranche {
        assert_eq!(*modifier, u64::from(Modifier::Linear), "LINEAR only");
        let fourcc = Fourcc::try_from(*code).expect("a known fourcc");
        assert!(
            !format!("{fourcc:?}").starts_with('X'),
            "{fourcc:?} is opaque-only: no overlay takes it"
        );
        assert!(default.tranches[0].formats.contains(&(*code, *modifier)));
        let format = Format {
            code: fourcc,
            modifier: Modifier::from(*modifier),
        };
        let (_, output) = fixture.state.outputs.at(0).expect("an output");
        let id = fixture.state.outputs.id_of(&output).expect("an id");
        let backend = fixture.state.backends.get(&id).expect("a render target");
        assert!(
            backend.imports_dmabuf_format(format),
            "{format:?} is offered for scanout and must import"
        );
    }
}

#[test]
fn an_unmarked_frame_holds_then_reverts() {
    // A forced capture frame unmarks exactly one frame: the client keeps its
    // layout for `REVERT_HOLD`, and is reverted only if nothing is marked
    // for that long. Unlike the primary arm, a gone window is not told apart
    // from a transiently unmarked one here -- the hold covers both, because
    // flapping a client's allocations is worse than a delayed revert.
    let mut fixture = steerable();
    assert_eq!(fixture.steer_overlay(), Steer::Sent);
    let start = Instant::now();
    assert_eq!(fixture.steer_marked(None, start), Steer::Holding);
    assert_eq!(
        fixture.steer_marked(None, start + REVERT_HOLD - Duration::from_millis(1)),
        Steer::Holding
    );
    assert_eq!(fixture.feedbacks(0).len(), 2, "nothing sent while holding");
    assert_eq!(
        fixture.steer_marked(None, start + REVERT_HOLD),
        Steer::Reverted
    );
    let seen = fixture.feedbacks(0);
    assert_eq!(seen.len(), 3);
    assert_eq!(seen[2], seen[0], "reverted to the default");
    // Still nothing marked: nothing further.
    assert_eq!(
        fixture.steer_marked(None, start + REVERT_HOLD * 3),
        Steer::Idle
    );
}

#[test]
fn marking_again_after_a_hold_keeps_without_resending() {
    // The common one-shot-capture shape: marked, one forced frame, marked
    // again. The client is never told anything past the first send.
    let mut fixture = steerable();
    assert_eq!(fixture.steer_overlay(), Steer::Sent);
    let start = Instant::now();
    assert_eq!(fixture.steer_marked(None, start), Steer::Holding);
    assert_eq!(fixture.steer_marked(Some(0), start), Steer::Kept);
    assert_eq!(fixture.feedbacks(0).len(), 2);
}

#[test]
fn unmapping_the_marked_window_holds_then_reverts() {
    let mut fixture = steerable();
    assert_eq!(fixture.steer_overlay(), Steer::Sent);
    fixture.done(Step::Unmap { window: 0 });
    // The surface is still alive client-side, so the hold -- not an
    // immediate revert -- covers the frames until the hold passes.
    let start = Instant::now();
    assert_eq!(fixture.steer_marked(None, start), Steer::Holding);
    assert_eq!(
        fixture.steer_marked(None, start + REVERT_HOLD),
        Steer::Reverted
    );
    assert_eq!(fixture.feedbacks(0).len(), 3);
}

#[test]
fn a_covering_fullscreen_window_is_never_steered_the_overlay_tranche() {
    // The fullscreen window owns the primary tranche: steering it the
    // overlay one would overwrite that feedback on its surface (the overlay
    // send runs after the primary one every frame). The mark excludes
    // fullscreen, and covered outputs mark nothing; this is the backstop.
    let mut fixture = steerable();
    fixture.configured(Step::SetFullscreen {
        window: 0,
        output: None,
    });
    fixture.done(Step::Draw {
        window: 0,
        color: WINDOW_BGRA,
    });
    assert_eq!(fixture.steer_overlay(), Steer::Idle);
    assert_eq!(fixture.feedbacks(0).len(), 1, "the default only");
}

#[test]
fn a_surface_asking_after_its_window_was_marked_gets_the_overlay_first() {
    // Like the primary arm's late asker: a client that binds feedback after
    // its window was steered is answered with the overlay feedback straight
    // away, while any other surface gets the default.
    let mut fixture = Fixture::new();
    fixture.map(WINDOW_BGRA);
    fixture.map(OTHER_BGRA);
    let plane = overlay_plane();
    fixture.install_overlay(&[&plane], 0);
    let output = fixture
        .state
        .outputs
        .primary_id()
        .expect("a harness output");
    let now = Instant::now();
    assert_eq!(
        fixture
            .state
            .steer_overlay_feedback(output, Some(fixture.id(0)), || now),
        Steer::Sent
    );
    fixture.settle();
    fixture.surface_feedback(0);
    fixture.surface_feedback(1);
    let marked = fixture.feedbacks(0);
    let other = fixture.feedbacks(1);
    assert_eq!(marked.len(), 1);
    assert_eq!(marked[0].tranches[0].flags, SCANOUT);
    assert_eq!(other.len(), 1);
    assert_eq!(other[0].tranches[0].flags, 0, "the default");
}

#[test]
fn removing_the_output_reverts_at_once() {
    // Unlike the frame path's hold-then-revert, removal knows the output
    // will never mark again: the surface is told the default immediately,
    // not after the hold.
    let mut fixture = steerable();
    assert_eq!(fixture.steer_overlay(), Steer::Sent);
    let output = fixture
        .state
        .outputs
        .primary_id()
        .expect("a harness output");
    let default = fixture
        .state
        .dmabuf_default
        .as_ref()
        .map(|default| default.feedback().clone());
    fixture
        .state
        .overlay_feedback
        .revert_output(output, default.as_ref());
    fixture.settle();
    let seen = fixture.feedbacks(0);
    assert_eq!(seen.len(), 3);
    assert_eq!(seen[2], seen[0], "reverted to the default");
}
