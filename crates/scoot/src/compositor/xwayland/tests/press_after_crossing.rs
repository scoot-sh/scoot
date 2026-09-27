//! A click on an X window the pointer has only just crossed onto from
//! another X window -- `scoot msg pointer click` onto the other of two X
//! apps, or a `pointer move` and then a `pointer button`.
//!
//! What went wrong (`docs/backlog/resolved/xwayland-press-after-crossing-done.md`):
//! scoot credits a crossing move's relative motion to the surface the
//! pointer is leaving (see `relative_pointer.rs`), so XWayland got
//! `relative_motion`, `leave`, `frame`, `enter`, `frame`. XWayland 24.1's
//! `pointer_handle_frame` returns early while no X window has the pointer,
//! without clearing what the frame carried, so the relative delta sat
//! pending through the `leave`, and the `frame` after the `enter` applied it
//! on top of the position the `enter` had just set -- a second copy of the
//! move, which put the X pointer off the window entered (clamped at the
//! screen edge here) and sent the press to whatever was there instead.
//!
//! Not a batching race: the press was lost however long the pointer sat
//! still first, because nothing after the `enter` corrects the position
//! until the next absolute motion. A hand on a mouse sends one at once; an
//! agent's click does not.

use scoot_core::Rect;
use scoot_ipc::PointerButton;
use x11rb::protocol::Event as XEvent;
use x11rb::protocol::xproto::{
    ChangeWindowAttributesAux, ConnectionExt as _, EventMask, Window as XWindow,
};

use super::drop::{centre, visible};
use super::live::{BLUE, Live, RED, live};
use super::x11::{Props, XClient};

/// Two X windows, each its own client's (as two X apps are), each selecting
/// presses and releases.
struct Pair {
    live: Live,
    other: XClient,
    a: XWindow,
    b: XWindow,
    a_rect: Rect,
    b_rect: Rect,
}

fn select_buttons(x: &XClient, window: XWindow) {
    x.conn
        .change_window_attributes(
            window,
            &ChangeWindowAttributesAux::new()
                .event_mask(EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE),
        )
        .expect("an attributes request")
        .check()
        .expect("the X server accepted the event mask");
}

fn pair(test: &str) -> Option<Pair> {
    let mut live = live(test)?;
    let a = live.x.map(&Props::new(RED));
    let a_id = live.managed(a);
    select_buttons(&live.x, a);
    let other = XClient::connect(live.display);
    let b = other.map(&Props::new(BLUE));
    let b_id = live.managed(b);
    select_buttons(&other, b);
    live.drain();
    let (a_place, b_place) = (live.placement(a_id), live.placement(b_id));
    visible("first X window", &a_place);
    visible("second X window", &b_place);
    Some(Pair {
        live,
        other,
        a,
        b,
        a_rect: a_place.rect,
        b_rect: b_place.rect,
    })
}

/// Presses and releases `window`'s client saw on `window`.
fn buttons(x: &XClient, window: XWindow) -> (usize, usize) {
    x.drain()
        .iter()
        .fold((0, 0), |(press, release), event| match event {
            XEvent::ButtonPress(e) if e.event == window => (press + 1, release),
            XEvent::ButtonRelease(e) if e.event == window => (press, release + 1),
            _ => (press, release),
        })
}

/// How the click follows the move onto the second window.
#[derive(Clone, Copy, Debug)]
enum Shape {
    /// Move, press and release in one input batch and one flush -- what
    /// `scoot msg pointer click` is, and a move and a button pipelined on
    /// one IPC connection.
    Batched,
    /// The move lands and settles, then the click.
    Settled,
    /// The pointer leaves the first window for the bare background between
    /// the columns and settles there, then clicks onto the second.
    ViaBackground,
}

/// What one click saw.
#[derive(Debug, Default, Clone, Copy)]
struct Seen {
    /// Presses and releases on the window clicked.
    target: (usize, usize),
    /// Presses and releases on the window left.
    left: (usize, usize),
    /// Where the X server had its pointer while the button was down.
    at: (i16, i16),
}

impl Pair {
    /// With the pointer resting on `from`, clicks the centre of `to`
    /// (nudged by `nudge`, so no two moves are alike), the way `shape` says.
    fn click_across(&mut self, onto_b: bool, shape: Shape, nudge: i32) -> Seen {
        let (from, to) = if onto_b {
            (self.a_rect, self.b_rect)
        } else {
            (self.b_rect, self.a_rect)
        };
        let (fx, fy) = centre(from);
        self.live.fixture.state.pointer_move(fx, fy);
        self.live.drain();
        if let Shape::ViaBackground = shape {
            // The gap between the two columns: no surface under it.
            let (left, right) = if from.x < to.x {
                (from, to)
            } else {
                (to, from)
            };
            let gap = f64::from(left.x + left.w + right.x) / 2.0;
            self.live.fixture.state.pointer_move(gap, fy);
            self.live.drain();
            assert!(
                self.live
                    .fixture
                    .state
                    .seat
                    .get_pointer()
                    .is_some_and(|pointer| pointer.current_focus().is_none()),
                "the gap between the columns is not bare background"
            );
        }
        self.live.x.drain();
        self.other.drain();
        let (tx, ty) = centre(to);
        self.live
            .fixture
            .state
            .pointer_move(tx + f64::from(nudge), ty);
        if let Shape::Settled | Shape::ViaBackground = shape {
            self.live.drain();
        }
        let state = &mut self.live.fixture.state;
        state.pointer_button(PointerButton::Left, true);
        // Where scoot's loop flushes (`post_dispatch`): after the batch.
        let _ = state.display_handle.flush_clients();
        self.live.drain();
        let pointer = self
            .live
            .x
            .conn
            .query_pointer(self.live.x.root)
            .expect("a pointer query")
            .reply()
            .expect("the pointer");
        self.live
            .fixture
            .state
            .pointer_button(PointerButton::Left, false);
        self.live.drain();
        let (on_a, on_b) = (buttons(&self.live.x, self.a), buttons(&self.other, self.b));
        let (target, left) = if onto_b { (on_b, on_a) } else { (on_a, on_b) };
        Seen {
            target,
            left,
            at: (pointer.root_x, pointer.root_y),
        }
    }

    /// Clicks back and forth `rounds` times each way, asserting each click
    /// lands on the window clicked and only there.
    fn assert_clicks_land(&mut self, shape: Shape, rounds: i32) {
        for round in 0..rounds {
            for onto_b in [true, false] {
                let to = if onto_b { self.b_rect } else { self.a_rect };
                let seen = self.click_across(onto_b, shape, round % 5);
                let (tx, ty) = centre(to);
                #[allow(clippy::cast_possible_truncation)]
                let expected = ((tx as i32 + round % 5) as i16, ty as i16);
                assert_eq!(
                    (seen.target, seen.left, seen.at),
                    ((1, 1), (0, 0), expected),
                    "{shape:?} click {round} onto the {} X window: \
                     (target presses/releases, left presses/releases, X pointer)",
                    if onto_b { "second" } else { "first" },
                );
            }
        }
    }
}

/// `scoot msg pointer click` onto the other of two X apps: the press and
/// release reach the window clicked, with the X pointer where the click
/// was. Failed before the fix with no press on either window and the X
/// pointer at the screen edge.
#[test]
fn a_click_crossing_from_one_x_window_to_another_lands_on_the_one_entered() {
    let Some(mut pair) =
        pair("a_click_crossing_from_one_x_window_to_another_lands_on_the_one_entered")
    else {
        return;
    };
    pair.assert_clicks_land(Shape::Batched, 3);
}

/// The same, with the move settled before the click: the lost press was
/// never a batching race.
#[test]
fn a_click_after_a_settled_crossing_between_x_windows_lands() {
    let Some(mut pair) = pair("a_click_after_a_settled_crossing_between_x_windows_lands") else {
        return;
    };
    pair.assert_clicks_land(Shape::Settled, 3);
}

/// Leaving an X window for the bare background is a `leave` too: the delta
/// of that move must not be applied when the pointer next enters an X
/// window.
#[test]
fn a_click_after_a_detour_over_the_background_lands() {
    let Some(mut pair) = pair("a_click_after_a_detour_over_the_background_lands") else {
        return;
    };
    pair.assert_clicks_land(Shape::ViaBackground, 3);
}

/// A measurement, run by hand: counts, per shape and direction, the clicks
/// that reached the window clicked and the ones that reached the window
/// left. Its numbers are in the resolution entry (see the module doc).
///
/// ```sh
/// SCOOT_REQUIRE_XWAYLAND=1 PRESS_N=40 cargo test -p scoot --features xwayland --bin scoot \
///     press_after_crossing_measurement -- --ignored --nocapture
/// ```
#[test]
#[ignore = "a measurement, run by hand: see its doc"]
fn press_after_crossing_measurement() {
    let Some(mut pair) = pair("press_after_crossing_measurement") else {
        return;
    };
    let n: i32 = std::env::var("PRESS_N")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(40);
    for shape in [Shape::Batched, Shape::Settled, Shape::ViaBackground] {
        for onto_b in [true, false] {
            let (mut target, mut left, mut off) = (0, 0, 0);
            for i in 0..n {
                let seen = pair.click_across(onto_b, shape, i % 5);
                target += seen.target.0;
                left += seen.left.0;
                let (tx, ty) = centre(if onto_b { pair.b_rect } else { pair.a_rect });
                #[allow(clippy::cast_possible_truncation)]
                let expected = ((tx as i32 + i % 5) as i16, ty as i16);
                off += usize::from(seen.at != expected);
            }
            eprintln!(
                "RESULT shape={shape:?} onto={} n={n} press_on_target={target} \
                 press_on_left={left} x_pointer_off_target={off}",
                if onto_b { "second" } else { "first" },
            );
        }
    }
}
