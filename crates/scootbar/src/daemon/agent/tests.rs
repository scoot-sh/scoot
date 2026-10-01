//! The geometry `layout` reports, and the bounds of a reply.

use super::*;
use crate::bar::{Bar, Edge, Margin};
use crate::control::protocol::ModuleView;

fn bar(edge: Edge, margin: &str) -> Bar {
    Bar {
        edge,
        height: 28,
        margin: Margin::parse(margin).unwrap(),
        ..Bar::default()
    }
}

fn size(width: u32, height: u32) -> Size {
    Size { width, height }
}

#[test]
fn a_top_bar_sits_at_the_top_left_of_its_output() {
    let rect = bar_rect(
        &bar(Edge::Top, "0"),
        (0, 0),
        size(1600, 1000),
        size(1600, 28),
    );
    assert_eq!(
        rect,
        Rect {
            x: 0,
            y: 0,
            width: 1600,
            height: 28
        }
    );
}

#[test]
fn a_bottom_bar_sits_against_the_bottom() {
    let rect = bar_rect(
        &bar(Edge::Bottom, "0"),
        (0, 0),
        size(1600, 1000),
        size(1600, 28),
    );
    assert_eq!((rect.y, rect.height), (972, 28));
}

#[test]
fn the_origin_and_the_margins_move_it() {
    // A second output at x 1600, a floating bar 8 in from the top, 12 in
    // from the sides.
    let rect = bar_rect(
        &bar(Edge::Top, "8,12"),
        (1600, 20),
        size(1600, 1000),
        size(1576, 28),
    );
    assert_eq!(
        rect,
        Rect {
            x: 1612,
            y: 28,
            width: 1576,
            height: 28
        }
    );
    // The same at the bottom: the bottom margin is above the bar.
    let rect = bar_rect(
        &bar(Edge::Bottom, "8,12"),
        (1600, 20),
        size(1600, 1000),
        size(1576, 28),
    );
    assert_eq!(rect.y, 20 + 1000 - 8 - 28);
}

#[test]
fn an_output_left_of_the_origin_has_a_negative_x() {
    let rect = bar_rect(
        &bar(Edge::Top, "0"),
        (-1600, 0),
        size(1600, 1000),
        size(1600, 28),
    );
    assert_eq!(rect.x, -1600);
}

#[test]
fn hostile_sizes_do_not_overflow() {
    let rect = bar_rect(
        &bar(Edge::Bottom, "0"),
        (i32::MIN, i32::MAX),
        size(u32::MAX, 0),
        size(u32::MAX, u32::MAX),
    );
    // Whatever it is, it did not panic and stays in range.
    let _ = rect;
    let rect = module_rect(
        u32::MAX,
        u32::MAX,
        Scale::Fractional(1),
        Rect {
            x: i32::MAX,
            y: 0,
            width: u32::MAX,
            height: 1,
        },
    );
    let _ = rect;
}

#[test]
fn a_module_rect_is_its_span_in_logical_pixels_rounded_outward() {
    let bar = Rect {
        x: 100,
        y: 50,
        width: 1000,
        height: 28,
    };
    // Scale 1: the span as it is, on the bar.
    assert_eq!(
        module_rect(40, 60, Scale::Integer(1), bar),
        Rect {
            x: 140,
            y: 50,
            width: 60,
            height: 28
        }
    );
    // Scale 2: device pixels halved.
    assert_eq!(
        module_rect(40, 60, Scale::Integer(2), bar),
        Rect {
            x: 120,
            y: 50,
            width: 30,
            height: 28
        }
    );
    // Scale 1.5: 7..20 device is 4.67..13.33 logical: out to 4..14.
    let rect = module_rect(7, 13, Scale::Fractional(180), bar);
    assert_eq!((rect.x - bar.x, rect.width), (4, 10));
}

#[test]
fn every_device_pixel_of_a_span_is_inside_its_rect() {
    // The property an agent aiming at the middle of a rect relies on: a
    // pointer anywhere on a drawn pixel of the module is inside its rect,
    // at every scale the bar draws at.
    let bar = Rect {
        x: 0,
        y: 0,
        width: 4000,
        height: 28,
    };
    for scale in [
        Scale::Integer(1),
        Scale::Integer(2),
        Scale::Integer(3),
        Scale::Fractional(120),
        Scale::Fractional(150),
        Scale::Fractional(180),
        Scale::Fractional(210),
        Scale::Fractional(300),
    ] {
        for (x, width) in [(0, 1), (1, 1), (7, 13), (100, 250), (333, 7), (999, 3)] {
            let rect = module_rect(x, width, scale, bar);
            for pixel in x..x + width {
                // The pixel's logical extent is [pixel/s, (pixel+1)/s).
                let lo = f64::from(pixel) / scale.factor();
                let hi = f64::from(pixel + 1) / scale.factor();
                assert!(
                    f64::from(rect.x) <= lo + 1e-9
                        && hi <= f64::from(rect.x) + f64::from(rect.width) + 1e-9,
                    "{scale:?} span {x}+{width}: pixel {pixel} at {lo}..{hi} not in {rect:?}"
                );
            }
        }
    }
}

#[test]
fn a_rect_is_cut_to_the_bar() {
    let bar = Rect {
        x: 0,
        y: 0,
        width: 100,
        height: 28,
    };
    let rect = module_rect(90, 50, Scale::Integer(1), bar);
    assert_eq!(rect.x + rect.width as i32, 100);
}

#[test]
fn a_trigger_is_named_by_its_key_without_on() {
    assert_eq!(trigger_named("click"), Some(Trigger::Click));
    assert_eq!(trigger_named("right-click"), Some(Trigger::RightClick));
    assert_eq!(trigger_named("middle-click"), Some(Trigger::MiddleClick));
    assert_eq!(trigger_named("scroll-up"), Some(Trigger::ScrollUp));
    assert_eq!(trigger_named("scroll-down"), Some(Trigger::ScrollDown));
    assert_eq!(trigger_named("on-click"), None);
    assert_eq!(trigger_named("double-click"), None);
    assert_eq!(trigger_named(""), None);
}

#[test]
fn the_worst_module_entry_is_far_inside_the_reply_bound() {
    // Every module at the longest text and tooltip (every character one
    // that JSON escapes to two bytes), a long output name and the largest
    // value, on the most outputs a bar serves: the reply stays inside the
    // bound by arithmetic, so the guard in `write_query` is a backstop.
    let escaped = "\"".repeat(crate::modules::MAX_TEXT);
    let name = "o".repeat(64);
    let view = ModuleView {
        id: &"i".repeat(crate::modules::custom::MAX_NAME),
        section: "center",
        output: Some(&name),
        text: &escaped,
        class: "normal",
        icon: Some('\u{10ffff}'),
        tooltip: &escaped,
        value: Some(serde_json::json!({
            "active": 4_294_967_295u64,
            "workspaces": vec![4_294_967_295u64; 32],
        })),
    };
    let one = serde_json::to_vec(&view).unwrap().len();
    let outputs = 8;
    assert!(
        one * crate::layout::MAX_MODULES * outputs < MAX_REPLY,
        "{one} bytes an entry"
    );
}
