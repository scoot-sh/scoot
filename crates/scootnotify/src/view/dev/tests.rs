use scootui::density::Scale;
use scootui::output::Size;

use super::Dev;
use crate::view::layout::Rect;

/// The scales a compositor sends: integer, the fractional ones laptops
/// ship with, and an odd one.
fn scales() -> [Scale; 8] {
    [
        Scale::Integer(1),
        Scale::Fractional(120),
        Scale::Fractional(150),
        Scale::Fractional(180),
        Scale::Fractional(210),
        Scale::Integer(2),
        Scale::Integer(3),
        Scale::Fractional(133),
    ]
}

#[test]
fn scale_one_is_the_identity() {
    let dev = Dev::new(Scale::Integer(1));
    assert!(dev.is_unit() && Dev::UNIT.is_unit());
    for value in [0u32, 1, 7, 360, 4000] {
        assert_eq!(dev.len(value), value);
        assert_eq!(dev.x(i64::from(value)), i64::from(value));
        assert_eq!(dev.y_at(value), value);
    }
    assert_eq!(dev.em(13.0).to_bits(), 13.0f32.to_bits());
    assert_eq!(dev.em(0.5).to_bits(), 0.5f32.to_bits());
    let rect = Rect {
        x: 3,
        y: 4,
        width: 5,
        height: 6,
    };
    assert_eq!(dev.rect(rect), rect);
    // An origin does not move anything at scale 1.
    assert_eq!(dev.from_origin(17, 9).x(5), 5);
    assert_eq!(dev.from_origin(17, 9).y(5), 5);
}

#[test]
fn the_far_edge_of_a_stack_is_the_buffers() {
    // The surface's logical size maps to the buffer the protocol asks for,
    // whatever the scale, so a card at the edge never falls short of it.
    for scale in scales() {
        let dev = Dev::new(scale);
        for width in 1..=500u32 {
            let (buffer, _) = scale
                .buffer(Size { width, height: 1 })
                .expect("a small size fits");
            assert_eq!(dev.x_at(width), buffer, "{width} at {scale}");
        }
    }
}

#[test]
fn a_length_does_not_depend_on_where_it_sits() {
    let dev = Dev::new(Scale::Fractional(150));
    let widths: Vec<u32> = (0..40)
        .map(|x| {
            dev.rect(Rect {
                x,
                y: 0,
                width: 21,
                height: 9,
            })
            .width
        })
        .collect();
    assert!(widths.iter().all(|width| *width == widths[0]), "{widths:?}");
}

#[test]
fn a_thin_line_stays_visible_and_zero_stays_zero() {
    for scale in scales() {
        let dev = Dev::new(scale);
        assert_eq!(dev.thick(0), 0);
        assert!(dev.thick(1) >= 1, "{scale}");
    }
    assert_eq!(Dev::new(Scale::Fractional(60)).thick(1), 1);
    assert_eq!(Dev::new(Scale::Fractional(60)).len(1), 1);
}

#[test]
fn an_origin_puts_a_layer_where_the_static_paint_puts_it() {
    // A layer is composited at d(origin) and holds the card at
    // d(x + origin) - d(origin): the same device pixel as the static paint.
    for scale in scales() {
        let stack = Dev::new(scale);
        for origin in 0..30u32 {
            let layer = stack.from_origin(origin, origin);
            for local in 0..40u32 {
                let global = i64::from(origin + local);
                assert_eq!(
                    stack.x(i64::from(origin)) + layer.x(i64::from(local)),
                    stack.x(global),
                    "{local} from {origin} at {scale}"
                );
                assert_eq!(
                    stack.y(i64::from(origin)) + layer.y(i64::from(local)),
                    stack.y(global)
                );
            }
        }
    }
}

#[test]
fn nothing_wraps_at_the_extremes() {
    let dev = Dev::new(Scale::Fractional(1920));
    assert_eq!(dev.len(u32::MAX), u32::MAX);
    assert_eq!(dev.x_at(u32::MAX), u32::MAX);
    assert_eq!(dev.x(i64::MIN), i64::MIN);
    // A negative position is off the surface, not wrapped to a huge one.
    assert_eq!(
        Dev::new(Scale::Fractional(150)).from_origin(0, 0).x_at(0),
        0
    );
    let negative = Dev::new(Scale::Fractional(150));
    assert!(negative.x(-5) < 0);
}
