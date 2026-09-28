//! Phase 5: capture. There is no X-specific capture path to test -- the IPC
//! screenshot and `ext-image-copy-capture-v1` both read an output's
//! composited framebuffer, and X windows are composited like any other --
//! so these pin that the claim holds rather than trusting it: a managed X
//! window and an override-redirect menu are in both captures, byte for byte
//! the frame on screen; each output's captures carry that output's X
//! windows and no other's; and neither capture carries an X pixel once the
//! session is locked.
//!
//! Both captures are compared against the framebuffer read back directly,
//! and both are checked for the X colors themselves, so neither "the
//! capture is the framebuffer" nor "the capture shows red" can pass alone
//! against a capture of the wrong thing.

use scoot_core::OutputId;

use super::live::{BLUE, BLUE_BGRA, CANVAS, RED, RED_BGRA, live, live_on};
use super::peer::{Ack, CaptureStep, Step};
use super::x11::{Props, eventually};
use crate::compositor::test_support::contains;

/// A managed X window of `RED` and an override-redirect `BLUE` menu at
/// `(menu_x, 20)` in X root coordinates (global logical ones here, at scale
/// 1), both drawn.
fn map_window_and_menu(live: &mut super::live::Live, menu_x: i16) {
    let xid = live.x.map(&Props::new(RED));
    live.managed(xid);
    let mut menu = Props::new(BLUE);
    menu.rect = (menu_x, 20, 50, 40);
    menu.override_redirect = true;
    let menu = live.x.map(&menu);
    eventually(&mut live.fixture, "the menu drawn", |fixture| {
        fixture
            .state
            .x11_unmanaged
            .iter()
            .any(|known| known.window_id() == menu && known.wl_surface().is_some())
    });
    live.drain();
}

/// The IPC screenshot's pixels of output `id` (`scoot msg screenshot
/// --output`, before the PNG encode), without the pointer.
fn screenshot(live: &mut super::live::Live, id: OutputId) -> Vec<u8> {
    live.fixture
        .state
        .capture_pixels_for(Some(id), false)
        .expect("an IPC capture")
        .bgra
}

/// An `ext-image-copy-capture-v1` capture of the output the peer's registry
/// listed `index`th, as the capturing client read its buffer back.
fn screencopy(live: &mut super::live::Live, index: usize) -> Vec<u8> {
    match live.fixture.run(Step::Capture(CaptureStep::Output(index))) {
        Ack::Captured(Ok(pixels)) => pixels,
        Ack::Captured(Err(why)) => panic!("the screen capture did not complete: {why}"),
        other => panic!("expected a capture, got {other:?}"),
    }
}

#[test]
fn x_windows_are_in_the_screenshot_and_the_screen_capture() {
    let Some(mut live) = live("x_windows_are_in_the_screenshot_and_the_screen_capture") else {
        return;
    };
    map_window_and_menu(&mut live, 20);
    let primary = live.fixture.state.outputs.primary_id().expect("an output");

    let shot = screenshot(&mut live, primary);
    let framebuffer = live.fixture.pixels_of(primary);
    assert!(
        contains(&shot, RED_BGRA),
        "the managed X window is not in the screenshot"
    );
    assert!(
        contains(&shot, BLUE_BGRA),
        "the X menu is not in the screenshot"
    );
    assert_eq!(
        shot, framebuffer,
        "the screenshot is not the frame on screen"
    );

    let captured = screencopy(&mut live, 0);
    let framebuffer = live.fixture.pixels_of(primary);
    assert!(
        contains(&captured, RED_BGRA),
        "the managed X window is not in the screen capture"
    );
    assert!(
        contains(&captured, BLUE_BGRA),
        "the X menu is not in the screen capture"
    );
    assert_eq!(
        captured, framebuffer,
        "an Argb8888 screen capture is not the frame on screen byte for byte"
    );
}

#[test]
fn each_output_captures_its_own_x_windows_and_no_other() {
    let Some(mut live) = live_on("each_output_captures_its_own_x_windows_and_no_other", 2) else {
        return;
    };
    let second = OutputId(2);
    let origin = {
        let state = &live.fixture.state;
        let output = state.outputs.get(second).expect("a second output").clone();
        state
            .space
            .output_geometry(&output)
            .expect("the second output is placed")
            .loc
    };
    assert!(origin.x >= CANVAS, "the outputs must sit side by side");
    // A window opens on the output under the pointer.
    live.fixture.state.pointer_move(
        f64::from(origin.x) + f64::from(CANVAS) / 2.0,
        f64::from(CANVAS) / 2.0,
    );
    let menu_x = i16::try_from(origin.x + 20).expect("a menu position in X range");
    map_window_and_menu(&mut live, menu_x);

    let first = OutputId(1);
    for (what, pixels) in [
        ("screenshot", screenshot(&mut live, first)),
        ("screen capture", screencopy(&mut live, 0)),
    ] {
        assert!(
            !contains(&pixels, RED_BGRA),
            "output 1's {what} shows the X window on output 2"
        );
        assert!(
            !contains(&pixels, BLUE_BGRA),
            "output 1's {what} shows the X menu on output 2"
        );
    }
    for (what, pixels) in [
        ("screenshot", screenshot(&mut live, second)),
        ("screen capture", screencopy(&mut live, 1)),
    ] {
        assert!(
            contains(&pixels, RED_BGRA),
            "output 2's {what} is missing its X window"
        );
        assert!(
            contains(&pixels, BLUE_BGRA),
            "output 2's {what} is missing its X menu"
        );
    }
}

#[test]
fn no_x_pixel_reaches_a_capture_under_the_lock() {
    let Some(mut live) = live("no_x_pixel_reaches_a_capture_under_the_lock") else {
        return;
    };
    map_window_and_menu(&mut live, 20);
    let primary = live.fixture.state.outputs.primary_id().expect("an output");
    // The premise: both are captured before the lock.
    let before = screenshot(&mut live, primary);
    assert!(contains(&before, RED_BGRA) && contains(&before, BLUE_BGRA));

    assert!(matches!(live.fixture.run(Step::Lock), Ack::Done));
    eventually(&mut live.fixture, "the session locking", |fixture| {
        fixture.state.session_lock.is_locked()
    });
    live.drain();

    for (what, pixels) in [
        ("screenshot", screenshot(&mut live, primary)),
        ("screen capture", screencopy(&mut live, 0)),
    ] {
        assert!(
            !contains(&pixels, RED_BGRA),
            "the X window reached a {what} under the lock"
        );
        assert!(
            !contains(&pixels, BLUE_BGRA),
            "the X menu reached a {what} under the lock"
        );
    }
}
