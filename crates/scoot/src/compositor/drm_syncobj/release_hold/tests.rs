//! The release hold's bookkeeping, with a payload that records its own drop
//! (standing in for a `Buffer`, whose drop is what signals a release point)
//! and a fence that records being waited on (standing in for a frame's render
//! `SyncPoint`).

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use smithay::backend::renderer::sync::{Fence, Interrupted, SyncPoint};

use super::{MAX_FRAMES, ReleaseHold};

/// A held "buffer": pushes its name onto `log` when dropped -- the moment a
/// real `Buffer` would signal its release point.
struct Tracked {
    name: &'static str,
    log: Rc<RefCell<Vec<&'static str>>>,
}

impl Drop for Tracked {
    fn drop(&mut self) {
        self.log.borrow_mut().push(self.name);
    }
}

/// A render fence that counts how often it is waited on, and can fail the
/// wait.
#[derive(Debug)]
struct CountingFence {
    waits: Arc<AtomicUsize>,
    interrupted: bool,
}

impl Fence for CountingFence {
    fn is_signaled(&self) -> bool {
        false
    }
    fn wait(&self) -> Result<(), Interrupted> {
        self.waits.fetch_add(1, Ordering::SeqCst);
        if self.interrupted {
            Err(Interrupted)
        } else {
            Ok(())
        }
    }
    fn is_exportable(&self) -> bool {
        false
    }
    fn export(&self) -> Option<std::os::fd::OwnedFd> {
        None
    }
}

struct Rig {
    hold: ReleaseHold<Tracked>,
    log: Rc<RefCell<Vec<&'static str>>>,
    waits: Arc<AtomicUsize>,
}

impl Rig {
    fn new() -> Self {
        Self {
            hold: ReleaseHold::default(),
            log: Rc::default(),
            waits: Arc::default(),
        }
    }

    fn buffer(&self, name: &'static str) -> Tracked {
        Tracked {
            name,
            log: Rc::clone(&self.log),
        }
    }

    fn fence(&self) -> SyncPoint {
        SyncPoint::from(CountingFence {
            waits: Arc::clone(&self.waits),
            interrupted: false,
        })
    }

    fn hold(&mut self, flip: u64, names: &[&'static str]) {
        let sync = self.fence();
        let buffers: Vec<Tracked> = names.iter().map(|name| self.buffer(name)).collect();
        self.hold.hold(flip, sync, buffers);
    }

    fn released(&self) -> Vec<&'static str> {
        let mut log = self.log.borrow().clone();
        log.sort_unstable();
        log
    }

    fn waits(&self) -> usize {
        self.waits.load(Ordering::SeqCst)
    }
}

#[test]
fn a_held_buffer_is_released_only_when_its_flip_completes() {
    let mut rig = Rig::new();
    rig.hold(7, &["a", "b"]);
    assert_eq!(rig.hold.held(), 2);
    assert!(
        rig.released().is_empty(),
        "nothing is released at hold time"
    );
    rig.hold.flip_completed(6);
    assert!(
        rig.released().is_empty(),
        "an earlier flip releases nothing of flip 7"
    );
    rig.hold.flip_completed(7);
    assert_eq!(rig.released(), ["a", "b"]);
    assert_eq!(rig.hold.held(), 0);
    assert_eq!(rig.hold.frames(), 0);
    assert_eq!(
        rig.waits(),
        1,
        "the frame's render is waited out before its buffers go (already done wherever \
         flips are fenced, which is what makes the wait free there)"
    );
}

#[test]
fn a_completed_flip_also_releases_every_earlier_frame() {
    // A queued frame `DrmCompositor` replaced never flips; the later one
    // that does flip carries it out.
    let mut rig = Rig::new();
    rig.hold(1, &["replaced"]);
    rig.hold(2, &["flipped"]);
    rig.hold.flip_completed(2);
    assert_eq!(rig.released(), ["flipped", "replaced"]);
    assert_eq!(rig.waits(), 2);
}

#[test]
fn a_frame_with_no_explicit_buffers_records_nothing() {
    let mut rig = Rig::new();
    rig.hold(1, &[]);
    assert_eq!(rig.hold.frames(), 0);
    // So it cannot push a real frame out through the cap either.
    rig.hold(2, &["x"]);
    rig.hold(3, &[]);
    rig.hold(4, &[]);
    assert_eq!(rig.hold.held(), 1);
    assert_eq!(rig.waits(), 0);
}

#[test]
fn more_frames_than_can_be_in_flight_wait_out_and_release_the_oldest() {
    let mut rig = Rig::new();
    for (flip, name) in [(1, "one"), (2, "two")] {
        rig.hold(flip, &[name]);
    }
    assert_eq!(rig.hold.frames(), MAX_FRAMES);
    assert_eq!(rig.waits(), 0);
    rig.hold(3, &["three"]);
    assert_eq!(
        rig.hold.frames(),
        MAX_FRAMES,
        "the hold never grows past the in-flight bound"
    );
    assert_eq!(rig.released(), ["one"], "the oldest goes, and only it");
    assert_eq!(rig.waits(), 1, "after its render was waited out");
}

#[test]
fn flips_that_never_complete_cannot_grow_the_hold() {
    let mut rig = Rig::new();
    for flip in 0..1000 {
        rig.hold(flip, &["frame"]);
    }
    assert_eq!(rig.hold.frames(), MAX_FRAMES);
    assert_eq!(rig.hold.held(), MAX_FRAMES);
    assert_eq!(rig.released().len(), 1000 - MAX_FRAMES);
}

#[test]
fn a_refused_queue_discards_its_own_frame_without_waiting() {
    // A frame whose queue was refused is never shown, so its buffers go at
    // once: no fence wait on that path.
    let mut rig = Rig::new();
    rig.hold(4, &["kept"]);
    rig.hold(5, &["refused"]);
    rig.hold.discard_frame(5);
    assert_eq!(rig.released(), ["refused"]);
    assert_eq!(rig.waits(), 0);
    assert_eq!(rig.hold.frames(), 1);
    // The number is free again for the retry, which is what the presenter
    // does (it only advances the number on a successful queue).
    rig.hold(5, &["retry"]);
    rig.hold.flip_completed(5);
    assert_eq!(rig.released(), ["kept", "refused", "retry"]);
}

#[test]
fn discard_frame_for_an_unheld_flip_is_a_no_op() {
    let mut rig = Rig::new();
    rig.hold(1, &["a"]);
    rig.hold.discard_frame(9);
    assert_eq!(rig.hold.held(), 1);
    assert_eq!(rig.waits(), 0);
    // Nor does a completion of an earlier flip wait on a later frame.
    rig.hold.flip_completed(0);
    assert_eq!(rig.waits(), 0);
    assert_eq!(rig.hold.held(), 1);
}

#[test]
fn discard_all_releases_everything_without_waiting() {
    // A pause, a reactivation's drain, a rebuilt compositor: nothing held
    // will be shown, and none of those paths may block on a GPU fence.
    let mut rig = Rig::new();
    rig.hold(1, &["a"]);
    rig.hold(2, &["b", "c"]);
    rig.hold.discard_all();
    assert_eq!(rig.released(), ["a", "b", "c"]);
    assert_eq!(rig.waits(), 0);
    assert_eq!(rig.hold.frames(), 0);
    rig.hold.flip_completed(2);
    assert_eq!(rig.released().len(), 3);
    // And the hold carries on normally afterwards.
    rig.hold(3, &["d"]);
    assert_eq!(rig.hold.held(), 1);
}

#[test]
fn release_all_waits_out_every_frame_and_empties() {
    // An errored completion: the frame that just flipped is on screen and
    // unknown, so every held render is waited out first.
    let mut rig = Rig::new();
    rig.hold(1, &["a"]);
    rig.hold(2, &["b", "c"]);
    rig.hold.release_all();
    assert_eq!(rig.released(), ["a", "b", "c"]);
    assert_eq!(rig.waits(), 2);
    assert_eq!(rig.hold.frames(), 0);
    // And a completion arriving afterwards for a released frame is harmless.
    rig.hold.flip_completed(2);
    assert_eq!(rig.released().len(), 3);
}

#[test]
fn an_interrupted_wait_still_releases() {
    let mut rig = Rig::new();
    let sync = SyncPoint::from(CountingFence {
        waits: Arc::clone(&rig.waits),
        interrupted: true,
    });
    let buffer = rig.buffer("stuck");
    rig.hold.hold(1, sync, [buffer]);
    rig.hold.release_all();
    assert_eq!(rig.released(), ["stuck"]);
}

#[test]
fn a_steady_stream_of_frames_reuses_its_storage() {
    // Per-frame path: once warmed up on a pattern, holding and completing
    // frames in that pattern must not grow either vector's allocation.
    let mut rig = Rig::new();
    let step = |rig: &mut Rig, flip: u64| {
        rig.hold(flip, &["a", "b", "c"]);
        if flip % 2 == 1 {
            rig.hold.flip_completed(flip);
        }
    };
    for flip in 0..100 {
        step(&mut rig, flip);
    }
    let (held_cap, frames_cap) = (rig.hold.held.capacity(), rig.hold.frames.capacity());
    for flip in 100..10_000 {
        step(&mut rig, flip);
    }
    assert_eq!(rig.hold.held.capacity(), held_cap);
    assert_eq!(rig.hold.frames.capacity(), frames_cap);
}
