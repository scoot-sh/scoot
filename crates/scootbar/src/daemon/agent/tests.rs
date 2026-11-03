//! The geometry `layout` reports, and the bounds of a reply.

use super::*;
use crate::bar::{Bar, Edge, Margin};
use crate::control::protocol::ModuleView;
use scootui::paint::Pad;

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
        Pad::ZERO,
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
        Pad::ZERO,
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
        Pad::ZERO,
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
        Pad::ZERO,
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
        Pad::ZERO,
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
        Pad::ZERO,
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
        0,
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
        module_rect(40, 60, Scale::Integer(1), bar, 0),
        Rect {
            x: 140,
            y: 50,
            width: 60,
            height: 28
        }
    );
    // Scale 2: device pixels halved.
    assert_eq!(
        module_rect(40, 60, Scale::Integer(2), bar, 0),
        Rect {
            x: 120,
            y: 50,
            width: 30,
            height: 28
        }
    );
    // Scale 1.5: 7..20 device is 4.67..13.33 logical: out to 4..14.
    let rect = module_rect(7, 13, Scale::Fractional(180), bar, 0);
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
            let rect = module_rect(x, width, scale, bar, 0);
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
    let rect = module_rect(90, 50, Scale::Integer(1), bar, 0);
    assert_eq!(rect.x + rect.width as i32, 100);
}

#[test]
fn a_margined_top_bar_with_side_room_reports_its_own_rect() {
    // Soft shadow on a top bar with margin 12: the grown room is top 8,
    // sides 12, window-facing bottom 16 (see `Style::shadow_room`). The
    // surface is the bar plus that room: 800 wide (sent sides 0), 52 high.
    let room = Pad {
        top: 8,
        right: 12,
        bottom: 16,
        left: 12,
    };
    let rect = bar_rect(
        &bar(Edge::Top, "12"),
        (0, 0),
        size(800, 600),
        size(800, 52),
        room,
    );
    assert_eq!(
        rect,
        Rect {
            x: 12,
            y: 12,
            width: 776,
            height: 28,
        }
    );
    // A module at the bar's local origin sits at the bar, not shifted by
    // the side room: its surface span starts at the room.
    let module = module_rect(12, 100, Scale::Integer(1), rect, room.left);
    assert_eq!(
        module,
        Rect {
            x: 12,
            y: 12,
            width: 100,
            height: 28,
        }
    );
}

#[test]
fn a_margined_bottom_bar_with_a_large_margin_reports_its_own_row() {
    // Deep shadow on a bottom bar with margin 100: the grown room is top
    // 38 (window-facing), sides 28, bottom 18 (see `Style::shadow_room`).
    // Sent margins are 72 on the sides, 82 on the anchored edge, so the
    // surface is 656x84 and the bar is 600x28 at y 472.
    let room = Pad {
        top: 38,
        right: 28,
        bottom: 18,
        left: 28,
    };
    let rect = bar_rect(
        &bar(Edge::Bottom, "100"),
        (0, 0),
        size(800, 600),
        size(656, 84),
        room,
    );
    assert_eq!(
        rect,
        Rect {
            x: 100,
            y: 472,
            width: 600,
            height: 28,
        }
    );
    // Its centre is inside the bar, not up in the shadow: the old surface
    // geometry put y at 416 with height 84 (centre 458, in the shadow).
    assert_eq!((rect.y, rect.height), (472, 28));
    assert!((472..500).contains(&(rect.y + rect.height as i32 / 2)));
    // A module spanning the bar's full width stays on the bar's row.
    let module = module_rect(28, 600, Scale::Integer(1), rect, room.left);
    assert_eq!(
        module,
        Rect {
            x: 100,
            y: 472,
            width: 600,
            height: 28,
        }
    );
}

#[test]
fn a_margin_smaller_than_the_shadow_room_clips_the_side_room() {
    // Glow shadow (16 everywhere) with margin 4: the side and anchored
    // room clip to 4, the window-facing bottom stays 16.
    let room = Pad {
        top: 4,
        right: 4,
        bottom: 16,
        left: 4,
    };
    let rect = bar_rect(
        &bar(Edge::Top, "4"),
        (0, 0),
        size(800, 600),
        size(800, 48),
        room,
    );
    assert_eq!(
        rect,
        Rect {
            x: 4,
            y: 4,
            width: 792,
            height: 28,
        }
    );
    let module = module_rect(4, 50, Scale::Integer(1), rect, room.left);
    assert_eq!(module.x, 4);
    assert_eq!((module.y, module.height), (4, 28));
}

#[test]
fn module_rect_with_side_room_starts_at_the_surface_origin_at_any_scale() {
    // The bar's true rect with 12 px of side room: the surface origin is
    // x 0, the bar at 12. Spans are in surface coordinates, so a span at
    // the room offset is the bar's local origin at every scale.
    let bar = Rect {
        x: 12,
        y: 12,
        width: 776,
        height: 28,
    };
    // Scale 1: device 12..112 is logical 12..112, minus the surface origin.
    assert_eq!(
        module_rect(12, 100, Scale::Integer(1), bar, 12),
        Rect {
            x: 12,
            y: 12,
            width: 100,
            height: 28
        }
    );
    // Scale 2: device pixels halved: 24..224 device is 12..112 logical.
    assert_eq!(
        module_rect(24, 200, Scale::Integer(2), bar, 12),
        Rect {
            x: 12,
            y: 12,
            width: 100,
            height: 28
        }
    );
    // Scale 1.5: 18..168 device is 12..112 logical.
    assert_eq!(
        module_rect(18, 150, Scale::Fractional(180), bar, 12),
        Rect {
            x: 12,
            y: 12,
            width: 100,
            height: 28
        }
    );
    // Fractional rounding still holds: 7..20 bar-local device at 1.5x is
    // 4..14 logical, offset by the 12 px room (25..38 surface device is
    // 16..26 logical, i.e. bar-local 4..14).
    let rect = module_rect(25, 13, Scale::Fractional(180), bar, 12);
    assert_eq!((rect.x - bar.x, rect.width), (4, 10));
    assert_eq!((rect.y, rect.height), (12, 28));
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
