//! The capture recording on the scanout tier -- mark, refusal, force and
//! clearing -- decided without hardware.
//!
//! These drive the code `draw_frame_scanout`, `Backend::capture` and
//! `State::ensure_scanout_capture_current` actually run, with only the two
//! things that need a live device replaced: the `GbmBuffer` a slot is keyed
//! and exported from ([`Captures::record`] takes the key and the export as
//! arguments; `note_frame` supplies them from the buffer), and the render
//! itself (a test says which way a frame landed instead of a `DrmCompositor`
//! deciding). The dma-bufs are real `Dmabuf`s over `/dev/null` fds:
//! nothing on this path binds, maps or reads them, it only records and hands
//! them back, and `Dmabuf` equality is identity, which is what "the capture
//! reads *this* frame" means.

use std::fs::File;
use std::os::fd::OwnedFd;

use smithay::backend::allocator::dmabuf::DmabufFlags;
use smithay::backend::allocator::{Fourcc, Modifier};
use smithay::backend::drm::compositor::FrameFlags;

use super::*;
use crate::compositor::tty::ForceComposite;

/// A distinct dma-buf for a test to record and later recognise. Only its
/// identity matters (see the module doc).
fn fake_dmabuf() -> Dmabuf {
    let fd: OwnedFd = File::open("/dev/null").expect("/dev/null opens").into();
    let mut builder = Dmabuf::builder(
        (4, 4),
        Fourcc::Argb8888,
        Modifier::Linear,
        DmabufFlags::empty(),
    );
    assert!(builder.add_plane(fd, 0, 16));
    builder.build().expect("one plane is a complete dma-buf")
}

/// An export that must not run: recording a slot the pool already holds is
/// a hit, and a hit that re-exported would allocate an fd per frame.
fn no_export() -> Result<Dmabuf, &'static str> {
    panic!("a pooled slot was exported again")
}

/// Records `key` as a freshly drawn composite, exporting `dmabuf` on a pool
/// miss -- what `note_frame` does for a real slot.
fn composite(captures: &mut Captures, key: usize, dmabuf: &Dmabuf) {
    let dmabuf = dmabuf.clone();
    captures.record(key, move || Ok::<_, &'static str>(dmabuf));
}

/// What a capture reads right now: the dma-buf, or the refusal message
/// `Backend::capture` reports (at `CaptureStage::Bind`).
fn target(captures: &mut Captures) -> Result<Dmabuf, &'static str> {
    captures.capture_target().map(|frame| frame.clone())
}

#[test]
fn a_fresh_recording_refuses_as_empty_and_is_owed_a_forced_frame() {
    // Before the first frame there is nothing to read: the capture refuses
    // with the empty message (not the direct one -- nothing went direct),
    // and the forcing path sees a stale recording.
    let mut captures = Captures::default();
    assert!(captures.capture_stale());
    assert_eq!(target(&mut captures), Err(REFUSE_NOTHING));
}

#[test]
fn a_direct_frame_marks_and_the_capture_refuses_rather_than_serve_the_old_screen() {
    // The harm this whole path exists to stop: a composite is recorded, the
    // next damaged frame goes primary-direct, and the recorded composite is
    // still sitting there. The capture must refuse with the direct message
    // -- not hand back the composite, which shows the screen as it was
    // before the direct frame.
    let mut captures = Captures::default();
    let old = fake_dmabuf();
    composite(&mut captures, 1, &old);
    assert_eq!(target(&mut captures), Ok(old));
    captures.note_direct();
    assert!(captures.capture_stale());
    assert_eq!(target(&mut captures), Err(REFUSE_DIRECT));
}

#[test]
fn the_whole_force_sequence_ends_in_a_capture_of_the_forced_frame() {
    // End to end, in the order the live code runs it, with the one thing no
    // test can run (the render) stood in by recording the slot it would
    // have drawn into:
    //
    //   frame 1  composite into slot A  -> capture reads A
    //   frame 2  primary-direct         -> capture refuses (direct)
    //   capture  ensure: stale -> arm   -> the forced frame's flags carry no
    //                                      primary bit
    //   frame 3  composite into slot B  -> mark cleared, capture reads B
    //   frame 4  unforced again         -> may go direct once more
    //
    // Every frame here is on an output judged eligible for primary-direct
    // (a covering fullscreen window): that is the only case in which the
    // unforced flags carry a primary bit at all.
    let mut captures = Captures::default();
    let mut force = ForceComposite::default();
    let (a, b) = (fake_dmabuf(), fake_dmabuf());
    let primary =
        FrameFlags::ALLOW_PRIMARY_PLANE_SCANOUT | FrameFlags::ALLOW_PRIMARY_PLANE_SCANOUT_ANY;

    assert!(force.take_flags(true).contains(primary));
    composite(&mut captures, 0xa, &a);
    assert_eq!(target(&mut captures), Ok(a.clone()));

    assert!(force.take_flags(true).contains(primary));
    captures.note_direct();
    assert_eq!(target(&mut captures), Err(REFUSE_DIRECT));

    // `ensure_scanout_capture_current` arms exactly when the capture would
    // refuse.
    assert!(captures.capture_stale());
    force.arm();
    let forced = force.take_flags(true);
    assert!(!forced.intersects(primary));
    composite(&mut captures, 0xb, &b);
    assert!(!captures.capture_stale());
    let read = target(&mut captures).expect("the forced composite is readable");
    assert_eq!(read, b);
    assert_ne!(read, a);

    assert!(force.take_flags(true).contains(primary));
}

#[test]
fn a_forced_composite_landing_in_a_pooled_slot_clears_the_mark_without_exporting() {
    // The swapchain cycles four slots, so the forced frame usually lands in
    // one already exported: the pool hit must clear the mark exactly like a
    // fresh export does, and must hand back that slot's dma-buf without
    // exporting it again.
    let mut captures = Captures::default();
    let a = fake_dmabuf();
    composite(&mut captures, 0xa, &a);
    captures.note_direct();
    captures.record(0xa, no_export);
    assert_eq!(target(&mut captures), Ok(a));
}

#[test]
fn a_forced_composite_whose_export_fails_keeps_the_mark() {
    // The one way the forced frame can draw and still not be recordable: its
    // slot's export fails. The recorded composite still predates what is on
    // screen, so the mark must survive and the capture must still refuse --
    // clearing it here would serve the pre-direct screen as current.
    let mut captures = Captures::default();
    let old = fake_dmabuf();
    composite(&mut captures, 0xa, &old);
    captures.note_direct();
    captures.record(0xb, || Err("export refused"));
    assert!(captures.capture_stale());
    assert_eq!(target(&mut captures), Err(REFUSE_DIRECT));
}

#[test]
fn a_failed_export_without_a_mark_keeps_serving_the_previous_frame() {
    // The documented lesser wrong, unchanged: no direct frame is involved,
    // so a failed export leaves the previous composite readable (stale by
    // one frame) rather than failing the capture outright.
    let mut captures = Captures::default();
    let old = fake_dmabuf();
    composite(&mut captures, 0xa, &old);
    captures.record(0xb, || Err("export refused"));
    assert_eq!(target(&mut captures), Ok(old));
}

#[test]
fn forgetting_slots_clears_the_mark_and_the_recording_together() {
    // A rebuilt swapchain (mode change, reactivation, CRTC switch, a failed
    // render) invalidates the pool and the recording; the mark describes
    // that recording and goes with it. What is left is the empty state --
    // stale, refusing as empty, owed a forced frame -- never a mark
    // surviving onto whatever the fresh slots draw.
    let mut captures = Captures::default();
    composite(&mut captures, 0xa, &fake_dmabuf());
    captures.note_direct();
    captures.forget_slots();
    assert!(captures.capture_stale());
    assert_eq!(target(&mut captures), Err(REFUSE_NOTHING));
    // And the pool really was dropped: the same key is a miss now.
    let fresh = fake_dmabuf();
    composite(&mut captures, 0xa, &fresh);
    assert_eq!(target(&mut captures), Ok(fresh));
}

#[test]
fn the_force_fires_for_exactly_the_captures_that_would_refuse() {
    // `ensure_scanout_capture_current` keys on `capture_stale`, and
    // `Backend::capture` refuses on `capture_target`. If the two predicates
    // ever disagreed, a capture could either refuse without having been
    // offered a forced frame, or pay for a forced frame it did not need.
    // Walk every state the recording can be in and check they agree.
    let mut captures = Captures::default();
    let check = |captures: &mut Captures| {
        let stale = captures.capture_stale();
        assert_eq!(stale, captures.capture_target().is_err());
    };
    check(&mut captures);
    composite(&mut captures, 1, &fake_dmabuf());
    check(&mut captures);
    captures.note_direct();
    check(&mut captures);
    captures.record(2, || Err("export refused"));
    check(&mut captures);
    composite(&mut captures, 3, &fake_dmabuf());
    check(&mut captures);
    captures.forget_slots();
    check(&mut captures);
}

#[test]
fn the_pool_stays_bounded_past_a_deeper_swapchain() {
    // `SLOT_POOL` is a capacity hint: recording more distinct slots than
    // that evicts the oldest instead of growing, and the evicted slot is a
    // miss (exported again) next time rather than a stale alias.
    let mut captures = Captures::default();
    for key in 0..=SLOT_POOL {
        composite(&mut captures, key, &fake_dmabuf());
    }
    assert_eq!(captures.exported.len(), SLOT_POOL);
    let again = fake_dmabuf();
    composite(&mut captures, 0, &again);
    assert_eq!(target(&mut captures), Ok(again));
}

#[test]
fn staleness_says_why_empty_before_a_frame_and_after_a_rebuild_direct_after_a_direct_frame() {
    // What `ensure_scanout_capture_current` picks its forced frame by: an
    // empty recording needs the swapchain reset (nothing to diff a static
    // screen against), a direct-marked one does not. `capture_stale` stays
    // exactly "some staleness", which is what the force keys on.
    let mut captures = Captures::default();
    assert_eq!(captures.staleness(), Some(Stale::Empty));
    let a = fake_dmabuf();
    composite(&mut captures, 0xa, &a);
    assert_eq!(captures.staleness(), None);
    assert!(!captures.capture_stale());
    captures.note_direct();
    assert_eq!(captures.staleness(), Some(Stale::Direct));
    assert!(captures.capture_stale());
    // A rebuild freed the slots while marked: empty wins -- there is no
    // recording left at all, so the reset is owed.
    captures.forget_slots();
    assert_eq!(captures.staleness(), Some(Stale::Empty));
}
