//! [`constrain`] on its own: the overflow bound, at and around its edge.
//!
//! These call the function directly because what they pin is arithmetic --
//! "no input inside the bound can overflow" -- which needs thousands of
//! inputs, not a client per case. Test builds check overflow, so a sum that
//! escapes `i32` anywhere in Smithay's constraint pass fails here as a panic.

use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_positioner::{
    Anchor as ServerAnchor, ConstraintAdjustment as ServerAdjust, Gravity as ServerGravity,
};
use smithay::utils::{Logical, Point, Rectangle, Size};
use smithay::wayland::shell::xdg::PositionerState;

use crate::compositor::popup_constraint::{COORDINATE_LIMIT, constrain};

const LIMIT: i32 = COORDINATE_LIMIT as i32;

const ANCHORS: [ServerAnchor; 9] = [
    ServerAnchor::None,
    ServerAnchor::Top,
    ServerAnchor::Bottom,
    ServerAnchor::Left,
    ServerAnchor::Right,
    ServerAnchor::TopLeft,
    ServerAnchor::BottomLeft,
    ServerAnchor::TopRight,
    ServerAnchor::BottomRight,
];

const GRAVITIES: [ServerGravity; 9] = [
    ServerGravity::None,
    ServerGravity::Top,
    ServerGravity::Bottom,
    ServerGravity::Left,
    ServerGravity::Right,
    ServerGravity::TopLeft,
    ServerGravity::BottomLeft,
    ServerGravity::TopRight,
    ServerGravity::BottomRight,
];

fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Logical> {
    Rectangle::new(Point::new(x, y), Size::new(w, h))
}

fn positioner(size: i32, anchor: i32, offset: i32) -> PositionerState {
    PositionerState {
        rect_size: Size::new(size, size),
        anchor_rect: rect(anchor, anchor, size, size),
        offset: Point::new(offset, offset),
        constraint_adjustment: ServerAdjust::all(),
        ..PositionerState::default()
    }
}

#[test]
fn every_input_at_the_limit_is_constrained_and_one_past_it_is_not() {
    let target = rect(0, 0, 200, 200);
    assert!(constrain(positioner(LIMIT, LIMIT, LIMIT), target).is_some());
    assert!(constrain(positioner(LIMIT, -LIMIT, -LIMIT), target).is_some());
    assert!(constrain(positioner(1, 0, LIMIT + 1), target).is_none());
    assert!(constrain(positioner(1, -LIMIT - 1, 0), target).is_none());
    assert!(constrain(positioner(LIMIT + 1, 0, 0), target).is_none());
    // `unsigned_abs`, not `abs`: `i32::MIN` has no positive counterpart.
    assert!(constrain(positioner(1, i32::MIN, 0), target).is_none());
    assert!(constrain(positioner(1, 0, i32::MIN), target).is_none());
    assert!(constrain(positioner(1, 0, 0), rect(i32::MIN, 0, 200, 200)).is_none());
    assert!(constrain(positioner(1, 0, 0), rect(0, LIMIT + 1, 200, 200)).is_none());
    assert!(constrain(positioner(1, 0, 0), rect(0, 0, i32::MAX, 200)).is_none());
}

/// Every anchor, gravity and adjustment combination, with every positioner
/// field and target coordinate at either extreme the bound admits: none of
/// it may overflow (which a test build turns into a panic).
#[test]
fn nothing_inside_the_limit_overflows() {
    let extremes = [-LIMIT, LIMIT];
    let mut calls = 0u32;
    for anchor in ANCHORS {
        for gravity in GRAVITIES {
            for bits in 0..=ServerAdjust::all().bits() {
                let adjust = ServerAdjust::from_bits_truncate(bits);
                for &at in &extremes {
                    for &offset in &extremes {
                        for &target_at in &extremes {
                            for target_size in [0, LIMIT] {
                                let positioner = PositionerState {
                                    rect_size: Size::new(LIMIT, LIMIT),
                                    anchor_rect: rect(at, at, LIMIT, LIMIT),
                                    anchor_edges: anchor,
                                    gravity,
                                    constraint_adjustment: adjust,
                                    offset: Point::new(offset, -offset),
                                    ..PositionerState::default()
                                };
                                let target = rect(target_at, -target_at, target_size, target_size);
                                assert!(constrain(positioner, target).is_some());
                                calls += 1;
                            }
                        }
                    }
                }
            }
        }
    }
    assert_eq!(calls, 9 * 9 * 64 * 16);
}
