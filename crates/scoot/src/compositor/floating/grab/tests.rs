//! The grab's pure arithmetic: which edges a modifier resize takes, what a
//! client's resize edge means, and the size a drag asks for.

use super::*;

const WINDOW: Rect = Rect::new(100, 100, 300, 300);

fn at(x: f64, y: f64) -> Point<f64, Logical> {
    Point::from((x, y))
}

fn edges(left: bool, right: bool, top: bool, bottom: bool) -> Edges {
    Edges {
        left,
        right,
        top,
        bottom,
    }
}

#[test]
fn a_modifier_resize_takes_the_nearest_edge_or_corner() {
    // Corners.
    assert_eq!(
        nearest_edges(WINDOW, at(110.0, 110.0)),
        edges(true, false, true, false)
    );
    assert_eq!(
        nearest_edges(WINDOW, at(390.0, 390.0)),
        edges(false, true, false, true)
    );
    // Edge middles.
    assert_eq!(
        nearest_edges(WINDOW, at(250.0, 110.0)),
        edges(false, false, true, false)
    );
    assert_eq!(
        nearest_edges(WINDOW, at(110.0, 250.0)),
        edges(true, false, false, false)
    );
    assert_eq!(
        nearest_edges(WINDOW, at(390.0, 250.0)),
        edges(false, true, false, false)
    );
    assert_eq!(
        nearest_edges(WINDOW, at(250.0, 390.0)),
        edges(false, false, false, true)
    );
    // The middle third: the nearest corner, never nothing.
    assert_eq!(
        nearest_edges(WINDOW, at(240.0, 240.0)),
        edges(true, false, true, false)
    );
    assert_eq!(
        nearest_edges(WINDOW, at(260.0, 260.0)),
        edges(false, true, false, true)
    );
    assert_eq!(
        nearest_edges(WINDOW, at(250.0, 250.0)),
        edges(false, true, false, true)
    );
}

#[test]
fn a_degenerate_window_still_resizes_something() {
    for rect in [Rect::new(0, 0, 0, 0), Rect::new(5, 5, -3, 1)] {
        let found = nearest_edges(rect, at(5.0, 5.0));
        assert!(
            found.left || found.right || found.top || found.bottom,
            "{rect:?}"
        );
    }
}

#[test]
fn a_client_resize_edge_names_its_edges_and_none_names_nothing() {
    use xdg_toplevel::ResizeEdge;
    assert_eq!(requested_edges(ResizeEdge::None), None);
    assert_eq!(
        requested_edges(ResizeEdge::Top),
        Some(edges(false, false, true, false))
    );
    assert_eq!(
        requested_edges(ResizeEdge::BottomRight),
        Some(edges(false, true, false, true))
    );
    assert_eq!(
        requested_edges(ResizeEdge::TopLeft),
        Some(edges(true, false, true, false))
    );
    assert_eq!(
        requested_edges(ResizeEdge::Left),
        Some(edges(true, false, false, false))
    );
}

#[test]
fn a_resize_drag_moves_the_dragged_edges_with_the_pointer() {
    let start = Rect::new(0, 0, 200, 100);
    assert_eq!(
        resized(start, edges(false, true, false, true), 30, 20),
        Size::new(230, 120)
    );
    assert_eq!(
        resized(start, edges(true, false, true, false), 30, 20),
        Size::new(170, 80)
    );
    // An axis with no dragged edge keeps its size.
    assert_eq!(
        resized(start, edges(false, false, false, true), 500, 20),
        Size::new(200, 120)
    );
    // Saturating, never overflowing: the core clamps what comes out.
    assert_eq!(
        resized(start, edges(true, false, false, false), i32::MIN, 0).w,
        i32::MAX
    );
}

#[test]
fn every_drag_has_a_cursor() {
    assert_eq!(drag_cursor(Drag::Move), CursorIcon::Grabbing);
    assert_eq!(
        drag_cursor(Drag::Resize(edges(true, false, true, false))),
        CursorIcon::NwResize
    );
    assert_eq!(
        drag_cursor(Drag::Resize(edges(false, true, false, true))),
        CursorIcon::SeResize
    );
    assert_eq!(
        drag_cursor(Drag::Resize(edges(false, false, false, true))),
        CursorIcon::SResize
    );
    assert_eq!(
        drag_cursor(Drag::Resize(edges(true, false, false, false))),
        CursorIcon::WResize
    );
}
