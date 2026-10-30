//! The popup's unit tests: what needs no compositor (anchors, icon
//! refusal, hit-testing, re-theme). The compositor's half is the
//! integration tests' (`tests/popups.rs`).

use super::*;
use crate::bus::hints::Urgency;
use crate::config::Anchor as StackAnchor;

#[test]
fn anchors_map_to_their_corners() {
    assert_eq!(anchor_of(StackAnchor::TopLeft), Anchor::Top | Anchor::Left);
    assert_eq!(
        anchor_of(StackAnchor::TopRight),
        Anchor::Top | Anchor::Right
    );
    assert_eq!(
        anchor_of(StackAnchor::BottomLeft),
        Anchor::Bottom | Anchor::Left
    );
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
        keep_history: true,
        output: None,
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
            pixels: vec![
                255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255,
            ],
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
        keep_history: true,
        output: None,
        value: None,
        seq: 1,
        timeout_ms: None,
        created_ms: 0,
        shown_ms: None,
        clock: crate::model::expiry::Clock::waiting(),
    };
    let bits = decode_icon(&note).expect("a tiny image decodes");
    assert_eq!(bits.side, ICON_SIDE);
    assert_eq!(
        bits.pixels.len(),
        ICON_SIDE as usize * ICON_SIDE as usize * 4
    );
    // Premultiplied BGRA red, opaque: every pixel reads back red.
    assert!(
        bits.pixels
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| *pixel == [0, 0, 255, 255])
    );
}

#[test]
fn hit_tests_buttons_before_bodies() {
    let app = App::new(Config::default(), Look::default());
    let mut app = app;
    app.stack = Stack {
        width: 360,
        height: 200,
        pad: scootui::paint::Pad::ZERO,
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
    let mut app = App::new(Config::default(), Look::default());
    app.dirty = false;
    app.content_size = Some(scootui::output::Size {
        width: 360,
        height: 100,
    });
    let new = Look {
        font_size_px: 20.0,
        ..Look::default()
    };
    app.retheme(new);
    assert_eq!(app.look.font_size_px, 20.0);
    assert!(app.dirty);
    assert_eq!(app.content_size, None);
}

#[test]
fn the_default_look_has_no_shadow_and_the_configured_margins() {
    let app = App::new(Config::default(), Look::default());
    assert!(app.shadow.is_none());
    assert!(app.stack.pad.is_zero());
    let margin = app.config.margin_px;
    assert_eq!(app.margins(), [margin; 4]);
}

#[test]
fn a_shadows_room_comes_out_of_the_margin_and_never_goes_negative() {
    let soft = Look {
        shadow: crate::look::ShadowStyle::Soft,
        ..Look::default()
    };
    let mut app = App::new(Config::default(), soft);
    let shadow = app.shadow.as_ref().expect("soft builds a shadow");
    app.stack.pad = shadow.pad();
    // The default 12 px margin against soft's room (top 8, right 12,
    // bottom 16, left 12): the cards stay 12 px from the edge where the
    // room fits inside the margin, and sit further in where it does not.
    assert_eq!(app.stack.pad.top, 8);
    assert_eq!(app.margins(), [4, 0, 0, 0]);
    app.config.margin_px = 0;
    assert_eq!(app.margins(), [0; 4]);
    app.config.margin_px = 128;
    let pad = app.stack.pad;
    assert_eq!(
        app.margins(),
        [
            128 - pad.top,
            128 - pad.right,
            128 - pad.bottom,
            128 - pad.left
        ]
    );
}

#[test]
fn retheme_builds_a_new_style_and_keeps_an_unchanged_one() {
    use crate::look::ShadowStyle;
    let mut app = App::new(Config::default(), Look::default());
    app.shadow_stale = false;
    app.retheme(Look {
        shadow: ShadowStyle::Deep,
        ..Look::default()
    });
    assert!(app.shadow.is_some() && app.shadow_stale);
    // The corner tables are sized at the next refresh, from the stack.
    app.prepare_shadow();
    assert!(!app.shadow_stale);
    let tables = app.shadow.clone();
    app.retheme(Look {
        shadow: ShadowStyle::Deep,
        font_size_px: 20.0,
        ..Look::default()
    });
    assert_eq!(app.shadow, tables, "the same style keeps its tables");
    assert!(app.shadow_stale, "but is re-sized for the new look");
    app.retheme(Look::default());
    assert!(app.shadow.is_none(), "none drops the shadow");
}

#[test]
fn a_config_reload_resizes_the_shadow_for_the_new_radius() {
    let mut app = App::new(
        Config::default(),
        Look {
            shadow: crate::look::ShadowStyle::Soft,
            ..Look::default()
        },
    );
    app.shadow_stale = false;
    app.apply_config(Config {
        radius: 20,
        ..Config::default()
    });
    assert!(app.shadow_stale);
}

// --- motion ---------------------------------------------------------------

use crate::look::{CardMotion, MotionTokens};
use crate::view::render::tests::{make_text, note};
use scootui::motion::Curve;

fn moving_look(curve: Curve) -> Look {
    Look {
        motion: MotionTokens {
            curve,
            duration_ms: 200,
            enter: CardMotion::Slide,
            ..MotionTokens::default()
        },
        ..Look::default()
    }
}

fn stack_of(app: &App, ids: &[u32]) -> Stack {
    let notes: Vec<Notification> = ids
        .iter()
        .map(|&id| note(id, &format!("Card {id}"), "A body that wraps"))
        .collect();
    let live: Vec<&Notification> = notes.iter().collect();
    layout::layout(&live, &app.config, &make_text(), 13.0, 2_000)
}

#[test]
fn a_look_without_motion_never_builds_motion_state() {
    let mut app = App::new(Config::default(), Look::default());
    assert!(!app.motion.enabled() && !app.motion.active());
    let stack = stack_of(&app, &[1, 2]);
    app.motion
        .retarget(&Stack::default_empty(), &stack, Instant::now());
    assert!(!app.motion.active());
    assert_eq!(app.wanted_size().map(|size| size.height), None);
}

#[test]
fn a_surface_that_goes_away_forgets_its_motion_and_its_pending_frame() {
    let mut app = App::new(Config::default(), moving_look(Curve::Spring));
    app.stack = stack_of(&app, &[1, 2]);
    app.motion
        .retarget(&Stack::default_empty(), &app.stack, Instant::now());
    app.frame_pending = true;
    assert!(app.motion.active());
    app.destroy_live();
    assert!(!app.motion.active());
    assert_eq!(app.motion.hold(), 0);
    assert!(!app.frame_pending);
    // `hide` goes through the same reset.
    app.motion
        .retarget(&Stack::default_empty(), &app.stack, Instant::now());
    app.frame_pending = true;
    app.hide();
    assert!(!app.motion.active() && !app.frame_pending);
}

#[test]
fn a_new_look_snaps_whatever_was_moving() {
    let mut app = App::new(Config::default(), moving_look(Curve::Spring));
    app.stack = stack_of(&app, &[1]);
    app.motion
        .retarget(&Stack::default_empty(), &app.stack, Instant::now());
    assert!(app.motion.active());
    // Re-themed to `motion = none`: nothing in flight, nothing built.
    app.retheme(Look::default());
    assert!(!app.motion.enabled() && !app.motion.active());
    // Re-themed to a look that moves: ready, and at rest.
    app.retheme(moving_look(Curve::EaseOut));
    assert!(app.motion.enabled() && !app.motion.active());
    assert!(app.refresh_due);
}

#[test]
fn a_new_anchor_restarts_the_motion_on_the_new_edge() {
    let mut app = App::new(Config::default(), moving_look(Curve::EaseOut));
    app.stack = stack_of(&app, &[1]);
    app.motion
        .retarget(&Stack::default_empty(), &app.stack, Instant::now());
    app.apply_config(Config {
        anchor: StackAnchor::BottomLeft,
        ..Config::default()
    });
    assert!(!app.motion.active());
    assert!(app.motion.bottom());
}

#[test]
fn the_surface_holds_the_height_a_motion_needs_and_returns_it() {
    let mut app = App::new(Config::default(), moving_look(Curve::EaseOut));
    let before = stack_of(&app, &[1, 2, 3]);
    let after = stack_of(&app, &[1, 3]);
    let t0 = Instant::now();
    app.stack = before.clone();
    app.motion.retarget(&Stack::default_empty(), &before, t0);
    // Everything entered; let it land, then close the middle card.
    app.motion.settle(t0 + std::time::Duration::from_secs(5));
    app.motion.retarget(&before, &after, t0);
    app.stack = after.clone();
    let wanted = app.wanted_size().expect("a card is on screen");
    assert_eq!(wanted.width, after.width);
    assert!(wanted.height >= before.height, "the old height is held");
    // Landed: back to the layout's own.
    app.advance(t0 + std::time::Duration::from_secs(5));
    assert_eq!(app.content_size.map(|s| s.height), Some(after.height));
    assert!(app.dirty);
}

#[test]
fn an_emptied_stack_keeps_its_surface_until_the_last_card_has_left() {
    let mut app = App::new(Config::default(), moving_look(Curve::EaseOut));
    let one = stack_of(&app, &[1]);
    let t0 = Instant::now();
    app.stack = one.clone();
    app.motion.retarget(&Stack::default_empty(), &one, t0);
    app.motion.settle(t0 + std::time::Duration::from_secs(5));
    app.motion.retarget(&one, &Stack::default_empty(), t0);
    let mut gone = Stack::default_empty();
    gone.width = one.width;
    app.stack = gone;
    // Cards are leaving: the surface is wanted, at the held height.
    let wanted = app.wanted_size().expect("the leaving card needs a surface");
    assert!(wanted.height >= one.height);
    // Once it has left, nothing is wanted.
    app.motion.settle(t0 + std::time::Duration::from_secs(5));
    assert_eq!(app.wanted_size(), None);
}

#[test]
fn a_card_in_motion_is_hit_where_it_is_seen() {
    let mut app = App::new(Config::default(), moving_look(Curve::EaseOut));
    let before = stack_of(&app, &[1, 2]);
    let after = stack_of(&app, &[1]);
    let t0 = Instant::now();
    app.stack = before.clone();
    app.motion.retarget(&Stack::default_empty(), &before, t0);
    app.motion.settle(t0 + std::time::Duration::from_secs(5));
    app.motion.retarget(&before, &after, Instant::now());
    app.stack = after.clone();
    app.pointer_in = true;
    // Card 1 is still drawn where it was (below its target); the point
    // the layout has it at is empty gap or the closing ghost, not it.
    let seen_y = before.cards[1].rect.y;
    (app.pointer_x, app.pointer_y) = (180, seen_y + 4);
    assert_eq!(
        app.hit().map(|(id, _)| id),
        Some(1),
        "hit where it is drawn"
    );
    // The ghost of card 2 is nowhere in the stack: a pointer on it hits
    // nothing.
    (app.pointer_x, app.pointer_y) = (180, before.cards[0].rect.y + 4);
    assert_ne!(app.hit().map(|(id, _)| id), Some(2));
    // Landed: the layout's own rectangles again.
    app.motion
        .settle(Instant::now() + std::time::Duration::from_secs(5));
    (app.pointer_x, app.pointer_y) = (180, after.cards[0].rect.y + 4);
    assert_eq!(app.hit().map(|(id, _)| id), Some(1));
}

#[test]
fn compositor_changes_make_the_next_turn_lay_out() {
    let mut app = App::new(Config::default(), moving_look(Curve::EaseOut));
    app.refresh_due = false;
    // A closed surface, a gone output and a reload all say so.
    app.retheme(Look::default());
    assert!(app.refresh_due);
    app.refresh_due = false;
    app.apply_config(Config::default());
    assert!(app.refresh_due);
}

#[test]
fn a_retheme_mid_motion_returns_the_surface_to_the_layouts_height() {
    // `refresh` ends in `sync_content_size`, so a re-theme (which replaces
    // the motion and clears the size) over an unchanged stack still shrinks
    // a surface that was held tall for a card in flight.
    let mut app = App::new(Config::default(), moving_look(Curve::EaseOut));
    let before = stack_of(&app, &[1, 2, 3]);
    let after = stack_of(&app, &[1, 3]);
    let t0 = Instant::now();
    app.motion.retarget(&Stack::default_empty(), &before, t0);
    app.motion.settle(t0 + std::time::Duration::from_secs(5));
    app.motion.retarget(&before, &after, t0);
    app.stack = after.clone();
    app.sync_content_size();
    assert!(app.content_size.is_some_and(|s| s.height >= before.height));
    app.retheme(Look::default());
    assert_eq!(app.content_size, None, "retheme forces a rebuild");
    app.sync_content_size();
    assert_eq!(
        app.content_size.map(|size| size.height),
        Some(after.height),
        "the held height is gone"
    );
    assert!(app.dirty);
}
