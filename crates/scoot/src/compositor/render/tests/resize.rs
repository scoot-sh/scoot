//! A resize under each renderer: what the target draws afterwards, what it
//! keeps, and what a failed one leaves behind.
//!
//! The subject is `Backend::resize_in_place` and the `State::resize_output`
//! path around it. Under GLES a resize reallocates the renderbuffer on the
//! renderer the backend already has; under pixman it builds a new backend,
//! as it always has. Either way the frame drawn afterwards has to be the one
//! a brand-new backend at that size would draw -- byte for byte, since
//! nothing here blends, scales or filters.
//!
//! The GLES tests name their renderer rather than reading
//! `SCOOT_TEST_RENDERER`, so the in-place path is pinned in the default run
//! too. Like the rest of this module's GLES tests they need a working EGL
//! and fail rather than skip without one (see the parent module's doc).

use scoot_core::{Event as CoreEvent, WindowId, WindowInfo};
use smithay::output::Mode;

use super::*;
use crate::compositor::decorations::Appearance;
use crate::compositor::test_support::{Harness, capture_logs};

/// Where the scene is drawn from, before any resize.
const START: (i32, i32) = (64, 64);

/// Grow, shrink, both non-square, then back to where it began: every
/// direction a drag can take, and a return to a size the backend has held.
const SIZES: [(i32, i32); 4] = [(96, 40), (40, 72), (130, 130), START];

/// Moves `output`'s current mode, as `State::resize_output`'s `set_mode`
/// does before it resizes anything -- the damage tracker reads the output's
/// mode for its size, so a backend resized without this would draw at the
/// old one.
fn set_output_mode(output: &Output, (width, height): (i32, i32)) {
    output.change_current_state(
        Some(Mode {
            size: (width, height).into(),
            refresh: 60_000,
        }),
        None,
        None,
        None,
    );
}

/// Draws a scene laid out against `size` -- green over the top half, red
/// over the bottom, a blue block in the bottom-right quarter -- through the
/// backend's own damage tracker into its own target, and reads it back.
///
/// Every edge sits at a fraction of the size, so a target left at the old
/// size, drawn at the wrong one, flipped or mirrored all show as different
/// bytes from a fresh backend's.
fn draw_scene(backend: &mut Backend, (width, height): (i32, i32)) -> Vec<u8> {
    let half = height / 2;
    let top = SolidColorBuffer::new((width, half), [0.0, 1.0, 0.0, 1.0]);
    let bottom = SolidColorBuffer::new((width, height - half), [1.0, 0.0, 0.0, 1.0]);
    let block = SolidColorBuffer::new((width / 4, height / 4), [0.0, 0.0, 1.0, 1.0]);
    let at = (width - width / 4, height - height / 4);
    let elements = [
        SolidColorRenderElement::from_buffer(&block, at, 1.0, 1.0, Kind::Unspecified),
        SolidColorRenderElement::from_buffer(&top, (0, 0), 1.0, 1.0, Kind::Unspecified),
        SolidColorRenderElement::from_buffer(&bottom, (0, half), 1.0, 1.0, Kind::Unspecified),
    ];
    {
        let Backend {
            pipeline, damage, ..
        } = &mut *backend;
        let clear = [0.0, 0.0, 0.0, 1.0];
        match pipeline {
            Pipeline::Pixman(cpu) => {
                let mut framebuffer = cpu.renderer.bind(&mut cpu.image).expect("a framebuffer");
                damage
                    .render_output(&mut cpu.renderer, &mut framebuffer, 0, &elements, clear)
                    .expect("a rendered frame");
            }
            Pipeline::Gles(gpu) => {
                let mut framebuffer = gpu.renderer.bind(&mut gpu.buffer).expect("a framebuffer");
                damage
                    .render_output(&mut gpu.renderer, &mut framebuffer, 0, &elements, clear)
                    .expect("a rendered frame");
            }
            #[cfg(feature = "gpu-scanout")]
            Pipeline::Scanout(_) => unreachable!("no test builds a scanout pipeline"),
        }
    }
    let pixels = backend
        .capture(<[u8]>::to_vec)
        .expect("the framebuffer reads back");
    assert_eq!(
        pixels.len(),
        (width * height * 4) as usize,
        "{width}x{height}"
    );
    pixels
}

fn gles_backend(output: &Output, (width, height): (i32, i32)) -> Backend {
    Backend::new(
        output,
        width,
        height,
        RendererKind::Gles,
        ScanoutHandoff::default(),
        None,
    )
    .expect("a GLES backend")
}

/// How many pixels of two equal-length frames differ.
fn differing(left: &[u8], right: &[u8]) -> usize {
    assert_eq!(left.len(), right.len(), "the two frames are the same size");
    left.chunks_exact(4)
        .zip(right.chunks_exact(4))
        .filter(|(a, b)| a != b)
        .count()
}

/// The core of the change, on real pixels: a GLES target resized in place,
/// through every direction in [`SIZES`], draws exactly what a GLES backend
/// built new at each size draws -- and does it on the renderer it started
/// with, which is the half a rebuild would also pass.
#[test]
fn a_gles_target_resized_in_place_draws_what_a_new_one_does() {
    let output = test_output(START.0, START.1);
    let mut backend = gles_backend(&output, START);
    let context = backend.gles_context_for_test();
    draw_scene(&mut backend, START);
    for size in SIZES {
        set_output_mode(&output, size);
        assert!(
            matches!(
                backend.resize_in_place(&output, size.0, size.1),
                InPlace::Resized
            ),
            "{size:?}: a GLES target resizes in place"
        );
        assert_eq!(backend.size(), size);
        let resized = draw_scene(&mut backend, size);
        let mut fresh = gles_backend(&output, size);
        let expected = draw_scene(&mut fresh, size);
        assert_eq!(
            differing(&resized, &expected),
            0,
            "{size:?}: the resized target and a new one at that size disagree"
        );
        assert!(
            backend.gles_context_for_test() == context,
            "{size:?}: the resize kept its renderer"
        );
    }
}

/// What the dma-buf feedback and every imported client texture rest on: a
/// resize changes none of the device, the render node and the renderer
/// context. The context is the strongest of the three -- a client texture is
/// cached per context, so the same one means nothing is re-imported (the
/// dumb-buffer suite, `dmabuf/tests/layouts.rs`, pins that on a real
/// client's texture).
#[test]
fn a_gles_resize_keeps_the_device_the_render_node_and_the_context() {
    let output = test_output(START.0, START.1);
    let mut backend = gles_backend(&output, START);
    let device = backend.gles_device();
    let node = backend.render_node();
    let context = backend.gles_context_for_test();
    assert!(device.is_some() && context.is_some(), "a GLES backend");
    for size in SIZES {
        set_output_mode(&output, size);
        assert!(matches!(
            backend.resize_in_place(&output, size.0, size.1),
            InPlace::Resized
        ));
        assert_eq!(backend.gles_device(), device, "{size:?}: the device moved");
        assert_eq!(
            backend.render_node(),
            node,
            "{size:?}: the render node moved"
        );
        assert!(
            backend.gles_context_for_test() == context,
            "{size:?}: the renderer context changed"
        );
    }
}

/// The limit a resize is checked against up front is the driver's own,
/// exactly: a target at the limit on either axis resizes in place, one past
/// it is refused as [`InPlace::TooLarge`] with nothing allocated and nothing
/// changed -- and the context is the same one throughout, so no rebuild was
/// tried. Too strict a limit would fail the first half, too lax the second.
/// The at-limit targets are one pixel thick, so a few tens of KiB apiece.
#[test]
fn a_gles_resize_is_checked_against_the_drivers_limit_up_front() {
    let output = test_output(START.0, START.1);
    let mut backend = gles_backend(&output, START);
    let before = draw_scene(&mut backend, START);
    let context = backend.gles_context_for_test();
    let renderbuffer = backend
        .gles_max_renderbuffer_size_for_test()
        .expect("a GLES backend answers");
    let (max_width, max_height) = backend
        .gles_max_target_for_test()
        .expect("the driver reports its limits");
    assert!(
        max_width <= renderbuffer && max_height <= renderbuffer,
        "the limit ({max_width}x{max_height}) is within GL_MAX_RENDERBUFFER_SIZE ({renderbuffer})"
    );

    for size in [(max_width, 1), (1, max_height)] {
        set_output_mode(&output, size);
        assert!(
            matches!(
                backend.resize_in_place(&output, size.0, size.1),
                InPlace::Resized
            ),
            "{size:?}: a target at the limit resizes in place"
        );
    }
    set_output_mode(&output, START);
    assert!(matches!(
        backend.resize_in_place(&output, START.0, START.1),
        InPlace::Resized
    ));
    let at_start = draw_scene(&mut backend, START);
    assert_eq!(differing(&before, &at_start), 0, "back at the start size");

    for size in [
        (max_width + 1, 1),
        (1, max_height + 1),
        (i32::MAX, i32::MAX),
    ] {
        let InPlace::TooLarge(limit) = backend.resize_in_place(&output, size.0, size.1) else {
            panic!("{size:?} must be refused up front (limit {max_width}x{max_height})");
        };
        assert_eq!(limit, (max_width, max_height), "{size:?}: the limit named");
        assert_eq!(
            backend.size(),
            START,
            "{size:?}: a refused resize moves nothing"
        );
        let unchanged = backend
            .capture(<[u8]>::to_vec)
            .expect("the old target still reads back");
        assert_eq!(differing(&before, &unchanged), 0, "{size:?}: the old frame");
    }
    assert!(
        backend.gles_context_for_test() == context,
        "no resize here replaced the renderer"
    );
    let again = draw_scene(&mut backend, START);
    assert_eq!(differing(&before, &again), 0, "the old target still draws");
}

/// The all-or-nothing half of `GlesBackend::resize` itself, below the
/// up-front check (which is what a failure that is not about size reaches,
/// a lost context say): a reallocation the driver refuses leaves the old
/// target, at the old size, holding its frame and still drawing. One past
/// `GL_MAX_RENDERBUFFER_SIZE` is the refusal every driver makes, and only
/// the bind check in `gles::target` catches it -- `glRenderbufferStorage`
/// reports it as a GL error flag and hands back a renderbuffer regardless.
#[test]
fn a_failed_reallocation_leaves_the_old_target_drawing() {
    let output = test_output(START.0, START.1);
    let mut backend = gles_backend(&output, START);
    let before = draw_scene(&mut backend, START);
    let max = backend
        .gles_max_renderbuffer_size_for_test()
        .expect("a GLES backend answers");
    let too_wide = max.checked_add(1).expect("a finite limit");
    let context = backend.gles_context_for_test();
    let Pipeline::Gles(gpu) = &mut backend.pipeline else {
        unreachable!("a GLES backend");
    };
    let error = gpu
        .resize(too_wide, 1)
        .expect_err("one past GL_MAX_RENDERBUFFER_SIZE must be refused");
    eprintln!("refused {too_wide}x1 as expected: {error}");
    assert_eq!(backend.size(), START, "a refused resize moves nothing");
    assert!(backend.gles_context_for_test() == context);
    // The frame it held, read back as it was -- through `copy_framebuffer`,
    // which has to cope with the GL error the refused storage left set.
    let after = backend
        .capture(<[u8]>::to_vec)
        .expect("the old target still reads back");
    assert_eq!(differing(&before, &after), 0, "the old frame survived");
    // ...and it still draws.
    let again = draw_scene(&mut backend, START);
    assert_eq!(differing(&before, &again), 0, "the old target still draws");
}

/// pixman has no in-place path and is not given one (its whole backend
/// rebuilds in microseconds): asked, it says so and changes nothing, which
/// is what sends `resize_output` to [`Backend::new`] exactly as before.
#[test]
fn pixman_is_not_resized_in_place() {
    let output = test_output(START.0, START.1);
    let mut backend = Backend::new(
        &output,
        START.0,
        START.1,
        RendererKind::Pixman,
        ScanoutHandoff::default(),
        None,
    )
    .expect("a pixman backend");
    assert!(matches!(
        backend.resize_in_place(&output, 96, 40),
        InPlace::Unsupported
    ));
    assert_eq!(backend.size(), START);
}

/// A live session with `windows` windows in the core -- so every frame
/// draws their focus rings, laid out against the output's size -- on
/// `renderer`, at `canvas` square.
fn ring_scene(renderer: RendererKind, canvas: i32, windows: u64) -> Harness<(), ()> {
    let mut fixture = Harness::headless_on(Appearance::default(), canvas, renderer);
    for index in 0..windows {
        fixture.state.world.handle_event(CoreEvent::WindowOpened {
            id: WindowId(index + 1),
            info: WindowInfo {
                app_id: "resize".to_string(),
                title: "resize".to_string(),
                hints: Default::default(),
            },
            output: None,
            focus: true,
        });
    }
    fixture
}

/// Renders a frame and reads the primary output's whole target back, at
/// whatever size it is now (the harness's own read-back insists on the
/// canvas it started at).
fn render_and_read(fixture: &mut Harness<(), ()>) -> Vec<u8> {
    fixture.state.request_render();
    fixture.state.render();
    let id = fixture.state.outputs.primary_id().expect("an output");
    let backend = fixture.state.backends.get_mut(&id).expect("a backend");
    backend
        .capture(<[u8]>::to_vec)
        .expect("the framebuffer reads back")
}

/// The whole path, `State::resize_output` and the frame after it, under both
/// renderers: at each size the resized session draws what the *same*
/// session draws with a brand-new backend swapped in at that size. The same
/// session, not a second one started at that size, so the comparison is of
/// render targets alone -- the core's arrangement is whatever it is either
/// way.
#[test]
fn a_resized_session_draws_what_a_new_backend_would() {
    for renderer in [RendererKind::Pixman, RendererKind::Gles] {
        let mut fixture = ring_scene(renderer, START.0, 3);
        let id = fixture.state.outputs.primary_id().expect("an output");
        let first = render_and_read(&mut fixture);
        assert!(
            first.chunks_exact(4).any(|pixel| pixel != &first[..4]),
            "{renderer}: the scene draws something to compare"
        );
        let context = fixture.state.backends[&id].gles_context_for_test();
        for (width, height) in SIZES {
            assert!(
                fixture.state.resize_output(width, height),
                "{renderer}: a resize to {width}x{height}"
            );
            assert_eq!(fixture.state.backends[&id].size(), (width, height));
            let resized = render_and_read(&mut fixture);
            assert_eq!(resized.len(), (width * height * 4) as usize);

            let output = fixture.state.outputs.get(id).expect("the output").clone();
            let fresh = Backend::new(
                &output,
                width,
                height,
                renderer,
                ScanoutHandoff::default(),
                fixture.state.gles_device(),
            )
            .expect("a new backend at this size");
            let kept = fixture
                .state
                .backends
                .insert(id, fresh)
                .expect("the resized backend");
            let expected = render_and_read(&mut fixture);
            fixture.state.backends.insert(id, kept);
            assert_eq!(
                differing(&resized, &expected),
                0,
                "{renderer} at {width}x{height}: the resized session and a new backend disagree"
            );
            // Under GLES the backend is the one it started as, never a
            // rebuild that happened to draw the same; pixman has no context
            // and rebuilds by design.
            assert!(
                fixture.state.backends[&id].gles_context_for_test() == context,
                "{renderer} at {width}x{height}: the renderer was replaced"
            );
        }
    }
}

/// A GLES resize to a size over the driver's limit fails the way any failed
/// resize does -- `false`, the old mode put back and the refused one taken
/// out of the mode list, the old target at the old size still holding and
/// drawing its frame -- and does it up front: no in-place allocation, no
/// fallback rebuild (the context is the one it started with, and nothing
/// logs building or failing to build another renderer), and exactly one
/// WARN, naming the limit.
///
/// Only the `resize_output` call is inside [`capture_logs`]: the fixture's
/// own first build logs "built another GLES renderer" whenever an earlier
/// test in the same process built one first (the once-per-process INFO
/// line, see `gles::FIRST_BUILD_LOGGED`), which is not the rebuild this
/// asserts never happens.
#[test]
fn a_refused_gles_resize_leaves_the_session_as_it_was() {
    let mut fixture = ring_scene(RendererKind::Gles, START.0, 3);
    let (id, output) = fixture
        .state
        .outputs
        .primary_entry()
        .map(|(id, output)| (id, output.clone()))
        .expect("an output");
    let before = render_and_read(&mut fixture);
    let backend = fixture.state.backends.get_mut(&id).expect("a backend");
    let (max_width, _) = backend
        .gles_max_target_for_test()
        .expect("the driver reports its limits");
    let context = backend.gles_context_for_test();
    let mode = output.current_mode();
    let too_wide = max_width.checked_add(1).expect("a finite limit");

    let (resized, logs) = capture_logs(|| fixture.state.resize_output(too_wide, 16));
    assert!(
        !resized,
        "a {too_wide}x16 target (limit {max_width}) must be refused"
    );
    assert_eq!(output.current_mode(), mode, "the old mode is put back");
    assert!(
        !output
            .modes()
            .iter()
            .any(|mode| mode.size == (too_wide, 16).into()),
        "the refused size is not left in the mode list"
    );
    let backend = fixture.state.backends.get_mut(&id).expect("a backend");
    assert_eq!(backend.size(), START);
    assert!(backend.gles_context_for_test() == context, "not replaced");
    let unchanged = backend
        .capture(<[u8]>::to_vec)
        .expect("the old target still reads back");
    assert_eq!(differing(&before, &unchanged), 0, "the old frame survived");
    let again = render_and_read(&mut fixture);
    assert_eq!(differing(&before, &again), 0, "the old target still draws");

    assert!(
        logs.contains("is larger than the GPU can render into"),
        "the refusal names the limit: {logs}"
    );
    for never in [
        "rebuilding it",
        "the GLES renderer is up",
        "built another GLES renderer",
        "could not rebuild the GLES renderer",
    ] {
        assert!(
            !logs.contains(never),
            "no rebuild was tried ({never:?}): {logs}"
        );
    }
    assert_eq!(
        logs.lines().filter(|line| line.contains(" WARN ")).count(),
        1,
        "one WARN for the refused resize: {logs}"
    );
}
