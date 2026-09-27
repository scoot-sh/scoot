//! The capture read-back pools: one whole-frame buffer per output, reused
//! capture after capture instead of allocated and faulted fresh each time.
//!
//! Two halves, pinned separately. The read-back half: `Backend::capture_into`
//! fills a caller-provided `Vec` (reusing its capacity) with exactly the
//! bytes the borrowed [`Backend::capture`] hands out, on either renderer --
//! so byte-equality between the two is the correctness of the pooling, and
//! capacity/pointer stability across repeated captures is the pooling
//! itself. The worker half (an encode's input `Vec` returning to the pool
//! with its reply) is pinned where the worker lives,
//! `ipc::connection::tests`, through a real IPC screenshot.
//!
//! Like the rest of this module's renderer-naming tests these need a working
//! EGL for the GLES arms, and fail rather than skip without one (see the
//! parent module's doc).

use super::*;
use crate::compositor::screenshot::MAX_IN_FLIGHT_SHOTS;

/// A drawn backend at `size`, on `renderer`: the marker scene, so the bytes
/// have something to say.
fn drawn_backend(size: (i32, i32), renderer: RendererKind) -> Backend {
    let output = test_output(size.0, size.1);
    let mut backend = Backend::new(
        &output,
        size.0,
        size.1,
        renderer,
        ScanoutHandoff::default(),
        None,
    )
    .expect("a backend");
    let Backend {
        pipeline, damage, ..
    } = &mut backend;
    match pipeline {
        Pipeline::Pixman(cpu) => draw_markers(&mut cpu.renderer, &mut cpu.image, damage),
        Pipeline::Gles(gpu) => draw_markers(&mut gpu.renderer, &mut gpu.buffer, damage),
        #[cfg(feature = "gpu-scanout")]
        Pipeline::Scanout(_) => unreachable!("no test builds a scanout pipeline"),
    }
    backend
}

/// `capture_into` hands out exactly what `capture` does: the pooled fill is
/// the same frame, not merely the same size. And it reports the size it
/// filled.
#[test]
fn capture_into_matches_the_borrowed_capture_on_both_renderers() {
    for renderer in [RendererKind::Pixman, RendererKind::Gles] {
        let mut backend = drawn_backend((MARKER_CANVAS, MARKER_CANVAS), renderer);
        let borrowed = backend
            .capture(<[u8]>::to_vec)
            .expect("the framebuffer reads back");
        let mut buf = backend.take_capture_buf();
        let size = backend
            .capture_into(&mut buf)
            .expect("the framebuffer fills a buffer");
        assert_eq!(
            size,
            (MARKER_CANVAS, MARKER_CANVAS),
            "{renderer}: the reported size"
        );
        assert_eq!(
            buf, borrowed,
            "{renderer}: the pooled fill is the same frame"
        );
    }
}

/// The pooling itself: the second fill reuses the first fill's allocation --
/// same pointer, same capacity -- and still reads the same frame. A path
/// that allocated per capture would pass every byte-equality check above
/// while faulting fresh megabytes each time; this is what catches it.
#[test]
fn a_second_capture_into_the_same_buffer_reuses_its_allocation() {
    for renderer in [RendererKind::Pixman, RendererKind::Gles] {
        let mut backend = drawn_backend((MARKER_CANVAS, MARKER_CANVAS), renderer);
        let mut buf = backend.take_capture_buf();
        backend.capture_into(&mut buf).expect("a first fill");
        let (ptr, capacity) = (buf.as_ptr(), buf.capacity());
        assert!(
            capacity >= MARKER_CANVAS as usize * MARKER_CANVAS as usize * 4,
            "{renderer}: the first fill holds the whole frame"
        );
        let first = buf.clone();
        backend.capture_into(&mut buf).expect("a second fill");
        assert_eq!(
            buf.as_ptr(),
            ptr,
            "{renderer}: the fill reused the allocation"
        );
        assert_eq!(
            buf.capacity(),
            capacity,
            "{renderer}: the fill grew nothing"
        );
        assert_eq!(
            buf, first,
            "{renderer}: nothing redrew, so the frame matches"
        );
    }
}

/// Take and recycle round-trip: a recycled buffer comes back empty (the pool
/// keeps capacity, never pixels) and is the same allocation that went in.
/// Two buffers taken without a recycle in between are two allocations --
/// what keeps concurrent captures from sharing one.
#[test]
fn take_and_recycle_keep_empty_buffers_and_no_sharing() {
    let output = test_output(16, 16);
    let mut backend = Backend::new(
        &output,
        16,
        16,
        RendererKind::Pixman,
        ScanoutHandoff::default(),
        None,
    )
    .expect("a backend");
    assert_eq!(
        backend.capture_bufs_kept_for_test(),
        0,
        "the pool starts empty"
    );
    let mut first = backend.take_capture_buf();
    backend.capture_into(&mut first).expect("a fill");
    assert!(!first.is_empty(), "the fill wrote pixels");
    let second = backend.take_capture_buf();
    assert!(
        second.as_ptr() != first.as_ptr() || second.capacity() != first.capacity(),
        "two outstanding buffers must not be the same allocation"
    );
    backend.recycle_capture_buf(first);
    backend.recycle_capture_buf(second);
    assert_eq!(backend.capture_bufs_kept_for_test(), 2, "both came home");
    let again = backend.take_capture_buf();
    assert!(again.is_empty(), "a kept buffer holds no pixels");
}

/// The pool is bounded: past [`CAPTURE_BUFS_KEPT`] a recycled buffer is
/// dropped rather than kept, so the pool cannot grow without bound no matter
/// how many captures ran.
#[test]
fn the_pool_keeps_at_most_its_bound() {
    let output = test_output(16, 16);
    let mut backend = Backend::new(
        &output,
        16,
        16,
        RendererKind::Pixman,
        ScanoutHandoff::default(),
        None,
    )
    .expect("a backend");
    for _ in 0..CAPTURE_BUFS_KEPT + 3 {
        backend.recycle_capture_buf(vec![0u8; 16 * 16 * 4]);
    }
    assert_eq!(
        backend.capture_bufs_kept_for_test(),
        CAPTURE_BUFS_KEPT,
        "the pool keeps its bound and drops the rest"
    );
}

/// A resized output reuses its buffer: the pool is keyed on nothing but
/// capacity, so a smaller frame after a resize fills the same allocation
/// rather than allocating fresh. GLES only -- pixman has no in-place resize
/// and rebuilds its backend (and its empty pool) instead.
#[test]
fn a_resized_output_reuses_its_buffer() {
    let output = test_output(16, 16);
    let mut backend = Backend::new(
        &output,
        16,
        16,
        RendererKind::Gles,
        ScanoutHandoff::default(),
        None,
    )
    .expect("a backend");
    let mut buf = backend.take_capture_buf();
    backend.capture_into(&mut buf).expect("a 16x16 fill");
    assert_eq!(buf.len(), 16 * 16 * 4);
    let ptr = buf.as_ptr();
    backend.recycle_capture_buf(buf);
    assert!(
        matches!(backend.resize_in_place(&output, 8, 8), InPlace::Resized),
        "the test resizes in place"
    );
    let mut buf = backend.take_capture_buf();
    backend.capture_into(&mut buf).expect("an 8x8 fill");
    assert_eq!(buf.len(), 8 * 8 * 4, "the fill is the new size");
    assert_eq!(
        buf.as_ptr(),
        ptr,
        "the smaller frame reused the same allocation"
    );
}

/// The pool bound covers the global in-flight bound: every accepted capture
/// holds exactly one buffer from dispatch to its encode's return, so one
/// output -- where all four in flight may land -- never needs more stashed
/// than that. Pinned as an equality so a change to either bound revisits
/// the other.
#[test]
fn the_pool_bound_covers_the_in_flight_bound() {
    assert_eq!(
        CAPTURE_BUFS_KEPT, MAX_IN_FLIGHT_SHOTS,
        "one output must be able to stash every in-flight capture's buffer"
    );
}
