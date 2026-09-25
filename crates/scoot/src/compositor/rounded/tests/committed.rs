//! The clip, the ring and the IPC `rect` follow what the client actually
//! drew, not the layout slot it was given (gh #205).
//!
//! `foot`'s default `resize-by-cells` commits a buffer rounded down to whole
//! character cells whenever it believes it is floating, so it draws a few
//! pixels short of its slot on the right and at the bottom. Clipping and
//! ringing the slot rounded a corner the content never reached. The client
//! here is that shape made exact: it draws `shrink` logical pixels short of
//! each configure, through a viewport at the session's preferred scale.

use super::*;

/// Odd on purpose: at 1.5 neither axis lands on a whole physical pixel, so
/// the drawn rect's rounding differs from the slot's.
const SHRINK: (i32, i32) = (13, 7);

/// `slot` with `shrink` logical pixels taken off its right and bottom: the
/// window's content stays anchored at the slot's top-left corner (that is
/// where the window is mapped), so only the far edges move.
fn drawn(slot: Rect, shrink: (i32, i32)) -> Rect {
    Rect::new(slot.x, slot.y, slot.w - shrink.0, slot.h - shrink.1)
}

fn check_under_fill(scale: f64) {
    const RADIUS: i32 = 10;
    const THICKNESS: i32 = 4;
    let mut fixture = Fixture::with_radius_at_scale(RADIUS, THICKNESS, scale);
    fixture.run(Step::ShortWindow {
        color: WINDOW_BGRA,
        shrink: SHRINK,
    });
    let slot = fixture.placement();
    let pixels = fixture.render();
    let drawn = drawn(slot, SHRINK);
    assert_ring_hugs_content(&pixels, drawn, scale, RADIUS, THICKNESS);

    // The strip of the slot the client left undrawn is background: no ring
    // around the slot's far corner, no window colour.
    let bg: [u8; 4] = pixels[0..4].try_into().expect("canvas corner");
    let slot_clip = clip_rect(slot, scale);
    let (right, bottom) = (
        slot_clip.loc.x + slot_clip.size.w - 1,
        slot_clip.loc.y + slot_clip.size.h - 1,
    );
    for (px, py, what) in [
        (
            right,
            slot_clip.loc.y + slot_clip.size.h / 2,
            "the undrawn right strip",
        ),
        (
            slot_clip.loc.x + slot_clip.size.w / 2,
            bottom,
            "the undrawn bottom strip",
        ),
        (right, bottom, "the slot's own bottom-right corner"),
    ] {
        assert_pixel(
            &pixels,
            CANVAS,
            px,
            py,
            bg,
            &format!("scale {scale}: {what}"),
        );
    }
}

#[test]
fn an_under_filling_client_is_clipped_and_ringed_where_it_drew_at_scale_one() {
    check_under_fill(1.0);
}

#[test]
fn an_under_filling_client_is_clipped_and_ringed_where_it_drew_at_scale_one_point_five() {
    check_under_fill(1.5);
}

#[test]
fn an_under_filling_client_is_clipped_and_ringed_where_it_drew_at_scale_two() {
    check_under_fill(2.0);
}

/// A resize, as the client experiences it: the ring and the clip follow each
/// frame the client commits, the frame it lands and not before -- short,
/// then full, then short by a different amount.
#[test]
fn the_ring_and_clip_follow_each_committed_frame() {
    const RADIUS: i32 = 10;
    const THICKNESS: i32 = 4;
    const SCALE: f64 = 1.5;
    let mut fixture = Fixture::with_radius_at_scale(RADIUS, THICKNESS, SCALE);
    fixture.run(Step::ShortWindow {
        color: WINDOW_BGRA,
        shrink: SHRINK,
    });
    let slot = fixture.placement();
    let pixels = fixture.render();
    assert_ring_hugs_content(&pixels, drawn(slot, SHRINK), SCALE, RADIUS, THICKNESS);

    fixture.run(Step::Redraw {
        window: 0,
        shrink: (0, 0),
    });
    assert_eq!(fixture.placement(), slot, "a redraw moves no slot");
    let pixels = fixture.render();
    assert_ring_hugs_content(&pixels, slot, SCALE, RADIUS, THICKNESS);

    let other = (5, 9);
    fixture.run(Step::Redraw {
        window: 0,
        shrink: other,
    });
    let pixels = fixture.render();
    assert_ring_hugs_content(&pixels, drawn(slot, other), SCALE, RADIUS, THICKNESS);
}

/// The square ring (`corner_radius = 0`) hugs the drawn rect too, so the ring
/// never disagrees with the IPC `rect` below at either radius: the top bar
/// ends where the content does.
#[test]
fn the_square_ring_hugs_what_the_client_drew() {
    let mut fixture = Fixture::with_radius(0);
    fixture.run(Step::ShortWindow {
        color: WINDOW_BGRA,
        shrink: SHRINK,
    });
    let slot = fixture.placement();
    let drawn = drawn(slot, SHRINK);
    let pixels = fixture.render();
    let bg: [u8; 4] = pixels[0..4].try_into().expect("canvas corner");
    let thickness = Appearance::default().focus_ring_width;
    let ring = pixel(&pixels, CANVAS, drawn.x + drawn.w / 2, drawn.y - 1);
    assert_ne!(ring, bg, "the top bar must draw");
    assert_ne!(ring, WINDOW_BGRA, "the sample must be ring, not window");
    for (px, py, what) in [
        (
            drawn.x + drawn.w,
            drawn.y + drawn.h / 2,
            "the right bar hugs the drawn edge",
        ),
        (
            drawn.x + drawn.w / 2,
            drawn.y + drawn.h,
            "the bottom bar hugs the drawn edge",
        ),
        (
            drawn.x + drawn.w + thickness - 1,
            drawn.y - thickness,
            "the top bar's far end",
        ),
    ] {
        assert_pixel(&pixels, CANVAS, px, py, ring, what);
    }
    for (px, py, what) in [
        (
            drawn.x + drawn.w + thickness,
            drawn.y - 1,
            "past the top bar's far end",
        ),
        (
            slot.x + slot.w - 1,
            slot.y + slot.h / 2,
            "the undrawn right strip",
        ),
        (
            slot.x + slot.w / 2,
            slot.y + slot.h - 1,
            "the undrawn bottom strip",
        ),
    ] {
        assert_pixel(&pixels, CANVAS, px, py, bg, what);
    }
}

/// IPC `rect` is what is drawn and clickable: the slot's origin and the part
/// of it the client committed. Short, it is the drawn rect; full, the slot;
/// past the slot (a shrink still in flight), the slot -- clamped, so the
/// report never claims space the layout gave a neighbor.
#[test]
fn the_ipc_rect_is_what_the_window_drew_clamped_to_its_slot() {
    let mut fixture = Fixture::with_radius_at_scale(10, 4, 1.5);
    fixture.run(Step::ShortWindow {
        color: WINDOW_BGRA,
        shrink: SHRINK,
    });
    let slot = fixture.placement();
    let reported = |fixture: &mut Fixture| {
        fixture.settle();
        let snapshots = fixture.state.window_snapshots();
        assert_eq!(snapshots.len(), 1, "one window");
        let rect = snapshots[0].rect;
        Rect::new(rect.x, rect.y, rect.width, rect.height)
    };
    assert_eq!(reported(&mut fixture), drawn(slot, SHRINK), "short");

    fixture.run(Step::Redraw {
        window: 0,
        shrink: (0, 0),
    });
    assert_eq!(reported(&mut fixture), slot, "full");

    // Past the slot. The core learns a width the window refuses to shrink
    // below (a frame wider than asked widens its column -- `observe_frame`),
    // so the slot itself may move; a column's height is the output's, so
    // the extra height stays past the slot and is what gets clamped.
    fixture.run(Step::Redraw {
        window: 0,
        shrink: (-20, -30),
    });
    let reported_now = reported(&mut fixture);
    let slot_now = fixture.placement();
    assert_eq!(reported_now, slot_now, "past the slot");
    assert!(
        slot_now.h < slot.h + 30,
        "the test needs a frame that really is taller than its slot: {slot_now:?}"
    );
}

/// A client bound to `xdg_wm_base` below version 2 is never sent a tiled
/// state (they do not exist at its version; Smithay filters them per
/// toplevel), though scoot sets them for every layout window.
#[test]
fn a_version_one_client_is_never_sent_tiled_states() {
    let mut fixture = Fixture::with_radius(0);
    fixture.run(Step::Window { color: WINDOW_BGRA });
    fixture.run(Step::Window { color: WINDOW_BGRA });
    match fixture.run(Step::SawTiled) {
        Ack::SawTiled(saw) => assert!(!saw, "a v1 client was sent a tiled state"),
        other => panic!("unexpected ack {other:?}"),
    }
}
