//! Per-surface scanout-tranche dma-buf feedback, as a real client receives
//! it (`dmabuf/scanout.rs`).
//!
//! A headless `State` has no DRM plane, so each test installs a scanout
//! feedback built from a plane format list it chooses, through the same
//! `ScanoutFeedbacks::refresh` the GPU scanout tier runs with its plane's
//! real list; the steering is then driven with the judgement the tier's
//! frame makes (`State::primary_direct_now`), through the same
//! `State::steer_scanout_feedback`. What is pinned is what reaches the
//! client over the wire -- which feedback, when, and how often -- and that
//! nothing is sent on a frame that changes nothing.

use std::os::fd::OwnedFd;
use std::time::{Duration, Instant};

use smithay::backend::allocator::format::FormatSet;
use smithay::backend::allocator::{Format, Fourcc, Modifier};
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols::wp::linux_dmabuf::zv1::client::{
    zwp_linux_dmabuf_feedback_v1, zwp_linux_dmabuf_v1,
};

use super::*;
use crate::compositor::dmabuf::scanout::{FormatsKey, REVERT_HOLD, Steer};

/// The `target_device` the tests' scanout tranche names: any value that is
/// not the harness's `main_device`, so the two tranches are told apart.
const DEVICE: libc::dev_t = 0xfeed;

/// `zwp_linux_dmabuf_feedback_v1.tranche_flags`' `scanout` bit.
const SCANOUT: u32 = 1;

/// One tranche, as the client saw it, with its indices resolved against the
/// feedback's format table.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct SeenTranche {
    target_device: Vec<u8>,
    flags: u32,
    formats: Vec<(u32, u64)>,
}

/// One complete (`done`-terminated) feedback, as the client saw it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct SeenFeedback {
    main_device: Vec<u8>,
    table: Vec<(u32, u64)>,
    tranches: Vec<SeenTranche>,
}

/// Per toplevel: the feedback being received and every complete one.
#[derive(Default)]
pub(super) struct Feedbacks {
    building: Vec<SeenFeedback>,
    tranche: Vec<SeenTranche>,
    complete: Vec<Vec<SeenFeedback>>,
}

impl Feedbacks {
    /// Makes room for the `window`-th toplevel's feedback.
    pub(super) fn expect(&mut self, window: usize) {
        while self.complete.len() <= window {
            self.building.push(SeenFeedback::default());
            self.tranche.push(SeenTranche::default());
            self.complete.push(Vec::new());
        }
    }

    pub(super) fn complete(&self, window: usize) -> Vec<SeenFeedback> {
        self.complete.get(window).cloned().unwrap_or_default()
    }
}

impl Dispatch<zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1, ()> for TestClient {
    fn event(
        _: &mut Self,
        _: &zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1,
        _: zwp_linux_dmabuf_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // Only the pre-feedback `format`/`modifier` events arrive here; this
        // suite reads feedback objects.
    }
}

impl Dispatch<zwp_linux_dmabuf_feedback_v1::ZwpLinuxDmabufFeedbackV1, Index> for TestClient {
    fn event(
        client: &mut Self,
        _: &zwp_linux_dmabuf_feedback_v1::ZwpLinuxDmabufFeedbackV1,
        event: zwp_linux_dmabuf_feedback_v1::Event,
        index: &Index,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use zwp_linux_dmabuf_feedback_v1::Event;
        let feedbacks = &mut client.feedback;
        let (Some(building), Some(tranche)) = (
            feedbacks.building.get_mut(index.0),
            feedbacks.tranche.get_mut(index.0),
        ) else {
            return;
        };
        match event {
            Event::FormatTable { fd, size } => building.table = read_table(fd, size),
            Event::MainDevice { device } => building.main_device = device,
            Event::TrancheTargetDevice { device } => tranche.target_device = device,
            Event::TrancheFlags { flags } => tranche.flags = flags.into(),
            Event::TrancheFormats { indices } => {
                tranche.formats = indices
                    .chunks_exact(2)
                    .filter_map(|pair| {
                        let index = u16::from_ne_bytes([pair[0], pair[1]]);
                        building.table.get(usize::from(index)).copied()
                    })
                    .collect();
            }
            Event::TrancheDone => building.tranches.push(std::mem::take(tranche)),
            Event::Done => {
                let done = std::mem::take(building);
                if let Some(complete) = feedbacks.complete.get_mut(index.0) {
                    complete.push(done);
                }
            }
            _ => {}
        }
    }
}

/// The format table: 16-byte `(fourcc, pad, modifier)` entries, native
/// endian. A short read is an empty table, which fails the assertions
/// downstream rather than panicking the client thread.
///
/// Read at offset 0, the way a real client maps it: a feedback sent twice
/// (the default, then the default again on a revert) passes the *same*
/// sealed file, whose shared file offset a plain `read` would have left at
/// the end after the first time.
fn read_table(fd: OwnedFd, size: u32) -> Vec<(u32, u64)> {
    use std::os::unix::fs::FileExt;
    let mut bytes = vec![0u8; size as usize];
    if std::fs::File::from(fd)
        .read_exact_at(&mut bytes, 0)
        .is_err()
    {
        return Vec::new();
    }
    bytes
        .chunks_exact(16)
        .filter_map(|entry| {
            Some((
                u32::from_ne_bytes(entry[0..4].try_into().ok()?),
                u64::from_ne_bytes(entry[8..16].try_into().ok()?),
            ))
        })
        .collect()
}

fn pair(code: Fourcc, modifier: Modifier) -> (u32, u64) {
    (code as u32, modifier.into())
}

/// The dev VM's primary plane, in shape: no `IN_FORMATS`, so every fourcc
/// at `Invalid` only.
fn virtio_plane() -> FormatSet {
    [Fourcc::Xrgb8888, Fourcc::Argb8888]
        .into_iter()
        .map(|code| Format {
            code,
            modifier: Modifier::Invalid,
        })
        .collect()
}

/// A plane that takes nothing the harness advertises.
fn useless_plane() -> FormatSet {
    std::iter::once(Format {
        code: Fourcc::Yuyv,
        modifier: Modifier::Invalid,
    })
    .collect()
}

impl Fixture {
    fn surface_feedback(&mut self, window: usize) {
        self.done(Step::SurfaceFeedback { window });
    }

    fn feedbacks(&mut self, window: usize) -> Vec<SeenFeedback> {
        match self.run(Step::Feedbacks { window }) {
            Ack::Feedbacks(all) => all,
            _ => panic!("expected feedbacks"),
        }
    }

    fn install(&mut self, plane: &FormatSet, planes: u64) {
        self.state
            .install_scanout_feedback(plane, DEVICE, FormatsKey { planes, lost: 0 });
    }

    /// One frame's steering, judged now, then flushed to the client.
    fn steer(&mut self) -> Steer {
        self.steer_at(Instant::now())
    }

    fn steer_at(&mut self, now: Instant) -> Steer {
        let steer = self.state.steer_now(now);
        self.settle();
        steer
    }

    fn fullscreen(&mut self, window: usize) {
        self.configured(Step::SetFullscreen {
            window,
            output: None,
        });
        self.done(Step::Draw {
            window,
            color: WINDOW_BGRA,
        });
    }
}

/// A mapped window that asked for surface feedback, and the virtio-shaped
/// scanout feedback installed: the starting point of most tests here.
fn steerable() -> Fixture {
    let mut fixture = Fixture::new();
    fixture.map(WINDOW_BGRA);
    fixture.surface_feedback(0);
    fixture.install(&virtio_plane(), 0);
    fixture
}

/// Asserts `feedback` is the scanout feedback built over `default`: the same
/// table and main device, a first tranche flagged `scanout` on [`DEVICE`]
/// holding `scanout`, and then exactly the default's own tranche.
fn assert_scanout(feedback: &SeenFeedback, default: &SeenFeedback, scanout: &[(u32, u64)]) {
    assert_eq!(feedback.table, default.table, "the default's format table");
    assert_eq!(feedback.main_device, default.main_device);
    assert_eq!(feedback.tranches.len(), 2, "{feedback:?}");
    let first = &feedback.tranches[0];
    assert_eq!(first.flags, SCANOUT);
    assert_eq!(first.target_device, DEVICE.to_ne_bytes().to_vec());
    assert_eq!(first.formats, scanout);
    assert_eq!(
        feedback.tranches[1], default.tranches[0],
        "then the main tranche"
    );
}

#[test]
fn a_covering_window_is_sent_a_scanout_tranche_once() {
    let mut fixture = steerable();
    let seen = fixture.feedbacks(0);
    assert_eq!(seen.len(), 1, "the default, on request");
    let default = seen[0].clone();
    assert_eq!(default.tranches.len(), 1);
    assert_eq!(default.tranches[0].flags, 0, "no scanout flag by default");

    // Not covering: nothing to steer.
    assert_eq!(fixture.steer(), Steer::Idle);
    fixture.fullscreen(0);
    assert_eq!(fixture.steer(), Steer::Sent);
    let seen = fixture.feedbacks(0);
    assert_eq!(seen.len(), 2);
    assert_scanout(
        &seen[1],
        &default,
        &[
            pair(Fourcc::Xrgb8888, Modifier::Linear),
            pair(Fourcc::Argb8888, Modifier::Linear),
        ],
    );

    // Frame after frame, drawn or not: nothing more.
    for _ in 0..5 {
        fixture.done(Step::Draw {
            window: 0,
            color: WINDOW_BGRA,
        });
        let _ = fixture.render();
        assert_eq!(fixture.steer(), Steer::Kept);
    }
    assert_eq!(fixture.feedbacks(0).len(), 2, "nothing is sent per frame");
}

#[test]
fn every_scanout_pair_is_one_the_default_offers_and_the_renderer_imports() {
    // The promise with teeth: a client that allocates from the scanout
    // tranche and then composites is imported like any other client.
    let mut fixture = steerable();
    fixture.fullscreen(0);
    assert_eq!(fixture.steer(), Steer::Sent);
    let seen = fixture.feedbacks(0);
    let (default, scanout) = (&seen[0], &seen[1]);
    let tranche = &scanout.tranches[0].formats;
    assert!(!tranche.is_empty(), "the test's own premise");
    let (_, output) = fixture.state.outputs.at(0).expect("an output");
    let id = fixture.state.outputs.id_of(&output).expect("an id");
    let backend = fixture.state.backends.get(&id).expect("a render target");
    for (code, modifier) in tranche {
        assert!(default.tranches[0].formats.contains(&(*code, *modifier)));
        let format = Format {
            code: Fourcc::try_from(*code).expect("a known fourcc"),
            modifier: Modifier::from(*modifier),
        };
        assert!(
            backend.imports_dmabuf_format(format),
            "{format:?} is offered for scanout and must import"
        );
    }
}

#[test]
fn leaving_fullscreen_reverts_at_once() {
    let mut fixture = steerable();
    let default = fixture.feedbacks(0)[0].clone();
    fixture.fullscreen(0);
    assert_eq!(fixture.steer(), Steer::Sent);
    fixture.configured(Step::UnsetFullscreen { window: 0 });
    fixture.done(Step::Draw {
        window: 0,
        color: WINDOW_BGRA,
    });
    assert_eq!(fixture.steer(), Steer::Reverted);
    let seen = fixture.feedbacks(0);
    assert_eq!(seen.len(), 3);
    assert_eq!(seen[2], default, "exactly the default again");
    assert_eq!(fixture.steer(), Steer::Idle);
    assert_eq!(fixture.feedbacks(0).len(), 3);
}

#[test]
fn unmapping_the_covering_window_forgets_it() {
    let mut fixture = steerable();
    fixture.fullscreen(0);
    assert_eq!(fixture.steer(), Steer::Sent);
    fixture.done(Step::Unmap { window: 0 });
    // The window is gone from the core, so nothing covers: the surface --
    // still alive, still holding a feedback object -- is told the default.
    assert_eq!(fixture.steer(), Steer::Reverted);
    assert_eq!(fixture.feedbacks(0).len(), 3);
}

#[test]
fn a_lock_holds_the_scanout_feedback_then_reverts() {
    // Locked is a transient reason with the same window still covering: the
    // client keeps its layout for `REVERT_HOLD`, and is reverted only if the
    // lock outlasts it.
    let mut fixture = steerable();
    fixture.fullscreen(0);
    assert_eq!(fixture.steer(), Steer::Sent);
    fixture.done(Step::LockSession);
    assert!(fixture.state.session_lock.is_locked());
    let start = Instant::now();
    assert_eq!(fixture.steer_at(start), Steer::Holding);
    assert_eq!(
        fixture.steer_at(start + REVERT_HOLD - Duration::from_millis(1)),
        Steer::Holding
    );
    assert_eq!(fixture.feedbacks(0).len(), 2, "nothing sent while holding");
    assert_eq!(fixture.steer_at(start + REVERT_HOLD), Steer::Reverted);
    let seen = fixture.feedbacks(0);
    assert_eq!(seen.len(), 3);
    assert_eq!(seen[2], seen[0], "reverted to the default");
    // Still locked: nothing further.
    assert_eq!(fixture.steer_at(start + REVERT_HOLD * 3), Steer::Idle);
}

#[test]
fn a_translucent_moment_shorter_than_the_hold_sends_nothing() {
    // A client fading its fullscreen window through `wp_alpha_modifier_v1`
    // and back: ineligible for a moment, eligible again before the hold ends.
    let mut fixture = steerable();
    fixture.fullscreen(0);
    assert_eq!(fixture.steer(), Steer::Sent);
    fixture.done(Step::SetAlpha {
        window: 0,
        multiplier: u32::MAX / 2,
    });
    let start = Instant::now();
    assert_eq!(fixture.steer_at(start), Steer::Holding);
    fixture.done(Step::SetAlpha {
        window: 0,
        multiplier: u32::MAX,
    });
    assert_eq!(
        fixture.steer_at(start + Duration::from_millis(500)),
        Steer::Kept
    );
    // A fresh hold starts from scratch: the earlier one does not count.
    fixture.done(Step::SetAlpha {
        window: 0,
        multiplier: u32::MAX / 2,
    });
    let again = start + REVERT_HOLD + Duration::from_millis(600);
    assert_eq!(fixture.steer_at(again), Steer::Holding);
    assert_eq!(fixture.feedbacks(0).len(), 2);
}

#[test]
fn an_overlay_surface_above_does_not_revert() {
    // Not an eligibility rule: Smithay composites the frame a notification
    // is drawn over, and the window goes direct again, in the layout this
    // feedback asked for, the moment it is gone. Reverting would make the
    // client reallocate twice per notification.
    let mut fixture = steerable();
    fixture.fullscreen(0);
    assert_eq!(fixture.steer(), Steer::Sent);
    fixture.done(Step::CreateLayer(Layer::Notification));
    assert_eq!(fixture.steer(), Steer::Kept);
    assert_eq!(fixture.feedbacks(0).len(), 2);
}

#[test]
fn a_surface_asking_after_it_went_fullscreen_gets_the_scanout_feedback_first() {
    // Smithay asks `new_surface_feedback` on a surface's first request; the
    // window already being steered is answered with the scanout feedback,
    // not the default until its eligibility next changes.
    let mut fixture = Fixture::new();
    fixture.map(WINDOW_BGRA);
    fixture.install(&virtio_plane(), 0);
    fixture.fullscreen(0);
    assert_eq!(fixture.steer(), Steer::Sent, "steered before it asked");
    fixture.surface_feedback(0);
    let seen = fixture.feedbacks(0);
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].tranches.len(), 2);
    assert_eq!(seen[0].tranches[0].flags, SCANOUT);
    assert_eq!(fixture.steer(), Steer::Kept);
    assert_eq!(fixture.feedbacks(0).len(), 1);
}

#[test]
fn a_plane_that_takes_nothing_advertised_steers_nothing() {
    let mut fixture = Fixture::new();
    fixture.map(WINDOW_BGRA);
    fixture.surface_feedback(0);
    fixture.install(&useless_plane(), 0);
    fixture.fullscreen(0);
    assert_eq!(fixture.steer(), Steer::Idle);
    assert_eq!(fixture.feedbacks(0).len(), 1, "the default, and only that");
}

#[test]
fn a_rebuilt_tranche_moves_the_current_window_onto_it() {
    // A CRTC switch (a new plane set) with the window still steered: it is
    // sent the rebuilt feedback, and the default if the new plane takes
    // nothing at all.
    let mut fixture = Fixture::new();
    fixture.map(WINDOW_BGRA);
    fixture.surface_feedback(0);
    let only_xr24: FormatSet = std::iter::once(Format {
        code: Fourcc::Xrgb8888,
        modifier: Modifier::Invalid,
    })
    .collect();
    fixture.install(&only_xr24, 0);
    fixture.fullscreen(0);
    assert_eq!(fixture.steer(), Steer::Sent);
    let default = fixture.feedbacks(0)[0].clone();
    // The opaque twin: an AR24 buffer is added as XR24, so both qualify.
    assert_scanout(
        &fixture.feedbacks(0)[1],
        &default,
        &[
            pair(Fourcc::Xrgb8888, Modifier::Linear),
            pair(Fourcc::Argb8888, Modifier::Linear),
        ],
    );
    // The same list under a new key: rebuilt, equal content, nothing sent.
    fixture.install(&only_xr24, 1);
    fixture.settle();
    assert_eq!(fixture.feedbacks(0).len(), 2);
    // A plane taking nothing: the window goes back to the default.
    fixture.install(&useless_plane(), 2);
    fixture.settle();
    let seen = fixture.feedbacks(0);
    assert_eq!(seen.len(), 3);
    assert_eq!(seen[2], default);
    assert_eq!(fixture.steer(), Steer::Idle);
}

#[test]
fn another_window_covering_moves_the_scanout_feedback_to_it() {
    let mut fixture = Fixture::new();
    fixture.map(WINDOW_BGRA);
    fixture.map(OTHER_BGRA);
    fixture.surface_feedback(0);
    fixture.surface_feedback(1);
    fixture.install(&virtio_plane(), 0);
    fixture.fullscreen(1);
    assert_eq!(fixture.steer(), Steer::Sent);
    assert_eq!(fixture.feedbacks(1).len(), 2);
    assert_eq!(
        fixture.feedbacks(0).len(),
        1,
        "the other window is untouched"
    );
    // Hand fullscreen from one window to the other between two frames: one
    // steering reverts the old and sends the new.
    fixture.configured(Step::UnsetFullscreen { window: 1 });
    let first_window = fixture.id(0);
    assert!(
        fixture
            .state
            .act(scoot_core::Action::FocusWindowId(first_window))
    );
    fixture.settle();
    fixture.fullscreen(0);
    assert_eq!(fixture.steer(), Steer::Sent);
    let first = fixture.feedbacks(0);
    let second = fixture.feedbacks(1);
    assert_eq!(first.len(), 2);
    assert_eq!(first[1].tranches[0].flags, SCANOUT);
    assert_eq!(second.len(), 3);
    assert_eq!(second[2], second[0], "the old window is reverted");
}

/// Prints what the two per-frame calls the scanout tier makes cost on the
/// steady states a fullscreen session sits in: the cache check plus a
/// `Kept` steer (eligible, target unchanged), and a `Holding` steer (the
/// one arm that reads the clock). Run by hand:
///
/// ```text
/// cargo test --release -p scoot --bin scoot --features gpu-scanout steering_cost -- --ignored --nocapture
/// ```
#[test]
#[ignore = "prints per-frame timings for a human; asserts nothing"]
fn steering_cost() {
    use crate::compositor::tty::scanout::ScanoutFormats;

    const ROUNDS: u32 = 200_000;
    let mut fixture = steerable();
    fixture.fullscreen(0);
    assert_eq!(fixture.steer(), Steer::Sent);
    let output = fixture.state.outputs.primary_id().expect("an output");
    let key = FormatsKey { planes: 0, lost: 0 };
    for (label, eligible) in [("kept (eligible)", true), ("holding (ineligible)", false)] {
        let now = Instant::now();
        let started = Instant::now();
        for _ in 0..ROUNDS {
            let state = &mut fixture.state;
            state.scanout_feedback.refresh(
                output,
                key,
                state.dmabuf_default.as_ref(),
                || -> ScanoutFormats<'_> { unreachable!("the key never moves here") },
            );
            let steer = state.steer_scanout_feedback(output, eligible, || now);
            std::hint::black_box(steer);
        }
        let each = started.elapsed() / ROUNDS;
        println!("scanout steering, {label}: {each:?} per frame ({ROUNDS} rounds)");
    }
    assert_eq!(
        fixture.feedbacks(0).len(),
        2,
        "nothing was sent while measuring"
    );
}
