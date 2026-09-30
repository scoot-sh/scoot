use super::{Pill, Shape};
use crate::density::Scale;

const ONE: Scale = Scale::Integer(1);

fn pill(shape: Shape) -> Pill {
    Pill {
        shape,
        radius: 0,
        inset: 0,
    }
}

#[test]
fn the_default_is_the_square_full_height_pill_it_always_was() {
    let pill = Pill::default();
    assert_eq!(pill.shape, Shape::Rect);
    assert_eq!(pill.rows(28, 16, ONE), (0, 28));
    assert_eq!(pill.radius(ONE), 0);
    assert_eq!(pill.extent((10, 30), (0, 100), (0, 28)), (10, 30));
}

#[test]
fn shapes_are_read_by_name_and_only_by_name() {
    assert_eq!(Shape::parse("rect"), Some(Shape::Rect));
    assert_eq!(Shape::parse("pill"), Some(Shape::Pill));
    assert_eq!(Shape::parse("circle"), Some(Shape::Circle));
    for bad in ["", "Circle", "round", "dot", " pill", "pill "] {
        assert_eq!(Shape::parse(bad), None, "{bad:?}");
    }
}

#[test]
fn the_inset_is_cut_back_to_keep_the_text_line() {
    let inset = |inset, height, line| {
        Pill {
            inset,
            ..Pill::default()
        }
        .rows(height, line, ONE)
    };
    assert_eq!(inset(4, 28, 12), (4, 24));
    // A 28-high bar with a 20-high line leaves 4 a side.
    assert_eq!(inset(50, 28, 20), (4, 24));
    // A line as tall as the bar leaves none; taller, none either.
    assert_eq!(inset(9, 28, 28), (0, 28));
    assert_eq!(inset(9, 28, 90), (0, 28));
    assert_eq!(inset(u32::MAX, 0, 0), (0, 0));
    // The inset scales.
    let p = Pill {
        inset: 3,
        ..Pill::default()
    };
    assert_eq!(p.rows(60, 10, Scale::Integer(2)), (6, 54));
}

#[test]
fn only_a_rect_has_its_own_radius() {
    let radius = |shape| {
        Pill {
            shape,
            radius: 5,
            inset: 0,
        }
        .radius(Scale::Integer(2))
    };
    assert_eq!(radius(Shape::Rect), 10);
    assert_eq!(radius(Shape::Pill), u32::MAX);
    assert_eq!(radius(Shape::Circle), u32::MAX);
}

#[test]
fn a_pill_and_a_rect_keep_the_items_extent() {
    for shape in [Shape::Rect, Shape::Pill] {
        assert_eq!(pill(shape).extent((10, 20), (0, 100), (0, 28)), (10, 20));
    }
}

#[test]
fn a_circle_grows_to_its_height_centered_on_the_number() {
    // A 10-wide item on a 24-high pill: a 24-wide disc, centered.
    let circle = pill(Shape::Circle);
    assert_eq!(circle.extent((40, 50), (0, 200), (2, 26)), (33, 57));
    // Already wider than tall (two digits): the pill is as wide as the
    // text needs, not cut to a disc.
    assert_eq!(circle.extent((40, 80), (0, 200), (2, 26)), (40, 80));
    // Exactly its height stays.
    assert_eq!(circle.extent((40, 64), (0, 200), (2, 26)), (40, 64));
}

#[test]
fn a_circles_growth_stops_at_its_neighbours_and_slides_back_in() {
    let circle = pill(Shape::Circle);
    // Room only 20 wide around a 10-wide item: a 20-wide pill, not 24.
    assert_eq!(circle.extent((40, 50), (35, 55), (0, 24)), (35, 55));
    // Room lopsided (a neighbour close on the left): the disc slides right.
    assert_eq!(circle.extent((40, 50), (38, 80), (0, 24)), (38, 62));
    // Against the span's start.
    assert_eq!(circle.extent((0, 10), (0, 100), (0, 24)), (0, 24));
    // Room no wider than the item itself: the item's own extent.
    assert_eq!(circle.extent((40, 50), (40, 50), (0, 24)), (40, 50));
}

#[test]
fn a_circle_never_leaves_its_room_whatever_the_numbers() {
    let circle = pill(Shape::Circle);
    for lo in [0u32, 1, 7, 100, u32::MAX - 3] {
        for width in [0u32, 1, 9, 40, u32::MAX] {
            for diameter in [0u32, 1, 24, u32::MAX] {
                let natural = (lo, lo.saturating_add(width));
                let room = (lo.saturating_sub(5), natural.1.saturating_add(5));
                let (a, b) = circle.extent(natural, room, (0, diameter));
                assert!(a <= b, "{natural:?} {room:?} {diameter}: {a} {b}");
                assert!(
                    a >= room.0 && b <= room.1,
                    "{natural:?} {room:?} {diameter}"
                );
                assert!(b - a >= natural.1 - natural.0, "shrank below the item");
            }
        }
    }
}
