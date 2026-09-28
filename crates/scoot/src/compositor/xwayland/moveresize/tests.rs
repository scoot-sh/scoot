//! The pure halves of `moveresize.rs`; the gate itself runs against a live
//! X server in `xwayland/tests/moveresize.rs`.

use smithay::xwayland::xwm::ResizeEdge;

use super::{x_button_code, x11_edges};

#[test]
fn x_buttons_map_to_the_evdev_codes_xwayland_numbers_them_from() {
    assert_eq!(x_button_code(1), Some(0x110), "left");
    assert_eq!(x_button_code(2), Some(0x112), "middle");
    assert_eq!(x_button_code(3), Some(0x111), "right");
    // The scroll axes are never a held button, and 0 names none.
    for scroll in [0, 4, 5, 6, 7] {
        assert_eq!(x_button_code(scroll), None, "{scroll}");
    }
    assert_eq!(x_button_code(8), Some(0x113), "BTN_SIDE");
    assert_eq!(x_button_code(9), Some(0x114), "BTN_EXTRA");
    // A client's number, so no overflow on the largest one.
    assert_eq!(x_button_code(u32::MAX), None);
}

#[test]
fn every_resize_edge_moves_the_edges_it_names() {
    let named = |edge| {
        let edges = x11_edges(edge);
        (edges.left, edges.right, edges.top, edges.bottom)
    };
    assert_eq!(named(ResizeEdge::Top), (false, false, true, false));
    assert_eq!(named(ResizeEdge::Bottom), (false, false, false, true));
    assert_eq!(named(ResizeEdge::Left), (true, false, false, false));
    assert_eq!(named(ResizeEdge::Right), (false, true, false, false));
    assert_eq!(named(ResizeEdge::TopLeft), (true, false, true, false));
    assert_eq!(named(ResizeEdge::TopRight), (false, true, true, false));
    assert_eq!(named(ResizeEdge::BottomLeft), (true, false, false, true));
    assert_eq!(named(ResizeEdge::BottomRight), (false, true, false, true));
}
