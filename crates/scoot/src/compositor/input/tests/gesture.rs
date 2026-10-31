//! What a client's `wl_pointer` hears of a swipe. scoot offers no
//! `zwp_pointer_gestures_v1` yet, so the answer is nothing at all: not even
//! the empty `wl_pointer.frame` a forwarded swipe used to send at update
//! rate.

use super::*;

#[test]
fn a_swipe_wakes_no_client_claimed_or_forwarded() {
    let mut fixture = ScrollFixture::new();
    let before = fixture.scroll_report().frames;
    let state = &mut fixture.fixture.state;
    // Scoot's (3 fingers) and not scoot's (2 and 4), a long update run each.
    for fingers in [3, 2, 4] {
        state.gesture_swipe_begin(fingers, 1000);
        for i in 0..50 {
            state.gesture_swipe_update(-3.0, 0.5, 1010 + 10 * i);
        }
        state.gesture_swipe_end(false, 2000);
    }
    fixture.fixture.settle();
    let after = fixture.scroll_report();
    assert!(after.entered, "premise: the client holds pointer focus");
    assert_eq!(
        after.frames, before,
        "a swipe sent the focused client wl_pointer.frame events"
    );
    assert!(after.axes.is_empty() && after.stops.is_empty());
}
