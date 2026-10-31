//! The motion state's tests: what enters, what leaves, what settles, and
//! that every step lands. Time is explicit (`Instant` plus an offset), so
//! nothing here sleeps.

use std::time::{Duration, Instant};

use scootui::motion::{Curve, DEFAULT_BOUNCE};
use scootui::paint::{Canvas, Pad};

use super::*;
use crate::config::Config;
use crate::look::MotionTokens;
use crate::model::store::Notification;
use crate::view::layout;
use crate::view::render::tests::{make_text, note};

const MS: u64 = 1;

fn at(t0: Instant, ms: u64) -> Instant {
    t0 + Duration::from_millis(ms * MS)
}

fn tokens(curve: Curve, enter: CardMotion) -> MotionTokens {
    MotionTokens {
        curve,
        duration_ms: 200,
        bounce: DEFAULT_BOUNCE,
        enter,
        exit: CardMotion::Fade,
    }
}

/// The laid-out stack for notifications `ids` (oldest first), padded for a
/// shadow as the popup does.
fn stack_of(ids: &[u32], pad: Pad) -> Stack {
    let notes: Vec<Notification> = ids
        .iter()
        .map(|&id| {
            note(
                id,
                &format!("Card {id}"),
                "A body that wraps around a few times",
            )
        })
        .collect();
    let live: Vec<&Notification> = notes.iter().collect();
    layout::inset(
        layout::layout(&live, &Config::default(), &make_text(), 13.0, 2_000),
        pad,
    )
}

fn empty() -> Stack {
    stack_of(&[], Pad::ZERO)
}

fn pose_dy(motion: &Motion, id: u32, now: Instant) -> Option<i32> {
    motion.pose(id, now).map(|pose| pose.dy)
}

#[test]
fn a_look_without_motion_builds_nothing() {
    let mut motion = Motion::new(&MotionTokens::default(), Anchor::TopRight);
    assert!(!motion.enabled());
    let t0 = Instant::now();
    motion.retarget(&empty(), &stack_of(&[1, 2], Pad::ZERO), t0);
    assert!(!motion.active());
    assert_eq!(motion.hold(), 0);
    assert!(motion.pool.is_empty() && motion.tracks.is_empty() && motion.ghosts.is_empty());
    assert_eq!(motion.pose(1, t0), None);
    assert!(!motion.settle(t0));
}

#[test]
fn zero_duration_is_no_motion() {
    let mut tokens = tokens(Curve::Spring, CardMotion::Slide);
    tokens.duration_ms = 0;
    let motion = Motion::new(&tokens, Anchor::TopRight);
    assert!(!motion.enabled());
}

#[test]
fn a_new_card_enters_and_lands_exactly() {
    for (style, check) in [
        (CardMotion::Fade, (0, 0, 1.0)),
        (CardMotion::Slide, (40, 0, 1.0)),
        (CardMotion::Drop, (0, -24, 1.0)),
        (CardMotion::Scale, (0, 0, SCALE_FROM)),
    ] {
        let mut motion = Motion::new(&tokens(Curve::EaseOut, style), Anchor::TopRight);
        let t0 = Instant::now();
        motion.retarget(&empty(), &stack_of(&[1], Pad::ZERO), t0);
        assert!(motion.active(), "{style:?}");
        let start = motion.pose(1, t0).expect("entering");
        assert_eq!(start.alpha, 0, "{style:?}");
        assert_eq!((start.dx, start.dy), (check.0, check.1), "{style:?}");
        assert!((start.scale - check.2).abs() < 1e-6, "{style:?}");
        let mid = motion.pose(1, at(t0, 100)).expect("still entering");
        assert!(mid.alpha > 0 && mid.alpha < 255, "{style:?} {mid:?}");
        // Late frame: past the deadline it is at rest and gone.
        assert!(motion.settle(at(t0, 5_000)));
        assert!(!motion.active() && motion.pose(1, at(t0, 5_000)).is_none());
        assert!(motion.pool.is_empty(), "an idle stack holds no layers");
        assert_eq!(motion.hold(), 0);
    }
}

#[test]
fn slide_comes_from_the_anchored_edge() {
    for (anchor, sign) in [
        (Anchor::TopRight, 1),
        (Anchor::BottomRight, 1),
        (Anchor::TopLeft, -1),
        (Anchor::BottomLeft, -1),
    ] {
        let mut motion = Motion::new(&tokens(Curve::EaseOut, CardMotion::Slide), anchor);
        let t0 = Instant::now();
        motion.retarget(&empty(), &stack_of(&[1], Pad::ZERO), t0);
        assert_eq!(motion.pose(1, t0).unwrap().dx, sign * 40, "{anchor:?}");
    }
}

#[test]
fn closing_a_middle_card_restacks_the_ones_behind_it() {
    let t0 = Instant::now();
    let before = stack_of(&[1, 2, 3], Pad::ZERO);
    let after = stack_of(&[1, 3], Pad::ZERO);
    // Newest first: 3, 2, 1. Closing 2 moves 1 up by 2's height and gap.
    let moved = before.cards[2].rect.y - after.cards[1].rect.y;
    assert!(moved > 0);
    let mut motion = Motion::new(&tokens(Curve::EaseOut, CardMotion::Fade), Anchor::TopRight);
    motion.retarget(&before, &after, t0);
    // Card 3 did not move; card 1 starts where it was seen.
    assert_eq!(motion.pose(3, t0), None);
    assert_eq!(pose_dy(&motion, 1, t0), Some(moved as i32));
    let mid = pose_dy(&motion, 1, at(t0, 100)).unwrap();
    assert!(mid > 0 && mid < moved as i32, "{mid}");
    // The closed card is a ghost, and is nowhere in the live poses.
    assert_eq!(motion.ghosts.len(), 1);
    assert_eq!(motion.pose(2, t0), None);
    // The surface keeps the old height while it travels.
    assert!(motion.hold() >= before.height);
    assert!(motion.settle(at(t0, 1_000)));
    assert!(!motion.active());
    assert_eq!(motion.hold(), 0);
}

#[test]
fn a_bottom_anchor_keeps_the_card_beside_the_edge_still() {
    let t0 = Instant::now();
    let before = stack_of(&[1, 2, 3], Pad::ZERO);
    let after = stack_of(&[1, 3], Pad::ZERO);
    let mut motion = Motion::new(
        &tokens(Curve::EaseOut, CardMotion::Fade),
        Anchor::BottomRight,
    );
    motion.retarget(&before, &after, t0);
    // The oldest card sits at the bottom edge and does not move; the
    // newer one (above the closed card) drops into the gap.
    assert_eq!(motion.pose(1, t0), None);
    let gap = pose_dy(&motion, 3, t0).expect("moves");
    assert!(gap < 0, "{gap}");
    assert!(motion.hold() >= before.height);
}

#[test]
fn an_arrival_pushes_a_top_anchored_stack_down_and_leaves_a_bottom_one_alone() {
    let t0 = Instant::now();
    let before = stack_of(&[1, 2], Pad::ZERO);
    let after = stack_of(&[1, 2, 3], Pad::ZERO);
    let mut top = Motion::new(&tokens(Curve::EaseOut, CardMotion::Fade), Anchor::TopLeft);
    top.retarget(&before, &after, t0);
    assert!(pose_dy(&top, 2, t0).unwrap() < 0);
    assert!(pose_dy(&top, 1, t0).unwrap() < 0);
    let mut bottom = Motion::new(
        &tokens(Curve::EaseOut, CardMotion::Fade),
        Anchor::BottomLeft,
    );
    bottom.retarget(&before, &after, t0);
    assert_eq!(bottom.pose(2, t0), None);
    assert_eq!(bottom.pose(1, t0), None);
    // Only the arrival enters.
    assert!(bottom.pose(3, t0).is_some());
}

#[test]
fn a_new_target_mid_flight_continues_from_what_is_on_screen() {
    let t0 = Instant::now();
    let mut motion = Motion::new(&tokens(Curve::EaseOut, CardMotion::Fade), Anchor::TopRight);
    let a = stack_of(&[1, 2, 3, 4], Pad::ZERO);
    let b = stack_of(&[1, 2, 4], Pad::ZERO);
    let c = stack_of(&[1, 4], Pad::ZERO);
    motion.retarget(&a, &b, t0);
    let seen = pose_dy(&motion, 1, at(t0, 80)).unwrap();
    // A second close lands 80 ms in: card 1 is drawn where it was, then
    // heads for the new target.
    motion.retarget(&b, &c, at(t0, 80));
    let just_after = pose_dy(&motion, 1, at(t0, 80)).unwrap();
    let new_gap = b.cards[2].rect.y - c.cards[1].rect.y;
    let _ = new_gap;
    // The visual position is (target + offset): the target moved by the
    // second close, the offset absorbed exactly the difference.
    let moved_target =
        i32::try_from(b.cards[2].rect.y).unwrap() - i32::try_from(c.cards[1].rect.y).unwrap();
    assert_eq!(just_after, seen + moved_target);
    assert!(motion.settle(at(t0, 10_000)) && !motion.active());
}

#[test]
fn a_card_closed_mid_enter_leaves_from_the_alpha_it_had() {
    let t0 = Instant::now();
    let mut motion = Motion::new(&tokens(Curve::EaseOut, CardMotion::Fade), Anchor::TopRight);
    let one = stack_of(&[1], Pad::ZERO);
    motion.retarget(&empty(), &one, t0);
    let alpha_then = motion.pose(1, at(t0, 60)).unwrap().alpha;
    assert!(alpha_then > 0 && alpha_then < 255);
    motion.retarget(&one, &empty(), at(t0, 60));
    assert_eq!(motion.tracks.len(), 0, "a leaving card is not a live one");
    assert_eq!(motion.ghosts.len(), 1);
    let away = motion.ghosts[0].away0;
    let drawn = ((1.0 - away) * 255.0).round() as u8;
    assert_eq!(drawn, alpha_then);
    assert!(motion.active());
    assert!(motion.settle(at(t0, 10_000)));
    assert!(!motion.active());
}

#[test]
fn a_burst_of_arrivals_keeps_every_card_and_stays_bounded() {
    let t0 = Instant::now();
    let mut motion = Motion::new(&tokens(Curve::Spring, CardMotion::Slide), Anchor::TopRight);
    let mut old = empty();
    let mut ids = Vec::new();
    for n in 1..=40u32 {
        ids.push(n);
        // The stack shows the newest five, like `max_visible`.
        let shown: Vec<u32> = ids.iter().rev().take(5).rev().copied().collect();
        let next = stack_of(&shown, Pad::ZERO);
        motion.retarget(&old, &next, at(t0, u64::from(n) * 5));
        assert!(motion.tracks.len() <= 5, "{}", motion.tracks.len());
        assert!(motion.ghosts.len() <= MAX_GHOSTS);
        assert!(motion.pool.len() <= MAX_GHOSTS + 8);
        old = next;
    }
    assert!(motion.settle(at(t0, 60_000)));
    assert!(!motion.active());
}

#[test]
fn closing_everything_at_once_caps_the_ghosts() {
    let t0 = Instant::now();
    let mut motion = Motion::new(&tokens(Curve::EaseOut, CardMotion::Fade), Anchor::TopRight);
    let config = Config {
        max_visible: 20,
        ..Config::default()
    };
    let notes: Vec<Notification> = (1..=20)
        .map(|id| note(id, &format!("c{id}"), "body"))
        .collect();
    let live: Vec<&Notification> = notes.iter().collect();
    let many = layout::layout(&live, &config, &make_text(), 13.0, 2_000);
    assert_eq!(many.cards.len(), 20);
    motion.retarget(&many, &empty(), t0);
    assert_eq!(motion.ghosts.len(), MAX_GHOSTS);
    assert!(motion.settle(at(t0, 10_000)) && !motion.active());
}

#[test]
fn a_spring_overshoots_and_an_ease_out_does_not() {
    let t0 = Instant::now();
    let before = stack_of(&[1, 2, 3], Pad::ZERO);
    let after = stack_of(&[1, 3], Pad::ZERO);
    for (curve, overshoots) in [(Curve::Spring, true), (Curve::EaseOut, false)] {
        let mut motion = Motion::new(&tokens(curve, CardMotion::Fade), Anchor::TopRight);
        motion.retarget(&before, &after, t0);
        let lowest = (0..=200)
            .filter_map(|ms| pose_dy(&motion, 1, at(t0, ms)))
            .min()
            .unwrap();
        assert_eq!(lowest < 0, overshoots, "{curve:?}: {lowest}");
        // The hold covers the overshoot: the card never leaves the surface.
        let height = i64::from(after.cards[1].rect.y)
            + i64::from(lowest)
            + i64::from(after.cards[1].rect.height);
        assert!(height <= i64::from(motion.hold()));
    }
}

#[test]
fn hit_testing_follows_the_card_where_it_is_seen() {
    let t0 = Instant::now();
    let mut motion = Motion::new(&tokens(Curve::EaseOut, CardMotion::Slide), Anchor::TopRight);
    let one = stack_of(&[1], Pad::ZERO);
    motion.retarget(&empty(), &one, t0);
    let card = &one.cards[0];
    let rect = card.rect;
    // At the start the card is drawn 40 px to the right: the point that
    // used to be its left edge is now outside it, and 40 px in is inside.
    let (x, _) = motion.unproject(card, i64::from(rect.x) + 45, i64::from(rect.y) + 5, 0, t0);
    assert_eq!(x, i64::from(rect.x) + 5);
    // Settled: the identity.
    assert!(motion.settle(at(t0, 5_000)));
    let (x, y) = motion.unproject(card, 11, 12, 0, at(t0, 5_000));
    assert_eq!((x, y), (11, 12));
}

#[test]
fn a_scaled_card_is_hit_through_its_scale() {
    let t0 = Instant::now();
    let mut motion = Motion::new(&tokens(Curve::EaseOut, CardMotion::Scale), Anchor::TopRight);
    let one = stack_of(&[1], Pad::ZERO);
    motion.retarget(&empty(), &one, t0);
    let card = &one.cards[0];
    let rect = card.rect;
    // At the start the card is 88% of its size about its center: its own
    // center maps to its center, and a point 44% of the width right of
    // center is the edge.
    let (cx, cy) = (
        i64::from(rect.x) + i64::from(rect.width) / 2,
        i64::from(rect.y) + i64::from(rect.height) / 2,
    );
    assert_eq!(motion.unproject(card, cx, cy, 0, t0), (cx, cy));
    let edge = (f64::from(rect.width) * 0.44).round() as i64;
    let (x, _) = motion.unproject(card, cx + edge, cy, 0, t0);
    assert!((x - (cx + i64::from(rect.width) / 2)).abs() <= 1, "{x}");
}

/// The rectangles `push_input_rects` gives for one card.
fn shape_of(
    motion: &Motion,
    card: &Card,
    radius: u32,
    shift: u32,
    surface: (u32, u32),
    now: Instant,
) -> Vec<scootui::region::Rect> {
    let mut out = Vec::new();
    motion.push_input_rects(card, radius, shift, surface, now, &mut out);
    out
}

/// Whether the point is inside any of the rectangles.
fn covers(rects: &[scootui::region::Rect], x: u32, y: u32) -> bool {
    rects
        .iter()
        .any(|r| x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height)
}

#[test]
fn the_input_region_is_where_the_cards_are_seen_and_nothing_for_a_ghost() {
    let t0 = Instant::now();
    let before = stack_of(&[1, 2], Pad::ZERO);
    let after = stack_of(&[1], Pad::ZERO);
    let mut motion = Motion::new(&tokens(Curve::EaseOut, CardMotion::Slide), Anchor::TopRight);
    motion.retarget(&before, &after, t0);
    let surface = (after.width, after.height.max(motion.hold()));
    let card = &after.cards[0];
    // Card 1 is still drawn where it was (below its target).
    let seen = shape_of(&motion, card, 0, 0, surface, t0);
    assert_eq!(seen.len(), 1, "a square card is one rectangle: {seen:?}");
    assert_eq!(seen[0].y, before.cards[1].rect.y);
    // The ghost has no card in the stack, so no rectangle is asked for it.
    assert!(motion.ghosts.len() == 1 && motion.pose(2, t0).is_none());
    // Fully off the surface: nothing.
    assert!(shape_of(&motion, card, 0, 10_000, surface, t0).is_empty());
    // At rest the rectangle is the layout's own.
    assert!(motion.settle(at(t0, 5_000)));
    let rest = shape_of(&motion, card, 0, 0, surface, at(t0, 5_000));
    let own = card.rect;
    assert_eq!(
        rest,
        [scootui::region::Rect {
            x: own.x,
            y: own.y,
            width: own.width,
            height: own.height
        }]
    );
}

#[test]
fn a_rounded_card_in_motion_leaves_its_cut_corner_to_the_desktop() {
    let t0 = Instant::now();
    let mut motion = Motion::new(&tokens(Curve::EaseOut, CardMotion::Scale), Anchor::TopRight);
    let one = stack_of(&[1], Pad::ZERO);
    motion.retarget(&empty(), &one, t0);
    let surface = (one.width, one.height.max(motion.hold()));
    let card = &one.cards[0];
    let pose = motion.pose(1, t0).unwrap();
    assert!(pose.scale < 1.0, "the card is mid-scale: {}", pose.scale);
    let radius = 20;
    let shape = shape_of(&motion, card, radius, 0, surface, t0);
    let square = shape_of(&motion, card, 0, 0, surface, t0);
    assert_eq!(square.len(), 1);
    let sq = square[0];
    // The pill-sized corner takes no click at the seen card's corner, and
    // does at its middle; the unrounded one takes both.
    assert!(!covers(&shape, sq.x, sq.y), "{shape:?}");
    assert!(covers(&square, sq.x, sq.y));
    assert!(covers(&shape, sq.x + sq.width / 2, sq.y + sq.height / 2));
    assert!(!covers(&shape, sq.x + sq.width - 1, sq.y + sq.height - 1));
    // Nothing leaves the seen card's box, and nothing is empty.
    for r in &shape {
        assert!(r.width > 0 && r.height > 0, "{r:?}");
        assert!(r.x >= sq.x && r.x + r.width <= sq.x + sq.width, "{r:?}");
        assert!(r.y >= sq.y && r.y + r.height <= sq.y + sq.height, "{r:?}");
    }
}

#[test]
fn a_shape_clipped_by_the_surface_drops_the_rows_outside_it() {
    let t0 = Instant::now();
    let mut motion = Motion::new(&tokens(Curve::EaseOut, CardMotion::Slide), Anchor::TopRight);
    let one = stack_of(&[1], Pad::ZERO);
    motion.retarget(&empty(), &one, t0);
    let card = &one.cards[0];
    // A surface cut through the card's corner rows keeps only what is on
    // it; appending keeps what the list already held.
    let surface = (one.width, 6);
    let mut out = vec![scootui::region::Rect {
        x: 1,
        y: 2,
        width: 3,
        height: 4,
    }];
    motion.push_input_rects(card, 30, 0, surface, t0, &mut out);
    assert_eq!(out[0].x, 1);
    assert!(out.len() > 1);
    for r in &out[1..] {
        assert!(r.y + r.height <= 6 && r.x + r.width <= one.width, "{r:?}");
    }
}

#[test]
fn a_card_too_large_to_move_just_snaps() {
    let t0 = Instant::now();
    let mut big = stack_of(&[1], Pad::ZERO);
    big.cards[0].rect.width = 5_000;
    big.cards[0].rect.height = 5_000;
    let mut motion = Motion::new(&tokens(Curve::EaseOut, CardMotion::Fade), Anchor::TopRight);
    motion.retarget(&empty(), &big, t0);
    assert!(!motion.active());
    motion.retarget(&big, &empty(), t0);
    assert!(!motion.active());
}

#[test]
fn reset_drops_everything() {
    let t0 = Instant::now();
    let mut motion = Motion::new(&tokens(Curve::EaseOut, CardMotion::Fade), Anchor::TopRight);
    motion.retarget(
        &stack_of(&[1, 2], Pad::ZERO),
        &stack_of(&[2], Pad::ZERO),
        t0,
    );
    assert!(motion.active());
    motion.reset();
    assert!(!motion.active());
    assert!(motion.pool.is_empty());
    assert_eq!(motion.hold(), 0);
}

#[test]
fn a_shadow_pad_travels_with_the_layer() {
    let pad = Pad {
        top: 8,
        right: 8,
        bottom: 16,
        left: 8,
    };
    let t0 = Instant::now();
    let stack = stack_of(&[1], pad);
    let mut motion = Motion::new(&tokens(Curve::EaseOut, CardMotion::Fade), Anchor::TopRight);
    motion.retarget(&empty(), &stack, t0);
    let picture = &motion.tracks[0].picture;
    assert_eq!(
        (picture.layer.width(), picture.layer.height()),
        (
            stack.cards[0].rect.width + 16,
            stack.cards[0].rect.height + 24
        )
    );
    // The card sits `pad` in from the layer's corner.
    assert_eq!((picture.local.rect.x, picture.local.rect.y), (8, 8));
}

#[test]
fn stepping_and_composing_allocate_nothing() {
    let t0 = Instant::now();
    let before = stack_of(&[1, 2, 3], Pad::ZERO);
    let after = stack_of(&[1, 3, 4], Pad::ZERO);
    let mut motion = Motion::new(
        &tokens(Curve::Spring, CardMotion::Scale),
        Anchor::BottomRight,
    );
    motion.retarget(&before, &after, t0);
    let (w, h) = (after.width, after.height.max(motion.hold()));
    let mut pixels = vec![0u8; w as usize * h as usize * 4];
    let mut canvas = Canvas::new(&mut pixels, w, h).unwrap();
    // Warm: the first compose touches nothing it has not already, and the
    // list has the room the rows take.
    motion.compose(&mut canvas, at(t0, 20));
    let mut out = Vec::with_capacity(128);
    let ((), allocations) = scootbg_mem::count_allocations(|| {
        for ms in (0..190).step_by(16) {
            let now = at(t0, ms);
            let _ = motion.pose(3, now);
            out.clear();
            motion.push_input_rects(&after.cards[0], 12, 0, (w, h), now, &mut out);
            let _ = motion.unproject(&after.cards[0], 5, 5, 0, now);
            motion.compose(&mut canvas, now);
        }
    });
    assert_eq!(allocations, 0);
}

#[test]
fn a_card_whose_place_changes_mid_flight_keeps_its_picture_at_its_corner() {
    // A second retarget moves the card's layout position: its picture is
    // refit to the new place, not left translated by the old one.
    let pad = Pad {
        top: 8,
        right: 8,
        bottom: 16,
        left: 8,
    };
    let t0 = Instant::now();
    let a = stack_of(&[1, 2, 3, 4], pad);
    let b = stack_of(&[1, 2, 4], pad);
    let c = stack_of(&[1, 4], pad);
    let mut motion = Motion::new(&tokens(Curve::EaseOut, CardMotion::Fade), Anchor::TopRight);
    motion.retarget(&a, &b, t0);
    motion.retarget(&b, &c, at(t0, 50));
    for track in &motion.tracks {
        let picture = &track.picture;
        // The card sits exactly the shadow's room in from its layer.
        assert_eq!((picture.local.rect.x, picture.local.rect.y), (8, 8));
        assert_eq!(picture.layer.height(), picture.local.rect.height + 24);
    }
    assert!(!motion.tracks.is_empty());
}

#[test]
fn a_replace_that_changes_a_moving_cards_height_resizes_its_layer() {
    let t0 = Instant::now();
    let before = stack_of(&[1, 2, 3], Pad::ZERO);
    let after = stack_of(&[1, 3], Pad::ZERO);
    let mut motion = Motion::new(&tokens(Curve::EaseOut, CardMotion::Fade), Anchor::TopRight);
    motion.retarget(&before, &after, t0);
    // Card 1 is mid-flight; its replacement is taller.
    let mut taller = after.clone();
    let last = taller.cards.len() - 1;
    taller.cards[last].rect.height += 30;
    motion.retarget(&after, &taller, at(t0, 40));
    let track = motion
        .tracks
        .iter()
        .find(|track| track.id == after.cards[last].id)
        .expect("still moving");
    assert_eq!(track.picture.layer.height(), taller.cards[last].rect.height);
    assert!(track.picture.stale);
}

#[test]
fn a_spring_never_carries_a_card_off_the_surface() {
    // Close the newest card (the stack's top moves) and the oldest (its
    // bottom does), under each anchor, with a spring that overshoots 9%.
    let t0 = Instant::now();
    let before = stack_of(&[1, 2, 3, 4, 5], Pad::ZERO);
    for (anchor, after) in [
        (Anchor::TopRight, stack_of(&[1, 2, 3, 4], Pad::ZERO)),
        (Anchor::TopRight, stack_of(&[2, 3, 4, 5], Pad::ZERO)),
        (Anchor::BottomRight, stack_of(&[1, 2, 3, 4], Pad::ZERO)),
        (Anchor::BottomRight, stack_of(&[2, 3, 4, 5], Pad::ZERO)),
    ] {
        let mut motion = Motion::new(&tokens(Curve::Spring, CardMotion::Fade), anchor);
        motion.retarget(&before, &after, t0);
        let surface = after.height.max(motion.hold());
        motion.fit(surface);
        let shift = motion.shift(surface, after.height);
        let mut clamped = false;
        for ms in 0..=200 {
            let now = at(t0, ms);
            for card in &after.cards {
                let Some(pose) = motion.pose(card.id, now) else {
                    continue;
                };
                let top = i64::from(card.rect.y) + i64::from(pose.dy) + i64::from(shift);
                let bottom = top + i64::from(card.rect.height);
                assert!(
                    top >= 0 && bottom <= i64::from(surface),
                    "{anchor:?} card {} at {ms} ms: {top}..{bottom} of {surface}",
                    card.id
                );
                clamped |= top == 0 || bottom == i64::from(surface);
            }
        }
        // Without `fit` the same spring may leave the surface.
        let mut loose = Motion::new(&tokens(Curve::Spring, CardMotion::Fade), anchor);
        loose.retarget(&before, &after, t0);
        let leaves = (0..=200).any(|ms| {
            after.cards.iter().any(|card| {
                loose.pose(card.id, at(t0, ms)).is_some_and(|pose| {
                    let top = i64::from(card.rect.y) + i64::from(pose.dy) + i64::from(shift);
                    top < 0 || top + i64::from(card.rect.height) > i64::from(surface)
                })
            })
        });
        // The limit engages exactly when the spring would have left.
        assert_eq!(clamped, leaves, "{anchor:?}");
    }
}
