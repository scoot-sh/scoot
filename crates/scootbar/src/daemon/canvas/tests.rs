use super::{SlotState, pick};
use crate::density::Scale;
use crate::outputs::{Frame, Size};

fn frame(width: u32, scale: u32) -> Frame {
    Frame {
        size: Size { width, height: 28 },
        scale: Scale::Integer(scale),
    }
}

fn dims(frame: Frame) -> (u32, u32) {
    frame.scale.buffer(frame.size).unwrap()
}

fn slot(frame: Frame, held: bool) -> Option<SlotState> {
    Some(SlotState {
        dims: dims(frame),
        held,
        painted: Some(frame),
    })
}

fn choose(slots: [Option<SlotState>; 2], wanted: Frame) -> Option<usize> {
    pick(slots.into_iter(), wanted, dims(wanted))
}

#[test]
fn the_first_draw_takes_the_first_empty_slot() {
    assert_eq!(choose([None, None], frame(1920, 1)), Some(0));
}

#[test]
fn a_redraw_while_the_first_is_held_takes_the_second() {
    let a = frame(1920, 1);
    let b = frame(1280, 1);
    assert_eq!(choose([slot(a, true), None], b), Some(1));
}

#[test]
fn every_buffer_held_stalls() {
    let a = frame(1920, 1);
    let b = frame(1280, 1);
    assert_eq!(choose([slot(a, true), slot(b, true)], a), None);
}

#[test]
fn a_free_buffer_showing_the_frame_is_reused_as_it_is() {
    let a = frame(1920, 1);
    let b = frame(1920, 2);
    // The scale flipped back: slot 1 still shows it.
    assert_eq!(choose([slot(b, true), slot(a, false)], a), Some(1));
    // Preferred over an empty slot and over a free slot of another frame.
    assert_eq!(choose([None, slot(a, false)], a), Some(1));
}

#[test]
fn a_free_buffer_of_the_right_size_is_preferred_over_a_new_one() {
    let a = frame(1920, 1);
    // Same size, other content (a later color, when modules come).
    let same_size = Some(SlotState {
        dims: dims(a),
        held: false,
        painted: None,
    });
    assert_eq!(choose([None, same_size], a), Some(1));
}

#[test]
fn a_free_buffer_of_the_wrong_size_is_taken_last() {
    let old = frame(1920, 1);
    let new = frame(1280, 1);
    // An empty slot first: the stale one may still be wanted back.
    assert_eq!(choose([slot(old, false), None], new), Some(1));
    // With no empty slot, the stale one is replaced.
    assert_eq!(choose([slot(old, false), slot(old, true)], new), Some(0));
}

#[test]
fn a_held_buffer_is_never_chosen_even_showing_the_frame() {
    let a = frame(1920, 1);
    assert_eq!(choose([slot(a, true), None], a), Some(1));
}
