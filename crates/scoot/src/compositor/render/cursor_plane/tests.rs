//! `render::cursor_plane`: the padding arithmetic, and the swap over a real
//! (pixman) renderer with a fake upload. The plane assignment itself needs a
//! KMS device; it is verified on `apple,dcp` (`Asahi.md`, Test 17).

use std::cell::RefCell;
use std::fs::File;
use std::os::fd::OwnedFd;
use std::rc::Rc;

use smithay::backend::allocator::Buffer as _;
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::element::memory::MemoryRenderBuffer;
use smithay::backend::renderer::pixman::PixmanRenderer;
use smithay::backend::renderer::{Bind, Color32F, ExportMem, Offscreen};
use smithay::utils::{Physical, Point, Rectangle, Scale, Size};

use super::*;

// -------------------------------------------------------------------------
// Pure arithmetic
// -------------------------------------------------------------------------

#[test]
fn a_small_cursor_is_padded_to_the_smallest_plane() {
    assert_eq!(plane_size(16, 16), Some((32, 32)));
    assert_eq!(plane_size(24, 24), Some((32, 32)));
    assert_eq!(plane_size(1, 1), Some((32, 32)));
    assert_eq!(plane_size(32, 32), Some((32, 32)));
}

#[test]
fn a_wide_cursor_is_padded_to_a_64_byte_pitch_and_a_tall_one_is_not() {
    assert_eq!(plane_size(33, 40), Some((48, 40)));
    assert_eq!(plane_size(48, 47), Some((48, 47)));
    assert_eq!(plane_size(49, 49), Some((64, 49)));
    for width in 1..=MAX_SIDE {
        let (padded, _) = plane_size(width, 1).expect("in range");
        assert_eq!(padded * 4 % 64, 0, "width {width}");
        assert!(padded >= width.max(MIN_SIDE), "width {width}");
        assert!(padded < width.max(MIN_SIDE) + PITCH_PIXELS, "width {width}");
    }
}

#[test]
fn an_empty_negative_or_huge_size_is_not_put_on_a_plane() {
    assert_eq!(plane_size(0, 24), None);
    assert_eq!(plane_size(24, 0), None);
    assert_eq!(plane_size(-24, 24), None);
    assert_eq!(plane_size(i32::MIN, i32::MIN), None);
    assert_eq!(plane_size(MAX_SIDE + 1, 24), None);
    assert_eq!(plane_size(24, i32::MAX), None);
    assert_eq!(plane_size(MAX_SIDE, MAX_SIDE), Some((MAX_SIDE, MAX_SIDE)));
}

/// A `width` x `height` image whose every pixel names its own position, so a
/// misplaced row or column is visible in the bytes.
fn numbered(width: usize, height: usize, stride: usize) -> Vec<u8> {
    let mut pixels = vec![0xEE; stride * height];
    for y in 0..height {
        for x in 0..width {
            let at = y * stride + x * 4;
            pixels[at..at + 4].copy_from_slice(&[x as u8, y as u8, 0x7F, 0xFF]);
        }
    }
    pixels
}

#[test]
fn padding_keeps_the_image_at_the_top_left_and_clears_the_rest() {
    let source = numbered(3, 2, 12);
    let padded = pad(&source, 12, (3, 2), (5, 4)).expect("fits");
    assert_eq!(padded.len(), 5 * 4 * 4);
    for y in 0..4 {
        for x in 0..5 {
            let at = (y * 5 + x) * 4;
            let expected = if x < 3 && y < 2 {
                [x as u8, y as u8, 0x7F, 0xFF]
            } else {
                [0, 0, 0, 0]
            };
            assert_eq!(padded[at..at + 4], expected, "pixel ({x}, {y})");
        }
    }
}

#[test]
fn padding_drops_a_source_strides_own_padding() {
    // 3 pixels wide, rows 20 bytes apart: the 8 trailing bytes per row
    // (`0xEE`) are the source's own padding and must not be copied.
    let source = numbered(3, 2, 20);
    let padded = pad(&source, 20, (3, 2), (4, 2)).expect("fits");
    assert_eq!(padded[12..16], [0, 0, 0, 0]);
    assert_eq!(padded[16..20], [0, 1, 0x7F, 0xFF]);
    assert!(!padded.contains(&0xEE));
}

#[test]
fn padding_refuses_inconsistent_sizes_rather_than_reading_past_the_source() {
    let source = numbered(3, 2, 12);
    // Shorter than the size claims.
    assert_eq!(pad(&source[..20], 12, (3, 2), (4, 4)), None);
    // A stride shorter than a row.
    assert_eq!(pad(&source, 8, (3, 2), (4, 4)), None);
    // A target smaller than the image.
    assert_eq!(pad(&source, 12, (3, 2), (2, 4)), None);
    assert_eq!(pad(&source, 12, (3, 2), (4, 1)), None);
    // Negative anything.
    assert_eq!(pad(&source, -12, (3, 2), (4, 4)), None);
    assert_eq!(pad(&source, 12, (-3, 2), (4, 4)), None);
}

// -------------------------------------------------------------------------
// The swap, over a real renderer
// -------------------------------------------------------------------------

/// What a fake upload saw: every call's pixels and size.
#[derive(Default)]
struct Seen {
    calls: Vec<(Vec<u8>, u32, u32)>,
}

/// An [`Upload`] that records each call and answers a dma-buf over
/// `/dev/null` (never read: nothing here scans out), or refuses on demand.
struct FakeUpload {
    seen: Rc<RefCell<Seen>>,
    refuse: bool,
}

impl Upload for FakeUpload {
    fn upload(&mut self, pixels: &[u8], width: u32, height: u32) -> Option<Dmabuf> {
        self.seen
            .borrow_mut()
            .calls
            .push((pixels.to_vec(), width, height));
        if self.refuse {
            return None;
        }
        let fd: OwnedFd = File::open("/dev/null").expect("/dev/null").into();
        let mut builder = Dmabuf::builder(
            (width as i32, height as i32),
            Fourcc::Argb8888,
            Modifier::Linear,
            DmabufFlags::empty(),
        );
        builder.add_plane(fd, 0, width * 4);
        builder.build()
    }
}

fn planes(refuse: bool) -> (CursorPlanes<FakeUpload>, Rc<RefCell<Seen>>) {
    let seen = Rc::new(RefCell::new(Seen::default()));
    let upload = FakeUpload {
        seen: Rc::clone(&seen),
        refuse,
    };
    (CursorPlanes::new(upload), seen)
}

/// A 24 px opaque red square with a blue top-left pixel: a cursor-sized
/// image whose corner shows any offset.
fn square() -> MemoryRenderBuffer {
    let mut pixels = vec![0u8; 24 * 24 * 4];
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.copy_from_slice(&[0, 0, 0xFF, 0xFF]);
    }
    pixels[..4].copy_from_slice(&[0xFF, 0, 0, 0xFF]);
    MemoryRenderBuffer::from_slice(
        &pixels,
        Fourcc::Argb8888,
        (24, 24),
        1,
        Transform::Normal,
        None,
    )
}

fn drawn(
    renderer: &mut PixmanRenderer,
    buffer: &MemoryRenderBuffer,
    at: Point<f64, Physical>,
) -> Elements<PixmanRenderer> {
    let element = MemoryRenderBufferRenderElement::from_buffer(
        renderer,
        at,
        buffer,
        None,
        None,
        None,
        Kind::Cursor,
    )
    .expect("a memory element");
    Elements::Cursor(CursorElement::Fallback(element))
}

const CANVAS: i32 = 96;

/// An output at scale 1 big enough that no test cursor is near its edge.
fn panel() -> (Scale<f64>, Size<i32, Physical>) {
    (Scale::from(1.0), Size::from((2560, 1600)))
}
const CLEAR: Color32F = Color32F::new(0.0, 0.5, 0.0, 1.0);

/// The frame `elements` composite to, at `scale`, as raw pixels.
fn frame(
    renderer: &mut PixmanRenderer,
    elements: &[Elements<PixmanRenderer>],
    scale: f64,
) -> Vec<u8> {
    let mut image = renderer
        .create_buffer(Fourcc::Argb8888, (CANVAS, CANVAS).into())
        .expect("an offscreen buffer");
    let mut framebuffer = renderer.bind(&mut image).expect("a framebuffer");
    let mut damage = OutputDamageTracker::new((CANVAS, CANVAS), scale, Transform::Normal);
    damage
        .render_output(renderer, &mut framebuffer, 0, elements, CLEAR)
        .expect("a rendered frame");
    let region = Rectangle::from_size((CANVAS, CANVAS).into());
    let mapping = renderer
        .copy_framebuffer(&framebuffer, region, Fourcc::Argb8888)
        .expect("a readback");
    renderer.map_texture(&mapping).expect("pixels").to_vec()
}

#[test]
fn a_drawn_cursor_becomes_a_plane_twin_at_the_same_place() {
    let mut renderer = PixmanRenderer::new().expect("a pixman renderer");
    let buffer = square();
    let at = Point::from((10.4, 20.6));
    let mut elements = vec![drawn(&mut renderer, &buffer, at)];
    let (mut planes, seen) = planes(false);
    planes.back(&mut renderer, &mut elements, panel());

    let Elements::Cursor(CursorElement::Plane(twin)) = &elements[0] else {
        panic!("the drawn cursor was not swapped");
    };
    assert_eq!(twin.kind(), Kind::Cursor);
    let geometry = twin.geometry(Scale::from(1.0));
    assert_eq!(
        geometry.loc,
        Point::from((10, 21)),
        "the memory element's rounded origin"
    );
    assert_eq!(
        geometry.size,
        Size::from((32, 32)),
        "padded to the smallest plane"
    );
    let Some(UnderlyingStorage::Dmabuf(dmabuf)) = twin.underlying_storage(&mut renderer) else {
        panic!("the twin does not answer its dma-buf");
    };
    assert_eq!(dmabuf.size(), Size::from((32, 32)));
    assert_eq!(dmabuf.format().code, Fourcc::Argb8888);
    assert_eq!(dmabuf.format().modifier, Modifier::Linear);

    let seen = seen.borrow();
    assert_eq!(seen.calls.len(), 1);
    let (pixels, width, height) = &seen.calls[0];
    assert_eq!((*width, *height), (32, 32));
    assert_eq!(
        pixels[..4],
        [0xFF, 0, 0, 0xFF],
        "the image's corner is the plane's"
    );
    assert_eq!(
        pixels[23 * 4..24 * 4],
        [0, 0, 0xFF, 0xFF],
        "the image's last column"
    );
    assert_eq!(
        pixels[24 * 4..25 * 4],
        [0, 0, 0, 0],
        "padding is transparent"
    );
    let last_row = 31 * 32 * 4;
    assert!(pixels[last_row..].iter().all(|byte| *byte == 0));
}

#[test]
fn the_twin_composites_to_exactly_the_pixels_the_drawn_cursor_does_at_scale_1() {
    // What a composited frame and a capture's re-render draw when the plane
    // test fails or the frame must land whole: no different from before.
    let mut renderer = PixmanRenderer::new().expect("a pixman renderer");
    let buffer = square();
    let at = Point::from((7.0, 9.0));
    let original = [drawn(&mut renderer, &buffer, at)];
    let drawn_frame = frame(&mut renderer, &original, 1.0);
    let mut elements = vec![drawn(&mut renderer, &buffer, at)];
    let (mut planes, _) = planes(false);
    planes.back(&mut renderer, &mut elements, panel());
    assert!(matches!(
        elements[0],
        Elements::Cursor(CursorElement::Plane(_))
    ));
    assert!(drawn_frame == frame(&mut renderer, &elements, 1.0));
}

#[test]
fn scaled_the_twin_differs_only_along_the_images_right_and_bottom_edge() {
    // Scaled, the drawn cursor's last row and column are filtered against a
    // clamped edge (`Repeat::Pad`), the twin's against its transparent
    // padding -- which is also what the plane's scaler sees. Everywhere
    // else the pixels are the same.
    for scale in [1.5, 2.0] {
        let mut renderer = PixmanRenderer::new().expect("a pixman renderer");
        let buffer = square();
        let at = Point::from((7.0, 9.0));
        let original = [drawn(&mut renderer, &buffer, at)];
        let drawn_frame = frame(&mut renderer, &original, scale);
        let mut elements = vec![drawn(&mut renderer, &buffer, at)];
        let (mut planes, _) = planes(false);
        planes.back(
            &mut renderer,
            &mut elements,
            (Scale::from(scale), Size::from((2560, 1600))),
        );
        assert!(matches!(
            elements[0],
            Elements::Cursor(CursorElement::Plane(_))
        ));
        let twin_frame = frame(&mut renderer, &elements, scale);
        // The location is physical, so the image ends at `at + 24 * scale`;
        // the band is the two physical pixels either side of that line.
        let edge = |origin: f64| (origin + 24.0 * scale) as i32;
        let (right, bottom) = (edge(7.0), edge(9.0));
        let mut differing = 0;
        for y in 0..CANVAS {
            for x in 0..CANVAS {
                let at = ((y * CANVAS + x) * 4) as usize;
                if drawn_frame[at..at + 4] == twin_frame[at..at + 4] {
                    continue;
                }
                differing += 1;
                let near_edge = (x - right).abs() <= 2 || (y - bottom).abs() <= 2;
                assert!(
                    near_edge,
                    "scale {scale}: pixel ({x}, {y}) differs away from the edge"
                );
            }
        }
        assert!(
            differing > 0,
            "scale {scale}: expected the filtered edge to differ"
        );
    }
}

#[test]
fn each_image_is_uploaded_once_however_many_frames_show_it() {
    let mut renderer = PixmanRenderer::new().expect("a pixman renderer");
    let buffer = square();
    let (mut planes, seen) = planes(false);
    for frame in 0..5 {
        let at = Point::from((f64::from(frame) * 3.0, 0.0));
        let mut elements = vec![drawn(&mut renderer, &buffer, at)];
        planes.back(&mut renderer, &mut elements, panel());
        assert!(matches!(
            elements[0],
            Elements::Cursor(CursorElement::Plane(_))
        ));
    }
    assert_eq!(seen.borrow().calls.len(), 1);
    assert_eq!(planes.len(), 1);
}

#[test]
fn a_refused_upload_composites_and_is_not_retried() {
    let mut renderer = PixmanRenderer::new().expect("a pixman renderer");
    let buffer = square();
    let (mut planes, seen) = planes(true);
    for _ in 0..3 {
        let mut elements = vec![drawn(&mut renderer, &buffer, Point::from((0.0, 0.0)))];
        planes.back(&mut renderer, &mut elements, panel());
        assert!(matches!(
            elements[0],
            Elements::Cursor(CursorElement::Fallback(_))
        ));
    }
    assert_eq!(seen.borrow().calls.len(), 1, "a refusal is remembered");
}

#[test]
fn a_format_other_than_argb8888_composites_without_an_upload() {
    let mut renderer = PixmanRenderer::new().expect("a pixman renderer");
    let pixels = vec![0xFFu8; 24 * 24 * 4];
    let buffer = MemoryRenderBuffer::from_slice(
        &pixels,
        Fourcc::Abgr8888,
        (24, 24),
        1,
        Transform::Normal,
        None,
    );
    let mut elements = vec![drawn(&mut renderer, &buffer, Point::from((0.0, 0.0)))];
    let (mut planes, seen) = planes(false);
    planes.back(&mut renderer, &mut elements, panel());
    assert!(matches!(
        elements[0],
        Elements::Cursor(CursorElement::Fallback(_))
    ));
    assert!(seen.borrow().calls.is_empty());
}

#[test]
fn a_buffer_scale_or_transform_the_twin_cannot_copy_composites_without_an_upload() {
    let mut renderer = PixmanRenderer::new().expect("a pixman renderer");
    let pixels = vec![0xFFu8; 24 * 24 * 4];
    let (mut planes, seen) = planes(false);
    for (scale, transform) in [(2, Transform::Normal), (1, Transform::_90)] {
        let buffer = MemoryRenderBuffer::from_slice(
            &pixels,
            Fourcc::Argb8888,
            (24, 24),
            scale,
            transform,
            None,
        );
        let mut elements = vec![drawn(&mut renderer, &buffer, Point::from((0.0, 0.0)))];
        planes.back(&mut renderer, &mut elements, panel());
        assert!(
            matches!(elements[0], Elements::Cursor(CursorElement::Fallback(_))),
            "scale {scale}, {transform:?}"
        );
    }
    assert!(seen.borrow().calls.is_empty());
}

#[test]
fn only_drawn_cursor_elements_are_swapped() {
    let mut renderer = PixmanRenderer::new().expect("a pixman renderer");
    let buffer = square();
    let ring = MemoryRenderBufferRenderElement::from_buffer(
        &mut renderer,
        Point::from((0.0, 0.0)),
        &buffer,
        None,
        None,
        None,
        Kind::Unspecified,
    )
    .expect("a memory element");
    let mut elements = vec![
        drawn(&mut renderer, &buffer, Point::from((0.0, 0.0))),
        Elements::PaintedRing(ring),
    ];
    let (mut planes, _) = planes(false);
    planes.back(&mut renderer, &mut elements, panel());
    assert!(matches!(
        elements[0],
        Elements::Cursor(CursorElement::Plane(_))
    ));
    assert!(matches!(elements[1], Elements::PaintedRing(_)));
}

#[test]
fn the_cache_starts_over_at_its_bound() {
    let mut renderer = PixmanRenderer::new().expect("a pixman renderer");
    let (mut planes, seen) = planes(false);
    let buffers: Vec<MemoryRenderBuffer> = (0..=MAX_IMAGES).map(|_| square()).collect();
    for (index, buffer) in buffers.iter().enumerate() {
        let mut elements = vec![drawn(&mut renderer, buffer, Point::from((0.0, 0.0)))];
        planes.back(&mut renderer, &mut elements, panel());
        assert!(planes.len() <= MAX_IMAGES, "after image {index}");
    }
    assert_eq!(
        planes.len(),
        1,
        "the bound was hit once and the cache started over"
    );
    // The first image is gone, so showing it again uploads it again.
    let mut elements = vec![drawn(&mut renderer, &buffers[0], Point::from((0.0, 0.0)))];
    planes.back(&mut renderer, &mut elements, panel());
    assert_eq!(seen.borrow().calls.len(), MAX_IMAGES + 2);
}

// -------------------------------------------------------------------------
// Edges: a plane the driver would clip under 32 px is not offered
// -------------------------------------------------------------------------

fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Physical> {
    Rectangle::new((x, y).into(), (w, h).into())
}

#[test]
fn a_plane_inside_the_output_fits() {
    let output = Size::from((2560, 1600));
    assert!(fits(rect(1196, 749, 48, 48), output));
    assert!(fits(rect(0, 0, 32, 32), output));
    assert!(fits(rect(2528, 1568, 32, 32), output));
}

#[test]
fn a_plane_clipped_to_32_or_more_fits_and_under_32_does_not() {
    let output = Size::from((2560, 1600));
    // Measured on apple,dcp (Asahi.md Test 17): 44 px left at the right
    // edge rode the overlay, 23.5 did not.
    assert!(fits(rect(2516, 749, 48, 48), output));
    assert!(!fits(rect(2537, 749, 48, 48), output));
    assert!(fits(rect(2528, 749, 48, 48), output));
    assert!(!fits(rect(2529, 749, 48, 48), output));
    // The top-left: a centred hotspot puts the origin above or left of it.
    assert!(fits(rect(-5, -2, 48, 48), output));
    assert!(fits(rect(-16, 0, 48, 48), output));
    assert!(!fits(rect(-17, 0, 48, 48), output));
    assert!(!fits(rect(0, 1580, 48, 48), output));
}

#[test]
fn a_plane_off_the_output_or_absurdly_placed_does_not_fit_and_does_not_panic() {
    let output = Size::from((2560, 1600));
    assert!(!fits(rect(3000, 100, 48, 48), output));
    assert!(!fits(rect(-100, 100, 48, 48), output));
    assert!(!fits(rect(i32::MAX, i32::MAX, 48, 48), output));
    assert!(!fits(rect(i32::MIN, i32::MIN, 48, 48), output));
    assert!(!fits(rect(i32::MIN, 0, i32::MAX, 48), output));
    assert!(!fits(rect(0, 0, 48, 48), Size::from((0, 0))));
}

#[test]
fn a_cursor_at_the_edge_stays_composited_and_rides_again_away_from_it() {
    let mut renderer = PixmanRenderer::new().expect("a pixman renderer");
    let buffer = square();
    let (mut planes, seen) = planes(false);
    // At scale 1.5 the 32 px twin is 48 physical: 20 px left at x = 2540.
    let output = (Scale::from(1.5), Size::from((2560, 1600)));
    let mut elements = vec![drawn(&mut renderer, &buffer, Point::from((2540.0, 700.0)))];
    planes.back(&mut renderer, &mut elements, output);
    assert!(matches!(
        elements[0],
        Elements::Cursor(CursorElement::Fallback(_))
    ));
    let mut elements = vec![drawn(&mut renderer, &buffer, Point::from((2500.0, 700.0)))];
    planes.back(&mut renderer, &mut elements, output);
    assert!(matches!(
        elements[0],
        Elements::Cursor(CursorElement::Plane(_))
    ));
    assert_eq!(
        seen.borrow().calls.len(),
        1,
        "the image is built once, edge or not"
    );
}

// -------------------------------------------------------------------------
// Cost
// -------------------------------------------------------------------------

/// What the swap adds to a frame once the image is cached: the drawn
/// element is built either way (that is the existing per-frame cost), and
/// the swap adds a lookup, a second `from_buffer` and the fit check. Run by
/// hand: `cargo nextest run -p scoot --features gpu-scanout --run-ignored
/// only -E 'test(/cursor_plane::tests::bench/)' --no-capture`.
#[test]
#[ignore = "prints per-frame swap timings for a human; asserts nothing"]
fn bench_the_swap_per_frame() {
    use std::time::Instant;
    const FRAMES: u32 = 200_000;
    let mut renderer = PixmanRenderer::new().expect("a pixman renderer");
    let buffer = square();
    let (mut planes, _) = planes(false);
    let mut elements = vec![drawn(&mut renderer, &buffer, Point::from((10.0, 10.0)))];
    planes.back(&mut renderer, &mut elements, panel());
    let at = |frame: u32| Point::from((f64::from(frame % 1000), 400.0));

    let start = Instant::now();
    for frame in 0..FRAMES {
        elements[0] = drawn(&mut renderer, &buffer, at(frame));
        std::hint::black_box(&elements);
    }
    let drawn_only = start.elapsed();

    let start = Instant::now();
    for frame in 0..FRAMES {
        elements[0] = drawn(&mut renderer, &buffer, at(frame));
        planes.back(&mut renderer, &mut elements, panel());
        std::hint::black_box(&elements);
    }
    let with_swap = start.elapsed();
    assert!(matches!(
        elements[0],
        Elements::Cursor(CursorElement::Plane(_))
    ));
    let per = |total: std::time::Duration| total.as_nanos() / u128::from(FRAMES);
    println!(
        "cursor element per frame: drawn only {} ns, drawn + swap {} ns ({FRAMES} frames)",
        per(drawn_only),
        per(with_swap)
    );
}
