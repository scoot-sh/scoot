//! The popup's unit tests: what needs no compositor (anchors, icon
//! refusal, hit-testing, re-theme). The compositor's half is the
//! integration tests' (`tests/popups.rs`).

use super::*;
use crate::bus::hints::Urgency;
use crate::config::Anchor as StackAnchor;

#[test]
fn anchors_map_to_their_corners() {
    assert_eq!(anchor_of(StackAnchor::TopLeft), Anchor::Top | Anchor::Left);
    assert_eq!(anchor_of(StackAnchor::TopRight), Anchor::Top | Anchor::Right);
    assert_eq!(anchor_of(StackAnchor::BottomLeft), Anchor::Bottom | Anchor::Left);
    assert_eq!(
        anchor_of(StackAnchor::BottomRight),
        Anchor::Bottom | Anchor::Right
    );
}

#[test]
fn icon_decode_refuses_garbage() {
    let mut note = crate::model::store::Notification {
        id: 1,
        app_name: "a".to_owned(),
        sender: ":1.1".to_owned(),
        icon: crate::model::store::Icon::None,
        summary: String::new(),
        body: String::new(),
        plain_body: String::new(),
        actions: Vec::new(),
        urgency: Urgency::Normal,
        category: String::new(),
        desktop_entry: String::new(),
        transient: false,
        resident: false,
        value: None,
        seq: 1,
        timeout_ms: None,
        created_ms: 0,
        shown_ms: None,
        clock: crate::model::expiry::Clock::waiting(),
    };
    // No icon, and a theme name, decode to the tile (nothing).
    assert_eq!(decode_icon(&note), None);
    note.icon = crate::model::store::Icon::Name("firefox".to_owned());
    assert_eq!(decode_icon(&note), None);
    // Truncated pixels are refused, not read past the end.
    note.icon = crate::model::store::Icon::Data(crate::bus::image::Image {
        width: 4,
        height: 4,
        pixels: vec![255u8; 10],
    });
    assert_eq!(decode_icon(&note), None);
    // Absurd dimensions are refused before any allocation.
    note.icon = crate::model::store::Icon::Data(crate::bus::image::Image {
        width: 5000,
        height: 5000,
        pixels: Vec::new(),
    });
    assert_eq!(decode_icon(&note), None);
    // A missing path is a tile, not an error.
    note.icon =
        crate::model::store::Icon::Path("/nonexistent/scootnotify-test-icon.png".to_owned());
    assert_eq!(decode_icon(&note), None);
}

#[test]
fn a_2x2_image_decodes_to_the_square() {
    let note = crate::model::store::Notification {
        id: 1,
        app_name: "a".to_owned(),
        sender: ":1.1".to_owned(),
        icon: crate::model::store::Icon::Data(crate::bus::image::Image {
            // Solid opaque red, straight RGBA.
            width: 2,
            height: 2,
            pixels: vec![255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255],
        }),
        summary: String::new(),
        body: String::new(),
        plain_body: String::new(),
        actions: Vec::new(),
        urgency: Urgency::Normal,
        category: String::new(),
        desktop_entry: String::new(),
        transient: false,
        resident: false,
        value: None,
        seq: 1,
        timeout_ms: None,
        created_ms: 0,
        shown_ms: None,
        clock: crate::model::expiry::Clock::waiting(),
    };
    let bits = decode_icon(&note).expect("a tiny image decodes");
    assert_eq!(bits.side, ICON_SIDE);
    assert_eq!(bits.pixels.len(), ICON_SIDE as usize * ICON_SIDE as usize * 4);
    // Premultiplied BGRA red, opaque: every pixel reads back red.
    assert!(bits.pixels.chunks_exact(4).all(|pixel| pixel == [0, 0, 255, 255]));
}

#[test]
fn hit_tests_buttons_before_bodies() {
    let app = App::new(Config::default(), Look::default(), None);
    let mut app = app;
    app.stack = Stack {
        width: 360,
        height: 200,
        cards: vec![crate::view::layout::Card {
            id: 7,
            rect: crate::view::layout::Rect {
                x: 0,
                y: 0,
                width: 360,
                height: 100,
            },
            summary: Vec::new(),
            meta: crate::view::layout::Line {
                runs: Vec::new(),
                x: 0,
                baseline: 0,
            },
            body: Vec::new(),
            body_hit: crate::view::layout::Rect {
                x: 12,
                y: 12,
                width: 336,
                height: 76,
            },
            buttons: vec![crate::view::layout::Button {
                key: "open".to_owned(),
                rect: crate::view::layout::Rect {
                    x: 12,
                    y: 60,
                    width: 100,
                    height: 28,
                },
                label: "Open".to_owned(),
            }],
            progress: None,
            progress_bar: None,
            has_icon: false,
            tile: 'T',
        }],
        hidden: 0,
        pill_label: String::new(),
        input: Vec::new(),
    };
    app.pointer_in = true;
    // On the button: the button, not the body.
    (app.pointer_x, app.pointer_y) = (20, 70);
    assert_eq!(app.hit(), Some((7, Some(0))));
    // On the body outside the button: the body.
    (app.pointer_x, app.pointer_y) = (200, 30);
    assert_eq!(app.hit(), Some((7, None)));
    // In the gap below the card: nothing (it clicks through).
    (app.pointer_x, app.pointer_y) = (200, 150);
    assert_eq!(app.hit(), None);
    // Pointer left the surface: nothing.
    app.pointer_in = false;
    (app.pointer_x, app.pointer_y) = (20, 70);
    assert_eq!(app.hit(), None);
}

#[test]
fn retheme_swaps_the_look_and_marks_dirty() {
    // `retheme` needs no compositor: it swaps the look and forces the
    // next refresh to rebuild (the stack comparison would call equal
    // sizes equal and skip the draw).
    let mut app = App::new(Config::default(), Look::default(), None);
    app.dirty = false;
    app.content_size = Some(scootui::output::Size { width: 360, height: 100 });
    let mut new = Look::default();
    new.font_size_px = 20.0;
    app.retheme(new);
    assert_eq!(app.look.font_size_px, 20.0);
    assert!(app.dirty);
    assert_eq!(app.content_size, None);
}
