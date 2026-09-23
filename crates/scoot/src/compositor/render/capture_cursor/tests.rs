//! The cursor-in-capture decision, without a renderer: which region a
//! capture must re-render for each combination of what the frame holds and
//! what was asked for, and the patch's own bounds.

use smithay::utils::{Physical, Rectangle};

use super::*;

fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Physical> {
    Rectangle::new((x, y).into(), (w, h).into())
}

fn composited_at(r: Rectangle<i32, Physical>) -> CursorInFrame {
    CursorInFrame {
        composited: Some(r),
        off_frame: false,
    }
}

const ON_PLANE: CursorInFrame = CursorInFrame {
    composited: None,
    off_frame: true,
};

const NOT_DRAWN: CursorInFrame = CursorInFrame {
    composited: None,
    off_frame: false,
};

#[test]
fn a_frame_that_composited_the_cursor_where_it_is_needs_nothing_when_asked() {
    // The dumb tier with the default request: the common case costs nothing.
    let at = rect(100, 100, 16, 16);
    assert_eq!(patch_region(composited_at(at), Some(at), true), None);
}

#[test]
fn a_frame_without_the_cursor_gets_it_drawn_where_it_is() {
    // Headless and nested (never drawn) and the scanout tier (on a plane):
    // the region is where the cursor sits now.
    let at = rect(100, 100, 16, 16);
    assert_eq!(patch_region(NOT_DRAWN, Some(at), true), Some(at));
    assert_eq!(patch_region(ON_PLANE, Some(at), true), Some(at));
}

#[test]
fn a_cursor_that_moved_since_the_frame_is_redrawn_at_both_places() {
    // Never two cursors: the stale one is re-rendered away in the same
    // region as the new one is drawn.
    let then = rect(10, 10, 16, 16);
    let now = rect(40, 20, 16, 16);
    assert_eq!(
        patch_region(composited_at(then), Some(now), true),
        Some(rect(10, 10, 46, 26))
    );
}

#[test]
fn a_partly_planed_cursor_is_redrawn_whole() {
    // Some elements composited, some on a plane (a client cursor tree):
    // the frame's footprint matches, but it is still incomplete.
    let at = rect(100, 100, 16, 16);
    let partial = CursorInFrame {
        composited: Some(at),
        off_frame: true,
    };
    assert_eq!(patch_region(partial, Some(at), true), Some(at));
}

#[test]
fn a_hidden_cursor_needs_nothing_unless_the_frame_still_shows_one() {
    assert_eq!(patch_region(NOT_DRAWN, None, true), None);
    assert_eq!(patch_region(ON_PLANE, None, true), None);
    // The frame composited a cursor that is gone now: take it out.
    let then = rect(5, 5, 16, 16);
    assert_eq!(patch_region(composited_at(then), None, true), Some(then));
}

#[test]
fn leaving_the_cursor_out_touches_only_where_the_frame_composited_it() {
    let then = rect(5, 5, 16, 16);
    let now = rect(300, 300, 16, 16);
    assert_eq!(
        patch_region(composited_at(then), Some(now), false),
        Some(then)
    );
    assert_eq!(patch_region(composited_at(then), None, false), Some(then));
    // Nothing composited (headless, nested, a planed cursor): the frame is
    // already cursorless, wherever the cursor is.
    assert_eq!(patch_region(NOT_DRAWN, Some(now), false), None);
    assert_eq!(patch_region(ON_PLANE, Some(now), false), None);
}

/// A stand-in element: only kind, id and geometry are read by
/// [`CursorInFrame::of`].
struct Stub {
    id: Id,
    kind: Kind,
    geometry: Rectangle<i32, Physical>,
}

impl Stub {
    fn cursor(geometry: Rectangle<i32, Physical>) -> Self {
        Self {
            id: Id::new(),
            kind: Kind::Cursor,
            geometry,
        }
    }

    fn window(geometry: Rectangle<i32, Physical>) -> Self {
        Self {
            id: Id::new(),
            kind: Kind::Unspecified,
            geometry,
        }
    }
}

impl Element for Stub {
    fn id(&self) -> &Id {
        &self.id
    }

    fn current_commit(&self) -> smithay::backend::renderer::utils::CommitCounter {
        Default::default()
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        Rectangle::from_size((1.0, 1.0).into())
    }

    fn geometry(&self, _scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.geometry
    }

    fn kind(&self) -> Kind {
        self.kind
    }
}

/// The output every record here is clamped to.
fn output() -> Rectangle<i32, Physical> {
    rect(0, 0, 800, 600)
}

#[test]
fn the_record_is_the_cursor_elements_only() {
    let elements = [
        Stub::cursor(rect(10, 10, 16, 16)),
        Stub::window(rect(0, 0, 800, 600)),
    ];
    let record = CursorInFrame::of(&elements, 1.0.into(), output(), |_| false);
    assert_eq!(record, composited_at(rect(10, 10, 16, 16)));
}

#[test]
fn the_record_unions_a_cursor_tree_and_notes_what_rode_a_plane() {
    let planed = Stub::cursor(rect(50, 50, 8, 8));
    let planed_id = planed.id.clone();
    let elements = [
        Stub::cursor(rect(10, 10, 16, 16)),
        Stub::cursor(rect(20, 30, 4, 4)),
        planed,
    ];
    let record = CursorInFrame::of(&elements, 1.0.into(), output(), |id| *id == planed_id);
    assert_eq!(
        record,
        CursorInFrame {
            composited: Some(rect(10, 10, 16, 24)),
            off_frame: true,
        }
    );
}

#[test]
fn the_record_clamps_to_the_output_before_it_unions() {
    // A client cursor surface can be made enormous (a viewport destination
    // is the client's number): clamped first, the union cannot overflow and
    // stays inside the output.
    let elements = [
        Stub::cursor(rect(790, 590, i32::MAX, i32::MAX)),
        Stub::cursor(rect(-20, -20, 30, 30)),
    ];
    let record = CursorInFrame::of(&elements, 1.0.into(), output(), |_| false);
    assert_eq!(record, composited_at(rect(0, 0, 800, 600)));
}

#[test]
fn a_cursor_off_this_output_is_not_in_its_record() {
    // The pointer on a second output lands its elements outside this one's
    // framebuffer: nothing to draw here, nothing to take out.
    let elements = [Stub::cursor(rect(900, 100, 16, 16))];
    let record = CursorInFrame::of(&elements, 1.0.into(), output(), |_| false);
    assert_eq!(record, NOT_DRAWN);
    // Touching the edge from outside is a zero-area overlap, not a pixel.
    let touching = [Stub::cursor(rect(800, 0, 16, 16))];
    assert_eq!(
        CursorInFrame::of(&touching, 1.0.into(), output(), |_| false),
        NOT_DRAWN
    );
}

fn patch(rect: Rectangle<i32, Physical>, fill: u8) -> CursorPatch {
    CursorPatch {
        rect,
        pixels: vec![fill; rect.size.w as usize * rect.size.h as usize * 4],
    }
}

#[test]
fn a_patch_is_written_over_exactly_its_rect() {
    let (width, height) = (8, 6);
    let mut frame = vec![0u8; (width * height * 4) as usize];
    patch(rect(2, 1, 3, 2), 0xAB).apply(&mut frame, width, height);
    for y in 0..height {
        for x in 0..width {
            let inside = (2..5).contains(&x) && (1..3).contains(&y);
            let at = ((y * width + x) * 4) as usize;
            let want = if inside { 0xAB } else { 0 };
            assert_eq!(frame[at..at + 4], [want; 4], "pixel ({x}, {y})");
        }
    }
}

#[test]
fn a_patch_that_does_not_fit_is_dropped_whole() {
    let (width, height) = (8, 6);
    for bad in [
        patch(rect(6, 0, 3, 2), 0xAB),  // past the right edge
        patch(rect(0, 5, 2, 2), 0xAB),  // past the bottom
        patch(rect(-1, 0, 2, 2), 0xAB), // negative origin
        CursorPatch {
            rect: rect(0, 0, 2, 2),
            pixels: vec![0xAB; 3], // short pixels
        },
    ] {
        assert!(!bad.fits(width, height), "{:?} fits", bad.rect);
        let mut frame = vec![0u8; (width * height * 4) as usize];
        bad.apply(&mut frame, width, height);
        assert!(frame.iter().all(|byte| *byte == 0), "{:?} wrote", bad.rect);
    }
    // And a frame shorter than its own dimensions is not indexed past.
    let mut short = vec![0u8; 10];
    patch(rect(0, 0, 1, 1), 0xAB).apply(&mut short, width, height);
    assert!(short.iter().all(|byte| *byte == 0));
}

#[test]
fn packing_drops_row_padding() {
    // A renderer that pads rows (stride 12 for 2-pixel rows) still yields
    // tight rows.
    let mut padded = Vec::new();
    for y in 0..3u8 {
        padded.extend_from_slice(&[y; 8]);
        padded.extend_from_slice(&[0xEE; 4]);
    }
    let packed = pack_rows(&padded, 2, 3).expect("packs");
    assert_eq!(packed.len(), 2 * 3 * 4);
    assert!(packed[..8].iter().all(|byte| *byte == 0));
    assert!(packed[16..].iter().all(|byte| *byte == 2));
    assert!(pack_rows(&padded[..10], 2, 3).is_err());
    assert!(pack_rows(&padded, 0, 3).is_err());
}
