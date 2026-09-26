//! One arrangement per frame tick, shared by every output.
//!
//! `State::render` computes the core's arrangement once per tick and hands
//! the same one to every output's element gathering (see `render::draw_frame`)
//! instead of arranging once per output per frame. A stale arrangement shows
//! wrong pixels with no error, so this suite is adversarial about
//! invalidation: every kind of layout input (open, close, move, resize,
//! focus, workspace switch, output add/remove, lock/unlock) must refresh the
//! next frame, while consecutive identical frames share one arrangement and
//! draw byte-identical pixels.
//!
//! What is *not* shown here, on purpose: whether the shared arrangement
//! draws the right thing in the first place. That is the pixel suites'
//! (`session_lock`, `layer_shell`, `output_clip`, `rounded`, `cursor`,
//! `output_scale`) job -- they pin exact pixels through the same gather
//! path, and all must pass unmodified. These tests pin the sharing itself:
//! the call count (via `arrange_calls_for_test`), the freshness across
//! mutations, and that no per-output frame input (geometry, scale) leaks
//! into the shared layout.
//!
//! The windows live in [`scoot_core`] only (opened straight into the core,
//! like `headless::bench`'s scenes): no client surface means the window
//! gather finds nothing mapped, but the focus ring is still drawn for every
//! placed window -- which is exactly the arrangement-dependent pixel this
//! suite watches. A mutation that re-lays-out moves or recolours a ring, so
//! the framebuffer changes; a mutation the frame missed leaves stale ring
//! pixels behind.

use std::time::{Duration, Instant};

use scoot_core::{
    Action, Event as CoreEvent, Horizontal, OutputId, Vertical, WindowId, WindowInfo,
};

use crate::compositor::decorations::Appearance;
use crate::compositor::headless;
use crate::compositor::test_support::{self, Harness};

/// Small square canvas: every assertion here compares whole framebuffers
/// byte for byte, and ring pixels read the same at any size.
const CANVAS: i32 = 200;

/// No client ever connects here (except the parked locker in
/// [`relock_refreshes_and_restores`]), so the step/ack vocabulary is empty.
type Fixture = Harness<(), ()>;

/// Opens `windows` windows straight into the core on the focused output --
/// the arrangement-dependent scene (rings, no client content) this suite
/// watches. Windows land on output 1, the focused one.
fn open_windows(fixture: &mut Fixture, first: u64, windows: u64) {
    for index in 0..windows {
        fixture.state.world.handle_event(CoreEvent::WindowOpened {
            id: WindowId(first + index),
            info: WindowInfo {
                app_id: "arrange-once".to_string(),
                title: "arrange-once".to_string(),
                hints: Default::default(),
                parent: None,
            },
            output: None,
            focus: true,
        });
    }
}

/// A live compositor with two side-by-side outputs and `windows` core
/// windows on the first: one populated strip plus one empty one, so a leak
/// in either direction reads unambiguously.
fn two_output_fixture(windows: u64) -> Fixture {
    let mut fixture = Fixture::headless(Appearance::default(), CANVAS);
    headless::add_output(&mut fixture.state, "headless-2", CANVAS, CANVAS)
        .expect("a second headless output");
    open_windows(&mut fixture, 1, windows);
    fixture
}

/// Draws every output and hands back both outputs' raw BGRA pixels.
fn render_all(fixture: &mut Fixture) -> (Vec<u8>, Vec<u8>) {
    fixture.state.request_render();
    fixture.state.render();
    (
        fixture.pixels_of(OutputId(1)),
        fixture.pixels_of(OutputId(2)),
    )
}

/// How many frame-loop arrangements happened since the count was reset.
fn arrange_calls(fixture: &Fixture) -> usize {
    fixture.state.arrange_calls_for_test.get()
}

fn reset_arrange_calls(fixture: &Fixture) {
    fixture.state.arrange_calls_for_test.set(0);
}

/// Two outputs, one arrangement: a full tick moves the count by exactly one,
/// the populated output shows its rings, and the empty output stays pristine
/// background -- no window state leaking across the shared layout in either
/// direction.
#[test]
fn one_arrange_per_frame_serves_every_output() {
    let mut fixture = two_output_fixture(3);
    reset_arrange_calls(&fixture);
    let (first, second) = render_all(&mut fixture);
    assert_eq!(
        arrange_calls(&fixture),
        1,
        "a two-output tick must arrange exactly once"
    );
    let background = second[0..4].to_vec();
    assert!(
        second.chunks_exact(4).all(|pixel| pixel == background),
        "the empty output must be pristine background, not another output's rings"
    );
    assert_ne!(
        first, second,
        "the populated output must draw its rings from the shared arrangement"
    );
}

/// Consecutive identical frames share and repeat: a second tick arranges
/// once more (one per tick, never cached across ticks) and draws
/// byte-identical pixels on both outputs -- the idle reuse half of the
/// ticket.
#[test]
fn identical_consecutive_frames_share_and_repeat() {
    let mut fixture = two_output_fixture(3);
    let (first, second) = render_all(&mut fixture);
    reset_arrange_calls(&fixture);
    let (again_first, again_second) = render_all(&mut fixture);
    assert_eq!(
        arrange_calls(&fixture),
        1,
        "an identical tick must still arrange exactly once (fresh, not cached)"
    );
    assert_eq!(again_first, first, "output 1 must repeat its pixels");
    assert_eq!(again_second, second, "output 2 must repeat its pixels");
}

/// An animated scene repeats exactly: cycling focus across the strip
/// recolours the rings each frame, and running the same cycle again draws
/// the same byte sequence -- sharing never drifts, and every animated tick
/// still costs exactly one arrangement.
///
/// The cycle walks left twice then right twice (`step` clamps rather than
/// wraps, so a march in one direction would stall at the edge watching
/// nothing): from the rightmost column, [L, L, R, R] focuses columns
/// [1, 0, 1, 2], and the second round repeats it from the same start.
#[test]
fn a_repeated_animation_draws_the_same_bytes() {
    let mut fixture = two_output_fixture(3);
    let cycle = |fixture: &mut Fixture| {
        let mut frames = Vec::new();
        for dir in [
            Horizontal::Left,
            Horizontal::Left,
            Horizontal::Right,
            Horizontal::Right,
        ] {
            let before = fixture.state.world.arrange();
            fixture.state.act(Action::FocusColumn(dir));
            assert_ne!(
                fixture.state.world.arrange(),
                before,
                "focus must move {dir:?}: otherwise this frame watches nothing"
            );
            reset_arrange_calls(fixture);
            let (first, second) = render_all(fixture);
            assert_eq!(
                arrange_calls(fixture),
                1,
                "an animated tick must arrange exactly once"
            );
            frames.push((first, second));
        }
        frames
    };
    let once = cycle(&mut fixture);
    let twice = cycle(&mut fixture);
    assert_ne!(
        once[0].0, once[1].0,
        "cycling focus must move the ring: otherwise this test watches nothing"
    );
    assert_eq!(once, twice, "the same animation must draw the same bytes");
}

/// Opening a window refreshes the next frame; closing it restores the exact
/// pre-open pixels on the populated output while the empty output never
/// moves -- freshness plus a byte-exact round trip.
#[test]
fn open_and_close_refresh() {
    let mut fixture = two_output_fixture(3);
    let (before_first, before_second) = render_all(&mut fixture);
    open_windows(&mut fixture, 100, 1);
    let (opened_first, opened_second) = render_all(&mut fixture);
    assert_ne!(
        opened_first, before_first,
        "an opened window must move output 1's rings"
    );
    assert_eq!(
        opened_second, before_second,
        "an opened window on output 1 must not touch output 2"
    );
    fixture
        .state
        .world
        .handle_event(CoreEvent::WindowClosed { id: WindowId(100) });
    let (closed_first, closed_second) = render_all(&mut fixture);
    assert_eq!(
        closed_first, before_first,
        "closing the window must restore output 1's exact pixels"
    );
    assert_eq!(
        closed_second, before_second,
        "output 2 must stay pristine throughout"
    );
}

/// Moving focus recolours the ring; moving it back restores the exact
/// pixels. The arrangement itself round-trips too, so a pixel mismatch
/// would blame the frame, not the layout.
#[test]
fn focus_change_refreshes_and_restores() {
    let mut fixture = two_output_fixture(3);
    let (before_first, _) = render_all(&mut fixture);
    let before = fixture.state.world.arrange();
    fixture.state.act(Action::FocusColumn(Horizontal::Left));
    assert_ne!(
        fixture.state.world.arrange(),
        before,
        "a focus move must re-lay-out (at least the focused window)"
    );
    let (moved_first, _) = render_all(&mut fixture);
    assert_ne!(
        moved_first, before_first,
        "a focus move must recolour output 1's rings"
    );
    // `act` reports spawn success, not movement, so the arrangement
    // comparison below is the real assertion that focus moved back.
    fixture.state.act(Action::FocusColumn(Horizontal::Right));
    assert_eq!(
        fixture.state.world.arrange(),
        before,
        "focus there and back must restore the arrangement"
    );
    let (back_first, _) = render_all(&mut fixture);
    assert_eq!(
        back_first, before_first,
        "focus there and back must restore output 1's exact pixels"
    );
}

/// Moving a window to another column moves its ring; the arrangement and
/// the pixels both round-trip on the way back.
#[test]
fn move_between_columns_refreshes_and_restores() {
    let mut fixture = two_output_fixture(3);
    let (before_first, _) = render_all(&mut fixture);
    let before = fixture.state.world.arrange();
    fixture.state.act(Action::ConsumeOrExpel(Horizontal::Left));
    assert_ne!(
        fixture.state.world.arrange(),
        before,
        "consuming a window must re-lay-out"
    );
    let (moved_first, _) = render_all(&mut fixture);
    assert_ne!(
        moved_first, before_first,
        "a moved window must move output 1's rings"
    );
    fixture.state.act(Action::ConsumeOrExpel(Horizontal::Right));
    assert_eq!(
        fixture.state.world.arrange(),
        before,
        "expelling it back must restore the arrangement"
    );
    let (back_first, _) = render_all(&mut fixture);
    assert_eq!(
        back_first, before_first,
        "expelling it back must restore output 1's exact pixels"
    );
}

/// Switching workspaces hides the strip; switching back restores the exact
/// pixels -- per-workspace placement refreshes through the shared
/// arrangement.
#[test]
fn workspace_switch_refreshes_and_restores() {
    let mut fixture = two_output_fixture(3);
    let (before_first, before_second) = render_all(&mut fixture);
    let before = fixture.state.world.arrange();
    fixture.state.act(Action::FocusWorkspace(Vertical::Down));
    assert_ne!(
        fixture.state.world.arrange(),
        before,
        "leaving the workspace must re-lay-out"
    );
    let (away_first, away_second) = render_all(&mut fixture);
    assert_ne!(
        away_first, before_first,
        "leaving the workspace must clear output 1's rings"
    );
    assert_eq!(
        away_second, before_second,
        "the workspace switch must not touch output 2"
    );
    fixture.state.act(Action::FocusWorkspace(Vertical::Up));
    assert_eq!(
        fixture.state.world.arrange(),
        before,
        "switching back must restore the arrangement"
    );
    let (back_first, back_second) = render_all(&mut fixture);
    assert_eq!(
        back_first, before_first,
        "switching back must restore output 1's exact pixels"
    );
    assert_eq!(
        back_second, before_second,
        "output 2 must stay pristine throughout"
    );
}

/// Resizing the output re-lays-out against the new geometry (the arrangement
/// itself changes); restoring the size restores the exact pixels.
#[test]
fn output_resize_refreshes_and_restores() {
    let mut fixture = two_output_fixture(3);
    let (before_first, _) = render_all(&mut fixture);
    let before = fixture.state.world.arrange();
    assert!(
        fixture.state.resize_output(CANVAS + 40, CANVAS),
        "the resize must apply"
    );
    assert_ne!(
        fixture.state.world.arrange(),
        before,
        "a resize must re-lay-out against the new geometry"
    );
    // The resized framebuffer is a different size, so no byte comparison
    // here -- only that the frame draws (which exercises the shared
    // arrangement at the new geometry).
    fixture.state.request_render();
    fixture.state.render();
    assert!(
        fixture.state.resize_output(CANVAS, CANVAS),
        "restoring the size must apply"
    );
    assert_eq!(
        fixture.state.world.arrange(),
        before,
        "restoring the size must restore the arrangement"
    );
    let (back_first, _) = render_all(&mut fixture);
    assert_eq!(
        back_first, before_first,
        "restoring the size must restore output 1's exact pixels"
    );
}

/// Removing the second output leaves the first output's exact pixels
/// untouched, and re-adding an output does not shift them either -- the
/// shared arrangement never blends one output's state into another's.
#[test]
fn output_remove_and_readd_leave_the_survivor_alone() {
    let mut fixture = two_output_fixture(3);
    let (before_first, _) = render_all(&mut fixture);
    assert!(
        fixture.state.remove_output(OutputId(2)),
        "removing the second output must succeed"
    );
    fixture.state.request_render();
    fixture.state.render();
    assert_eq!(
        fixture.pixels_of(OutputId(1)),
        before_first,
        "removing output 2 must not move output 1's pixels"
    );
    headless::add_output(&mut fixture.state, "headless-3", CANVAS, CANVAS)
        .expect("a re-added output");
    let (readded_first, readded_third) = {
        fixture.state.request_render();
        fixture.state.render();
        (
            fixture.pixels_of(OutputId(1)),
            fixture.pixels_of(OutputId(3)),
        )
    };
    assert_eq!(
        readded_first, before_first,
        "re-adding an output must not move output 1's pixels"
    );
    let background = readded_third[0..4].to_vec();
    assert!(
        readded_third
            .chunks_exact(4)
            .all(|pixel| pixel == background),
        "the re-added output must be pristine background"
    );
}

/// Locking blanks both outputs without arranging (locked frames gather no
/// arrangement at all); unlocking restores the exact pre-lock pixels --
/// the lock-state half of the invalidation story.
#[test]
fn relock_refreshes_and_restores() {
    let mut fixture = two_output_fixture(3);
    let (before_first, before_second) = render_all(&mut fixture);
    let locker = fixture.spawn(test_support::locker);
    fixture.wait_for_ack(locker);
    let deadline = Instant::now() + Duration::from_secs(10);
    while !fixture.state.session_lock.is_locked() {
        assert!(
            Instant::now() < deadline,
            "the lock request never landed; the frames below would pass unlocked"
        );
        fixture
            .event_loop
            .dispatch(Some(Duration::from_millis(5)), &mut fixture.state)
            .expect("a compositor dispatch");
    }
    reset_arrange_calls(&fixture);
    let (locked_first, locked_second) = render_all(&mut fixture);
    assert_eq!(
        arrange_calls(&fixture),
        0,
        "a locked tick must not arrange at all"
    );
    assert_ne!(locked_first, before_first, "locking must blank output 1");
    assert_eq!(
        locked_first, locked_second,
        "with no lock surfaces both outputs must show the same blank"
    );
    fixture.disconnect(locker);
    let deadline = Instant::now() + Duration::from_secs(10);
    while fixture.state.session_lock.is_locked() {
        assert!(
            Instant::now() < deadline,
            "the unlock never landed; the frame below would pass locked"
        );
        fixture
            .event_loop
            .dispatch(Some(Duration::from_millis(5)), &mut fixture.state)
            .expect("a compositor dispatch");
    }
    let (back_first, back_second) = render_all(&mut fixture);
    assert_eq!(
        back_first, before_first,
        "unlocking must restore output 1's exact pixels"
    );
    assert_eq!(
        back_second, before_second,
        "unlocking must restore output 2's exact pixels"
    );
}

/// Scale never enters the arrangement: the same scene at 1.0 and 2.0 shares
/// the same code path (one arrangement per tick each), draws its rings at
/// both scales, and the per-output scale reaches only the frame's element
/// gathering downstream.
#[test]
fn scale_reaches_only_the_gathering() {
    let mut plain = Fixture::headless(Appearance::default(), CANVAS);
    open_windows(&mut plain, 1, 3);
    let mut scaled = Harness::headless_scaled(Appearance::default(), CANVAS, 2.0);
    open_windows(&mut scaled, 1, 3);
    reset_arrange_calls(&plain);
    reset_arrange_calls(&scaled);
    let plain_pixels = plain.render();
    let scaled_pixels = scaled.render();
    assert_eq!(
        arrange_calls(&plain),
        1,
        "the unscaled tick must arrange exactly once"
    );
    assert_eq!(
        arrange_calls(&scaled),
        1,
        "the scaled tick must arrange exactly once"
    );
    for (what, pixels) in [("unscaled", &plain_pixels), ("scaled", &scaled_pixels)] {
        let background = pixels[0..4].to_vec();
        assert!(
            pixels.chunks_exact(4).any(|pixel| pixel != background),
            "{what}: the frame must draw its rings from the shared arrangement"
        );
    }
}
