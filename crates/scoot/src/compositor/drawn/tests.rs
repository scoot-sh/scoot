//! [`clamp_to_slot`]'s edge cases. The end-to-end behaviour -- the clip, the
//! ring and IPC `rect` following a real client's commits -- is pinned in
//! `rounded/tests/committed.rs`.

use super::*;

const SLOT: Rect = Rect::new(12, 30, 400, 300);

#[test]
fn a_client_that_fills_its_slot_reports_the_slot() {
    assert_eq!(clamp_to_slot(SLOT, 400, 300), SLOT);
}

#[test]
fn a_short_client_keeps_the_slot_origin_and_its_own_size() {
    assert_eq!(clamp_to_slot(SLOT, 387, 293), Rect::new(12, 30, 387, 293));
}

#[test]
fn each_axis_clamps_on_its_own() {
    assert_eq!(clamp_to_slot(SLOT, 500, 200), Rect::new(12, 30, 400, 200));
    assert_eq!(clamp_to_slot(SLOT, 100, 900), Rect::new(12, 30, 100, 300));
}

#[test]
fn a_client_past_its_slot_reports_the_slot() {
    assert_eq!(clamp_to_slot(SLOT, i32::MAX, i32::MAX), SLOT);
}

#[test]
fn nothing_committed_reports_the_slot() {
    for (w, h) in [(0, 0), (0, 300), (400, 0), (-1, 50), (50, i32::MIN)] {
        assert_eq!(clamp_to_slot(SLOT, w, h), SLOT, "committed {w}x{h}");
    }
}

#[test]
fn no_window_reports_the_slot() {
    assert_eq!(drawn_rect(SLOT, None), SLOT);
}
