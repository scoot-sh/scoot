//! `[appearance] focus_ring_inactive_width`: the ring around a window that is
//! not focused has its own thickness, measured here on real frame pixels.
//!
//! Two windows side by side, so one is always active and one inactive. Each
//! test measures how many ring-colored pixels run outward from the middle of
//! every side of both windows (the straight runs, clear of any corner) and
//! that the next pixel is background, which is what a stale ring left in
//! the gap by a focus move would break. With a corner radius the ring's
//! outer arc is probed on the diagonal too: its radius is the window's plus
//! *that window's* ring width, concentric, so a ring painted at the wrong
//! thickness shows there even when the straight runs happen to agree.
//!
//! Widths 5 and 2 fit the default gap of 12 side by side (`5 + 2 < 12`), so
//! the two rings never meet between the columns. Scale stays 1.0.

use scoot_ipc::{Action, Horizontal, Request, Response};

use super::*;

const ACTIVE: i32 = 5;
const INACTIVE: i32 = 2;
const RADIUS: i32 = 10;

fn fixture(radius: i32, active: i32, inactive: Option<i32>) -> Fixture {
    let mut fixture = Harness::headless(
        Appearance {
            corner_radius: radius,
            focus_ring_width: active,
            focus_ring_inactive_width: inactive,
            ..Appearance::default()
        },
        CANVAS,
    );
    fixture.spawn(run_client);
    fixture.run(Step::Window { color: WINDOW_BGRA });
    fixture.run(Step::Window { color: WINDOW_BGRA });
    fixture
}

fn focus_left(fixture: &mut Fixture) {
    let response = fixture
        .state
        .handle_request(Request::Action(Action::FocusColumn {
            direction: Horizontal::Left,
        }));
    assert!(matches!(response, Response::Ok { .. }), "{response:?}");
    fixture.settle();
}

/// `(rect, is_focused)` for both windows, left to right.
fn windows(fixture: &mut Fixture) -> Vec<(Rect, bool)> {
    fixture.settle();
    let arrangement = fixture.state.world.arrange();
    assert_eq!(arrangement.placements.len(), 2, "two windows, two columns");
    let mut windows: Vec<(Rect, bool)> = arrangement
        .placements
        .iter()
        .map(|p| (p.rect, arrangement.focused == Some(p.id)))
        .collect();
    windows.sort_by_key(|(rect, _)| rect.x);
    assert_eq!(
        windows.iter().filter(|(_, focused)| *focused).count(),
        1,
        "exactly one window is focused"
    );
    windows
}

/// How many pixels starting at `(x, y)` and stepping `(dx, dy)` are `color`.
fn run(pixels: &[u8], x: i32, y: i32, (dx, dy): (i32, i32), color: [u8; 4]) -> i32 {
    (0..40)
        .take_while(|k| pixel(pixels, CANVAS, x + dx * k, y + dy * k) == color)
        .count() as i32
}

/// Asserts the ring on each of `rect`'s four sides is exactly `width` pixels
/// thick and followed by background -- or absent (the first pixel outward is
/// background) when `width` is 0. Returns the ring color (`None` at 0).
fn assert_thickness(
    pixels: &[u8],
    rect: Rect,
    width: i32,
    bg: [u8; 4],
    what: &str,
) -> Option<[u8; 4]> {
    let (mx, my) = (rect.x + rect.w / 2, rect.y + rect.h / 2);
    // (first pixel outside the side, outward step)
    let sides = [
        ("top", (mx, rect.y - 1), (0, -1)),
        ("bottom", (mx, rect.y + rect.h), (0, 1)),
        ("left", (rect.x - 1, my), (-1, 0)),
        ("right", (rect.x + rect.w, my), (1, 0)),
    ];
    let mut ring = None;
    for (side, (x, y), step) in sides {
        let first = pixel(pixels, CANVAS, x, y);
        if width == 0 {
            assert_eq!(first, bg, "{what}: {side}: a zero-width ring drew pixels");
            continue;
        }
        assert_ne!(
            first, bg,
            "{what}: {side}: no ring pixel next to the window"
        );
        ring = Some(*ring.get_or_insert(first));
        assert_eq!(first, ring.unwrap(), "{what}: {side}: ring color differs");
        let thickness = run(pixels, x, y, step, first);
        assert_eq!(thickness, width, "{what}: {side}: ring thickness");
        assert_eq!(
            pixel(
                pixels,
                CANVAS,
                x + step.0 * thickness,
                y + step.1 * thickness
            ),
            bg,
            "{what}: {side}: something past the ring that is not background (stale ring?)"
        );
    }
    ring
}

/// The top-left corner arcs of a window with corner radius `RADIUS` and ring
/// `width`: on the diagonal through the corner circle's center, ring color
/// between the window's arc and the ring's outer arc (`RADIUS + width`),
/// background just outside it, window just inside.
fn assert_arc(pixels: &[u8], rect: Rect, width: i32, ring: [u8; 4], bg: [u8; 4], what: &str) {
    // A one-pixel band is narrower than the diagonal probe's half-pixel error; the
    // straight runs already pin it.
    if width < 2 {
        return;
    }
    let center = (f64::from(rect.x + RADIUS), f64::from(rect.y + RADIUS));
    let at = |d: f64| {
        let offset = d / std::f64::consts::SQRT_2;
        pixel(
            pixels,
            CANVAS,
            (center.0 - offset).floor() as i32,
            (center.1 - offset).floor() as i32,
        )
    };
    let outer = f64::from(RADIUS + width);
    assert_eq!(
        at(f64::from(RADIUS) - 1.5),
        WINDOW_BGRA,
        "{what}: inside the window's own arc"
    );
    assert_eq!(
        at(f64::from(RADIUS) + f64::from(width) / 2.0),
        ring,
        "{what}: the middle of the ring band"
    );
    assert_eq!(
        at(outer + 1.5),
        bg,
        "{what}: outside the ring's outer arc (concentric, radius + width)"
    );
}

/// Measures both windows of the frame against the expected widths by focus
/// state, at the current radius.
fn assert_frame(pixels: &[u8], windows: &[(Rect, bool)], radius: i32, active: i32, inactive: i32) {
    let bg: [u8; 4] = pixels[0..4].try_into().expect("canvas corner");
    let mut colors = Vec::new();
    for &(rect, focused) in windows {
        let width = if focused { active } else { inactive };
        let what = format!("{} window", if focused { "focused" } else { "unfocused" });
        let ring = assert_thickness(pixels, rect, width, bg, &what);
        if let Some(ring) = ring {
            if radius > 0 {
                assert_arc(pixels, rect, width, ring, bg, &what);
            }
            colors.push((focused, ring));
        }
    }
    if let [(_, a), (_, b)] = colors[..] {
        assert_ne!(a, b, "active and inactive rings share a color");
    }
}

#[test]
fn an_inactive_window_draws_its_own_thinner_ring_square_and_rounded() {
    for radius in [0, RADIUS] {
        let mut fixture = fixture(radius, ACTIVE, Some(INACTIVE));
        let windows = windows(&mut fixture);
        let pixels = fixture.render();
        assert_frame(&pixels, &windows, radius, ACTIVE, INACTIVE);
    }
}

/// Moving focus swaps the two widths and leaves nothing behind: the frame
/// drawn after the move is byte-identical to the same final state drawn
/// fresh in one frame, so no pixel of the old, thicker ring survives in the
/// gap around the window whose ring shrank.
#[test]
fn moving_focus_swaps_the_widths_and_leaves_no_stale_ring_pixels() {
    for radius in [0, RADIUS] {
        let mut moved = fixture(radius, ACTIVE, Some(INACTIVE));
        let first = moved.render();
        assert_frame(&first, &windows(&mut moved), radius, ACTIVE, INACTIVE);
        focus_left(&mut moved);
        let windows_after = windows(&mut moved);
        assert!(
            windows_after[0].1,
            "focus moved to the left window: {windows_after:?}"
        );
        let after = moved.render();
        assert_ne!(first, after, "a focus move must change the frame");
        assert_frame(&after, &windows_after, radius, ACTIVE, INACTIVE);

        let mut fresh = fixture(radius, ACTIVE, Some(INACTIVE));
        focus_left(&mut fresh);
        assert_eq!(windows(&mut fresh), windows_after);
        let fresh_frame = fresh.render();
        assert!(
            after == fresh_frame,
            "radius {radius}: the frame after a focus move differs from the same state drawn \
             fresh: a stale or missing ring pixel"
        );
    }
}

#[test]
fn a_zero_inactive_width_draws_no_ring_on_unfocused_windows_before_and_after_a_move() {
    for radius in [0, RADIUS] {
        let mut fixture = fixture(radius, 4, Some(0));
        let windows = windows(&mut fixture);
        assert_frame(&fixture.render(), &windows, radius, 4, 0);
        focus_left(&mut fixture);
        let windows = self::windows(&mut fixture);
        assert_frame(&fixture.render(), &windows, radius, 4, 0);
    }
}

/// Unset means the active width, so a config that never heard of the key
/// renders exactly as before it existed.
#[test]
fn an_unset_inactive_width_follows_the_active_width() {
    for radius in [0, RADIUS] {
        let mut fixture = fixture(radius, 3, None);
        let windows = windows(&mut fixture);
        assert_frame(&fixture.render(), &windows, radius, 3, 3);
        let mut explicit = self::fixture(radius, 3, Some(3));
        assert_eq!(
            fixture.render(),
            explicit.render(),
            "unset and explicitly equal must draw the same frame"
        );
    }
}

/// A live width change (what `scoot msg reload` does) repaints the ring at the
/// new thickness, including the painted rounded ring whose cache key holds
/// the thickness.
#[test]
fn changing_the_inactive_width_live_repaints_the_ring() {
    for radius in [0, RADIUS] {
        let mut fixture = fixture(radius, ACTIVE, Some(INACTIVE));
        let windows = windows(&mut fixture);
        assert_frame(&fixture.render(), &windows, radius, ACTIVE, INACTIVE);
        fixture.state.appearance.focus_ring_inactive_width = Some(4);
        assert_frame(&fixture.render(), &windows, radius, ACTIVE, 4);
        fixture.state.appearance.focus_ring_inactive_width = Some(1);
        assert_frame(&fixture.render(), &windows, radius, ACTIVE, 1);
        fixture.state.appearance.focus_ring_inactive_width = None;
        assert_frame(&fixture.render(), &windows, radius, ACTIVE, ACTIVE);
    }
}
