//! Tests for the renderer seam itself: the read-back's orientation contract,
//! the damage-tracker contract the `--tty` retry rests on, and the
//! damage-bbox arithmetic the presenters copy through.
//!
//! What is *not* here, on purpose: whether a frame draws the right thing.
//! That is asserted on real pixels by the suites that drive a real client
//! through a real `State` -- `session_lock`, `layer_shell`, `alpha_modifier`,
//! `single_pixel_buffer`, `cursor`, `output_scale` -- all of which read the
//! framebuffer back through [`Backend::capture`]. They are the regression net
//! for this module; duplicating them here would only pin the seam against
//! itself.

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::element::solid::{SolidColorBuffer, SolidColorRenderElement};
use smithay::backend::renderer::pixman::PixmanRenderer;
use smithay::backend::renderer::{Bind, Offscreen};
use smithay::output::{Mode, Output, PhysicalProperties, Subpixel};
use smithay::utils::Transform;

use super::*;

/// Opaque green and opaque red as the BGRA bytes a pixman `Argb8888`
/// framebuffer holds them in.
const GREEN_BGRA: [u8; 4] = [0, 255, 0, 255];
const RED_BGRA: [u8; 4] = [0, 0, 255, 255];

/// The side of the orientation test's framebuffer. Even, so the two halves
/// are the same height.
const MARKER_CANVAS: i32 = 64;

fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Physical> {
    Rectangle::new((x, y).into(), (w, h).into())
}

/// An `Output` with a real mode, which is all [`Backend::new`] needs.
fn test_output(width: i32, height: i32) -> Output {
    let output = Output::new(
        "seam-test".to_string(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "scoot".into(),
            model: "seam-test".into(),
            serial_number: "0".into(),
        },
    );
    output.change_current_state(
        Some(Mode {
            size: (width, height).into(),
            refresh: 60_000,
        }),
        Some(Transform::Normal),
        Some(smithay::output::Scale::Integer(1)),
        Some((0, 0).into()),
    );
    output
}

/// The pixel at `(x, y)` of a BGRA read-back `width` pixels across.
fn pixel(bytes: &[u8], width: i32, x: i32, y: i32) -> [u8; 4] {
    let index = ((y * width + x) * 4) as usize;
    bytes[index..index + 4]
        .try_into()
        .expect("four bytes per pixel")
}

/// The orientation contract [`read_back`]'s doc states, pinned on real
/// pixels rather than on a comment: a marker drawn at the *logical* top-left
/// comes back at the *start* of the buffer.
///
/// This is the fail-first pin for the `flipped()` trap. Smithay's
/// `PixmanMapping::flipped()` answers `false` and `GlesMapping::flipped()`
/// answers `true` for byte-identical layouts (GLES's bottom-left origin is
/// already compensated for by `flip180` in the projection), so a future
/// reader who "fixes" the read-back by reversing rows when `flipped()` is
/// set would invert the screen for everyone. Reverse the row order anywhere
/// between `render_output` and the returned slice and this test fails.
#[test]
fn the_read_back_hands_out_the_logical_top_row_first() {
    let output = test_output(MARKER_CANVAS, MARKER_CANVAS);
    let mut backend = Backend::new(&output, MARKER_CANVAS, MARKER_CANVAS).expect("a backend");
    let half = MARKER_CANVAS / 2;
    let top = SolidColorBuffer::new((MARKER_CANVAS, half), [0.0, 1.0, 0.0, 1.0]);
    let bottom = SolidColorBuffer::new((MARKER_CANVAS, half), [1.0, 0.0, 0.0, 1.0]);
    let elements = [
        SolidColorRenderElement::from_buffer(&top, (0, 0), 1.0, 1.0, Kind::Unspecified),
        SolidColorRenderElement::from_buffer(&bottom, (0, half), 1.0, 1.0, Kind::Unspecified),
    ];

    let Backend {
        pipeline: Pipeline::Pixman(cpu),
        damage,
        ..
    } = &mut backend;
    let mut framebuffer = cpu.renderer.bind(&mut cpu.image).expect("a framebuffer");
    damage
        .render_output(
            &mut cpu.renderer,
            &mut framebuffer,
            0,
            &elements,
            [0.0, 0.0, 0.0, 1.0],
        )
        .expect("a rendered frame");
    drop(framebuffer);

    let bytes = backend
        .capture(<[u8]>::to_vec)
        .expect("the framebuffer reads back");
    assert_eq!(
        pixel(&bytes, MARKER_CANVAS, 0, 0),
        GREEN_BGRA,
        "the first pixel of the buffer must be the one drawn at the logical top-left"
    );
    assert_eq!(
        pixel(&bytes, MARKER_CANVAS, 0, MARKER_CANVAS - 1),
        RED_BGRA,
        "the last row of the buffer must be the one drawn at the logical bottom"
    );
}

/// [`Backend::capture`] reads back the whole target, at the size the target
/// was built at -- which is what `screencopy.rs` advertises to clients as
/// their buffer size, and what `screenshot.rs` encodes against.
#[test]
fn a_capture_covers_the_whole_target_at_the_backends_own_size() {
    let output = test_output(40, 24);
    let mut backend = Backend::new(&output, 40, 24).expect("a backend");
    assert_eq!(backend.size(), (40, 24));
    let bytes = backend
        .capture(<[u8]>::to_vec)
        .expect("the framebuffer reads back");
    assert_eq!(
        bytes.len(),
        40 * 24 * 4,
        "a capture is four bytes per pixel of the whole target"
    );
}

#[test]
fn a_single_rect_is_its_own_bounding_box() {
    assert_eq!(union_bbox(&[rect(10, 20, 30, 40)]), rect(10, 20, 30, 40));
}

#[test]
fn disjoint_rects_bound_the_gap_between_them() {
    // An old cursor position and a new one some distance away: the bbox must
    // cover both plus whatever's between them, since a single read-back (and
    // the dumb-buffer write behind it) can only ever copy one contiguous
    // region.
    let old_position = rect(0, 0, 16, 16);
    let new_position = rect(100, 50, 16, 16);
    assert_eq!(
        union_bbox(&[old_position, new_position]),
        rect(0, 0, 116, 66)
    );
}

#[test]
fn an_overlapping_rect_does_not_grow_the_box_past_the_union() {
    let a = rect(0, 0, 20, 20);
    let b = rect(10, 10, 20, 20);
    assert_eq!(union_bbox(&[a, b]), rect(0, 0, 30, 30));
}

#[test]
fn a_rect_fully_containing_another_wins_alone() {
    let outer = rect(0, 0, 100, 100);
    let inner = rect(40, 40, 10, 10);
    assert_eq!(union_bbox(&[inner, outer]), outer);
}

/// The damage-tracker contract `Tty`'s failed-flip retry relies on, verified
/// against the pinned Smithay source rather than assumed:
/// `damage_output_internal` extends an unchanged frame's (empty) new damage
/// with `old_damage.take(age - 1)`, so age 1 asks for nothing and reports
/// `None`, while age 0 takes the full-redraw branch and reports the whole
/// output. A retry that reads as age 1 therefore presents nothing on a quiet
/// screen (the loss that fix closes); a retry at age 0 always re-presents.
#[test]
fn an_unchanged_frame_reports_no_damage_at_age_one_but_full_damage_at_age_zero() {
    let mut renderer = PixmanRenderer::new().expect("a cpu renderer");
    let mut image = renderer
        .create_buffer(Fourcc::Argb8888, (64, 64).into())
        .expect("an image");
    let mut tracker = OutputDamageTracker::new((64, 64), 1.0, Transform::Normal);
    let buffer = SolidColorBuffer::new((64, 64), [1.0, 0.0, 1.0, 1.0]);
    let element =
        SolidColorRenderElement::from_buffer(&buffer, (0, 0), 1.0, 1.0, Kind::Unspecified);
    let mut framebuffer = renderer.bind(&mut image).expect("a framebuffer");
    let first = tracker
        .render_output(
            &mut renderer,
            &mut framebuffer,
            0,
            &[element],
            [0.0, 0.0, 0.0, 1.0],
        )
        .expect("a first render");
    assert!(first.damage.is_some());
    drop(first);
    // Unchanged, at the age a failed flip's retry would read without the age
    // clear: nothing new, and no history requested either.
    let mut framebuffer = renderer.bind(&mut image).expect("a framebuffer");
    let element =
        SolidColorRenderElement::from_buffer(&buffer, (0, 0), 1.0, 1.0, Kind::Unspecified);
    let second = tracker
        .render_output(
            &mut renderer,
            &mut framebuffer,
            1,
            &[element],
            [0.0, 0.0, 0.0, 1.0],
        )
        .expect("a second render");
    assert!(second.damage.is_none());
    drop(second);
    // The same unchanged frame at age 0 -- what the cleared slot reads as --
    // redraws the whole output.
    let mut framebuffer = renderer.bind(&mut image).expect("a framebuffer");
    let element =
        SolidColorRenderElement::from_buffer(&buffer, (0, 0), 1.0, 1.0, Kind::Unspecified);
    let third = tracker
        .render_output(
            &mut renderer,
            &mut framebuffer,
            0,
            &[element],
            [0.0, 0.0, 0.0, 1.0],
        )
        .expect("a third render");
    assert!(third.damage.is_some());
}
