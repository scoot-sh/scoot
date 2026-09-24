//! What a capture leaves allocated on the GLES renderer: nothing, once it
//! has returned.
//!
//! Smithay's `GlesRenderer` never deletes a GL object when its handle drops.
//! `Drop` sends it down a cleanup queue, and only `GlesRenderer::cleanup`
//! empties that queue: a frame's `finish`, `unbind`, `cleanup_texture_cache`
//! and `invalidate_caches` at the pinned rev. A capture makes two such
//! objects. One is the pixel-pack buffer `copy_framebuffer` reads into,
//! which is the size of the region, so a whole frame for a screenshot. The
//! other is the framebuffer object `bind` makes for a renderbuffer target. A
//! capture of a screen that is not redrawing reaches none of those drains,
//! so each one used to keep a whole frame of memory until the next frame
//! was drawn, and on a static screen there is no next frame. That was
//! 6.25 MB per `scootctl screenshot` at 1600x1000 on the dev VM
//! (`docs/backlog/resolved/gles-capture-leaks-a-frame-per-shot-done.md`).
//!
//! Measured as live GL object names ([`LiveGlObjects`]) rather than as
//! memory: deterministic, blind to the allocator's high-water mark, and a
//! count of the queue itself. The first test checks that the probe can see
//! a queued object at all, so the others cannot pass by counting nothing.
//!
//! Like the rest of this module's GLES tests these name their renderer and
//! need a working EGL, and fail rather than skip without one (see the parent
//! module's doc).

use smithay::backend::renderer::ExportMem;

use super::*;
use crate::compositor::decorations::Appearance;
use crate::compositor::test_support::Harness;

/// How many captures each test takes of the unchanged frame. Any number
/// above one shows growth; this many makes a per-capture leak unmistakable
/// next to the one or two objects a frame itself may leave queued.
const CAPTURES: u32 = 12;

/// A theme name nothing resolves, so the pointer a capture draws in is this
/// compositor's own arrow whatever the machine has installed (see
/// `cursor/tests.rs`'s `NO_THEME`).
const NO_THEME: &str = "scoot-test-no-such-theme";

/// A GLES backend at `MARKER_CANVAS` square with one frame already drawn
/// into it, and nothing drawn after: the static screen an agent polls.
fn drawn_gles_backend() -> Backend {
    let output = test_output(MARKER_CANVAS, MARKER_CANVAS);
    let mut backend = Backend::new(
        &output,
        MARKER_CANVAS,
        MARKER_CANVAS,
        RendererKind::Gles,
        ScanoutHandoff::default(),
        None,
    )
    .expect("a GLES backend");
    let Backend {
        pipeline, damage, ..
    } = &mut backend;
    let Pipeline::Gles(gpu) = pipeline else {
        panic!("asked for a GLES backend and got another pipeline");
    };
    draw_markers(&mut gpu.renderer, &mut gpu.buffer, damage);
    backend
}

fn live(backend: &mut Backend) -> LiveGlObjects {
    backend
        .gles_live_objects_for_test()
        .expect("a GLES backend")
}

/// Asserts `backend`'s cleanup queue is empty: draining it now deletes
/// nothing. Stronger than "no growth", which a path that leaves the same
/// one frame queued after every capture would pass.
fn assert_nothing_queued(backend: &mut Backend, what: &str) {
    let now = live(backend);
    backend.cleanup_texture_cache().expect("a drain");
    assert_eq!(live(backend), now, "{what}: GL objects were still queued");
}

/// The premise, pinned against the pinned Smithay rev: a read-back's
/// pixel-pack buffer is live while its mapping is held, *stays* live once
/// the mapping drops, and goes only when the cleanup queue is drained.
///
/// This is also what shows the probe can see a queued object. Without it,
/// every "nothing left behind" assertion below could pass by counting
/// nothing. If a Smithay bump starts deleting on drop, the middle assertion
/// fails, and the drain on the capture path can be reconsidered then.
#[test]
fn a_dropped_read_back_buffer_stays_allocated_until_the_queue_is_drained() {
    let mut backend = drawn_gles_backend();
    let Pipeline::Gles(gpu) = &mut backend.pipeline else {
        unreachable!("drawn_gles_backend builds gles");
    };
    Renderer::cleanup_texture_cache(&mut gpu.renderer).expect("a drain");
    let empty = LiveGlObjects::of(&mut gpu.renderer);

    let region = Rectangle::from_size((MARKER_CANVAS, MARKER_CANVAS).into());
    let framebuffer = gpu.renderer.bind(&mut gpu.buffer).expect("a framebuffer");
    let mapping = gpu
        .renderer
        .copy_framebuffer(&framebuffer, region, Fourcc::Argb8888)
        .expect("a read-back");
    drop(framebuffer);
    let held = LiveGlObjects::of(&mut gpu.renderer);
    assert_eq!(
        held,
        LiveGlObjects {
            buffers: empty.buffers + 1,
            framebuffers: empty.framebuffers + 1,
        },
        "the probe sees the held pixel-pack buffer and the bind's framebuffer object"
    );

    drop(mapping);
    assert_eq!(
        LiveGlObjects::of(&mut gpu.renderer),
        held,
        "dropping the mapping only queues its buffer"
    );

    Renderer::cleanup_texture_cache(&mut gpu.renderer).expect("a drain");
    assert_eq!(
        LiveGlObjects::of(&mut gpu.renderer),
        empty,
        "a drain deletes both"
    );
}

/// [`Backend::capture`], the funnel for both IPC screenshots and
/// `ext-image-copy-capture-v1`, taken over and over of a frame nothing
/// redraws, holds no more GL objects after the last capture than after the
/// first. Before the drain it held one more pixel-pack buffer and one more
/// framebuffer object per capture.
#[test]
fn repeated_captures_of_an_unchanged_frame_leave_no_gl_objects_behind() {
    let mut backend = drawn_gles_backend();
    let first = backend
        .capture(<[u8]>::to_vec)
        .expect("the framebuffer reads back");
    let settled = live(&mut backend);
    for _ in 0..CAPTURES {
        let again = backend
            .capture(<[u8]>::to_vec)
            .expect("the framebuffer reads back");
        assert_eq!(again, first, "nothing redrew, so every capture matches");
    }
    assert_eq!(
        live(&mut backend),
        settled,
        "{CAPTURES} captures of an unchanged frame must not leave GL objects queued"
    );
    assert_nothing_queued(&mut backend, "after the last capture");
}

/// The same through the whole IPC path, `State::capture_pixels_for`, with
/// and without the pointer. With the pointer, a headless frame never holds
/// the cursor, so every capture also re-renders the cursor's region into
/// the capture pool's own target (`render::capture_cursor`) and reads that
/// back as well: a second read-back per capture, on a second target.
#[test]
fn screenshots_of_a_static_screen_leave_no_gl_objects_behind() {
    let appearance = Appearance {
        cursor_theme: Some(NO_THEME.to_owned()),
        ..Appearance::default()
    };
    let mut fixture: Harness<(), ()> = Harness::headless_on(appearance, 48, RendererKind::Gles);
    let id = fixture.state.outputs.primary_id().expect("an output");
    fixture.state.pointer_move(10.0, 10.0);
    for cursor in [false, true] {
        let shot = |fixture: &mut Harness<(), ()>| {
            fixture
                .state
                .capture_pixels_for(Some(id), cursor)
                .expect("a screenshot")
                .bgra
        };
        let first = shot(&mut fixture);
        let settled = live(fixture.state.backends.get_mut(&id).expect("a backend"));
        for _ in 0..CAPTURES {
            assert_eq!(shot(&mut fixture), first, "cursor {cursor}: nothing moved");
        }
        assert_eq!(
            live(fixture.state.backends.get_mut(&id).expect("a backend")),
            settled,
            "cursor {cursor}: {CAPTURES} screenshots of a static screen must not \
             leave GL objects queued"
        );
        assert_nothing_queued(
            fixture.state.backends.get_mut(&id).expect("a backend"),
            &format!("cursor {cursor}: after the last screenshot"),
        );
    }
    assert_eq!(
        fixture
            .state
            .backends
            .get(&id)
            .expect("a backend")
            .patch_targets_built(),
        1,
        "the pointer's region really was re-rendered, into one reused target"
    );
}
