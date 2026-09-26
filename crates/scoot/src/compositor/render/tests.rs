//! Tests for the renderer seam itself: the read-back's orientation contract
//! (under *both* renderers, and their agreement), which renderer a session
//! resolves to, the damage-tracker contract the `--tty` retry rests on, and
//! the damage-bbox arithmetic the presenters copy through.
//!
//! What is *not* here, on purpose: whether a frame draws the right thing.
//! That is asserted on real pixels by the suites that drive a real client
//! through a real `State` -- `session_lock`, `layer_shell`, `alpha_modifier`,
//! `single_pixel_buffer`, `cursor`, `output_scale` -- all of which read the
//! framebuffer back through [`Backend::capture`], and all of which run under
//! either renderer (`SCOOT_TEST_RENDERER=gles`, see `test_support`). They are
//! the regression net for this module; duplicating them here would only pin
//! the seam against itself.
//!
//! # The GLES tests here require a working EGL, and fail rather than skip
//!
//! Deliberate. Every suite in this crate already needs a real Linux session
//! (a writable `$XDG_RUNTIME_DIR`, a bindable wayland socket), and any such
//! machine has Mesa's software EGL device even with no GPU at all -- that is
//! exactly what this project's dev VM is. A silently-skipped GLES test is
//! worse than a failing one: `cargo test` captures output, so the skip would
//! be invisible and "the suite passed" would mean nothing about the renderer
//! this stage exists to add. Nothing about the default pixman path depends on
//! EGL being there.

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

/// Draws the orientation scene -- green across the logical top half, red
/// across the bottom -- with whichever renderer `backend` is carrying, and
/// hands back the read-back bytes.
///
/// Generic below the match for the same reason [`draw_frame`] is: the arms
/// choose a renderer and nothing under them knows which one, so the two
/// renderers are measured by *identical* code rather than by two
/// hand-written scenes that could differ.
fn marker_pixels(renderer: RendererKind) -> Vec<u8> {
    let output = test_output(MARKER_CANVAS, MARKER_CANVAS);
    let mut backend = Backend::new(
        &output,
        MARKER_CANVAS,
        MARKER_CANVAS,
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
        // Unreachable: `Backend::new` only builds this variant from a
        // `ScanoutHandoff` carrying a renderer, and the one above is empty.
        // The scanout tier needs a live DRM device, so no test can build one;
        // the orientation contract it shares is pinned through the GLES arm,
        // which is the same `GlesRenderer` drawing the same elements.
        #[cfg(feature = "gpu-scanout")]
        Pipeline::Scanout(_) => unreachable!("no test builds a scanout pipeline"),
    }
    backend
        .capture(<[u8]>::to_vec)
        .expect("the framebuffer reads back")
}

fn draw_markers<R, T>(renderer: &mut R, target: &mut T, damage: &mut OutputDamageTracker)
where
    R: Renderer + Bind<T>,
{
    let half = MARKER_CANVAS / 2;
    let top = SolidColorBuffer::new((MARKER_CANVAS, half), [0.0, 1.0, 0.0, 1.0]);
    let bottom = SolidColorBuffer::new((MARKER_CANVAS, half), [1.0, 0.0, 0.0, 1.0]);
    let elements = [
        SolidColorRenderElement::from_buffer(&top, (0, 0), 1.0, 1.0, Kind::Unspecified),
        SolidColorRenderElement::from_buffer(&bottom, (0, half), 1.0, 1.0, Kind::Unspecified),
    ];
    let mut framebuffer = renderer.bind(target).expect("a framebuffer");
    damage
        .render_output(
            renderer,
            &mut framebuffer,
            0,
            &elements,
            [0.0, 0.0, 0.0, 1.0],
        )
        .expect("a rendered frame");
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
/// between `render_output` and the returned slice and this test fails --
/// under *either* renderer, which is the point of running it under both: the
/// one that answers `true` is the one such a "fix" would be written for.
#[test]
fn the_read_back_hands_out_the_logical_top_row_first() {
    for renderer in [RendererKind::Pixman, RendererKind::Gles] {
        let bytes = marker_pixels(renderer);
        assert_eq!(
            pixel(&bytes, MARKER_CANVAS, 0, 0),
            GREEN_BGRA,
            "{renderer}: the first pixel of the buffer must be the one drawn at the \
             logical top-left"
        );
        assert_eq!(
            pixel(&bytes, MARKER_CANVAS, 0, MARKER_CANVAS - 1),
            RED_BGRA,
            "{renderer}: the last row of the buffer must be the one drawn at the \
             logical bottom"
        );
    }
}

/// The stronger form of the same claim, and the one stage 2 rests on: the
/// two renderers do not merely each put their own top row first, they
/// produce the *same bytes* for the same scene -- which is what lets every
/// pixel-readback suite in the crate run unchanged under either (see this
/// module's doc).
///
/// Byte equality rather than a tolerance because this scene is opaque solid
/// colour with no blending, no scaling and no filtering: there is nothing
/// for two correct renderers to round differently. A tolerance here would
/// hide exactly the layout and channel-order bugs this exists to catch.
#[test]
fn both_renderers_lay_the_same_frame_out_the_same_way() {
    let cpu = marker_pixels(RendererKind::Pixman);
    let gpu = marker_pixels(RendererKind::Gles);
    assert_eq!(cpu.len(), gpu.len(), "both frames are the same size");
    let differing = cpu
        .chunks_exact(4)
        .zip(gpu.chunks_exact(4))
        .filter(|(left, right)| left != right)
        .count();
    assert_eq!(
        differing,
        0,
        "pixman and gles disagree on {differing} of {} pixels of an opaque two-colour frame",
        cpu.len() / 4
    );
}

/// [`Backend::capture`] reads back the whole target, at the size the target
/// was built at -- which is what `screencopy.rs` advertises to clients as
/// their buffer size, and what `screenshot.rs` encodes against. True of
/// either renderer: a capture client's buffer size cannot depend on which
/// one the session was started with.
#[test]
fn a_capture_covers_the_whole_target_at_the_backends_own_size() {
    for renderer in [RendererKind::Pixman, RendererKind::Gles] {
        let output = test_output(40, 24);
        let mut backend = Backend::new(&output, 40, 24, renderer, ScanoutHandoff::default(), None)
            .expect("a backend");
        assert_eq!(backend.size(), (40, 24), "{renderer}");
        assert_eq!(backend.renderer(), renderer, "{renderer}");
        let bytes = backend
            .capture(<[u8]>::to_vec)
            .expect("the framebuffer reads back");
        assert_eq!(
            bytes.len(),
            40 * 24 * 4,
            "{renderer}: a capture is four bytes per pixel of the whole target"
        );
    }
}

/// Which renderer a session ends up with, across the four ways of asking.
/// The flag beats the file -- including `--renderer pixman` against a file
/// asking for `gles`, which is the case a plain `RendererKind` (rather than
/// an `Option`) at either site would quietly get wrong.
#[test]
fn the_flag_beats_the_config_file_and_neither_means_pixman() {
    use RendererKind::{Gles, Pixman};
    assert_eq!(resolve(None, None, false), Pixman, "nothing named");
    assert_eq!(resolve(None, Some(Gles), false), Gles, "the file alone");
    assert_eq!(resolve(Some(Gles), None, false), Gles, "the flag alone");
    assert_eq!(
        resolve(Some(Pixman), Some(Gles), false),
        Pixman,
        "an explicit --renderer pixman must beat a file asking for gles"
    );
    assert_eq!(
        resolve(Some(Gles), Some(Pixman), false),
        Gles,
        "an explicit --renderer gles must beat a file asking for pixman"
    );
}

/// Without a scanout tier compiled in, `--tty` overrides both: the only GLES
/// pipeline that exists there reads the GPU frame back only to memcpy it into
/// a dumb buffer, which is slower than compositing on the CPU in the first
/// place. A warning and pixman, never a refusal to start -- on `--tty`, scoot
/// *is* the session.
///
/// `resolve_with` rather than `resolve` so both answers are pinned from
/// either build: which Cargo features this test binary happens to carry must
/// not decide which half of the behaviour is covered.
#[test]
fn tty_without_a_scanout_tier_keeps_pixman_however_gles_was_asked_for() {
    use RendererKind::{Gles, Pixman};
    for (flag, file) in [
        (Some(Gles), None),
        (None, Some(Gles)),
        (Some(Gles), Some(Gles)),
        (Some(Gles), Some(Pixman)),
    ] {
        let (chosen, warning) = resolve_with(flag, file, true, false);
        assert_eq!(chosen, Pixman, "{flag:?} / {file:?}");
        assert!(
            warning.is_some_and(|text| text.contains("gpu-scanout")),
            "the refusal must name the missing Cargo feature, not just say no"
        );
    }
    // ...and asking for nothing under --tty is still the same default, not a
    // second code path, and warns about nothing.
    assert_eq!(resolve_with(None, None, true, false), (Pixman, None));
}

/// With the scanout tier compiled in, `--tty --renderer gles` is a real
/// choice and resolves to `gles` -- silently, because there is nothing to
/// warn about. Whether the *device* can actually drive it is `tty::init`'s
/// question, not this one's, and it falls back there with its own distinct
/// wording.
#[test]
fn tty_with_a_scanout_tier_honours_gles() {
    use RendererKind::{Gles, Pixman};
    assert_eq!(resolve_with(Some(Gles), None, true, true), (Gles, None));
    assert_eq!(resolve_with(None, Some(Gles), true, true), (Gles, None));
    // An explicit `--renderer pixman` still beats a file asking for gles,
    // under `--tty` exactly as anywhere else.
    assert_eq!(
        resolve_with(Some(Pixman), Some(Gles), true, true),
        (Pixman, None)
    );
    // And the default is still pixman: having the tier available does not
    // make it the default.
    assert_eq!(resolve_with(None, None, true, true), (Pixman, None));
}

/// The scanout tier is a `--tty` thing only: `--headless`/`--nested` are
/// unaffected by whether the feature is compiled in, in either direction.
#[test]
fn the_scanout_feature_changes_nothing_off_tty() {
    use RendererKind::{Gles, Pixman};
    for available in [false, true] {
        assert_eq!(
            resolve_with(Some(Gles), None, false, available),
            (Gles, None)
        );
        assert_eq!(resolve_with(None, None, false, available), (Pixman, None));
    }
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

/// The desync `dumb-tier-damage-history-desync.md` records, pinned through
/// the same decision the frame path gates `advance_generation` on: three
/// identical frames must report `Some`, `None`, then still `None`. The
/// empty-damage frame freezes Smithay's history (the early return runs
/// before `old_damage.push_front` -- the contract test above pins the
/// `None`-at-1 half of that), so scoot's generation must freeze with it:
/// the third frame runs at the same age as the second. Advancing
/// unconditionally (the old `draw_frame_with` behaviour) puts the third
/// frame at age 2 against a one-entry history, and the tracker answers a
/// full repaint instead.
///
/// The ages below are what `BufferPool` hands these frames under the gate --
/// 0 (never written), 1 (written by the first render), 1 again (the second
/// reported no damage, so no advance) -- per `buffers.rs`'s `age` pins, not
/// invented here. The advance in the loop goes through the production
/// `history_advanced`, not a copy of its expression.
/// Neuter check: make `history_advanced` return `true` unconditionally and
/// the third frame runs at age 2 and reports the whole 64x64 output, not
/// `None`.
#[test]
fn an_empty_damage_frame_freezes_history_so_the_next_identical_frame_is_still_empty() {
    let mut renderer = PixmanRenderer::new().expect("a cpu renderer");
    let mut image = renderer
        .create_buffer(Fourcc::Argb8888, (64, 64).into())
        .expect("an image");
    let mut tracker = OutputDamageTracker::new((64, 64), 1.0, Transform::Normal);
    let buffer = SolidColorBuffer::new((64, 64), [1.0, 0.0, 1.0, 1.0]);
    // scoot's own generation, advanced only through the gate the frame path
    // uses; `last_written` mirrors one slot written by every damaging
    // render, the way `write_region` records it. The two are deliberately
    // separate arms, as they are in production: the advance models
    // `draw_frame_with`'s gate (the fix under test), while the record
    // models `present`/`write_region` running only for a damaging render --
    // a `None` frame advances nothing anywhere, but an unconditional advance
    // (the bug) moves the generation while leaving `last_written` behind.
    let mut generation = 0u64;
    let mut last_written = None;
    for (frame, expect_damage) in [true, false, false].into_iter().enumerate() {
        let age = last_written.map_or(0, |last| (generation - last + 1) as usize);
        let mut framebuffer = renderer.bind(&mut image).expect("a framebuffer");
        let element =
            SolidColorRenderElement::from_buffer(&buffer, (0, 0), 1.0, 1.0, Kind::Unspecified);
        let rendered = tracker
            .render_output(
                &mut renderer,
                &mut framebuffer,
                age,
                &[element],
                [0.0, 0.0, 0.0, 1.0],
            )
            .expect("a render");
        assert_eq!(
            rendered.damage.is_some(),
            expect_damage,
            "frame {frame} at age {age}: expected damage-{expect_damage}, got {:?}",
            rendered.damage,
        );
        if history_advanced(rendered.damage) {
            generation += 1;
        }
        if rendered.damage.is_some() {
            last_written = Some(generation);
        }
        drop(rendered);
    }
}

#[cfg(feature = "gpu-scanout")]
#[test]
fn only_an_empty_recording_forces_a_swapchain_reset() {
    // A direct-marked recording is refreshed by a plain composite frame
    // (Smithay damages the whole output coming back from direct scanout);
    // resetting there reallocated the swapchain on every screenshot of a
    // direct window. An empty one has nothing to diff against.
    assert!(force_needs_reset(scanout::Stale::Empty));
    assert!(!force_needs_reset(scanout::Stale::Direct));
}

mod resize;

mod arrange_once;

mod capture_release;

#[cfg(feature = "gpu-scanout")]
mod host_copy;
