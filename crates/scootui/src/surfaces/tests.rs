//! The lifecycle without a compositor: every ordering of events the
//! glue can forward, hostile sizes, and the buffer-slot policy.

use wayland_client::protocol::wl_shm;

use super::{Demand, Effect, Frame, Plan, SlotState, pick};
use crate::density::Scale;
use crate::output::Size;

fn size(width: u32, height: u32) -> Size {
    Size { width, height }
}

#[test]
fn show_hide_is_create_then_destroy() {
    let mut demand = Demand::new();
    assert!(!demand.is_live());
    assert_eq!(demand.show(), Effect::Create);
    assert!(demand.is_live());
    assert_eq!(demand.hide(), Effect::Destroy);
    assert!(!demand.is_live());
}

#[test]
fn repeated_create_and_hide_within_one_dispatch_are_one() {
    let mut demand = Demand::new();
    // A second show before the first surface exists is a no-op: the app
    // carries Create out once, however many cards arrived in one read.
    assert_eq!(demand.show(), Effect::Create);
    assert_eq!(demand.show(), Effect::None);
    assert_eq!(demand.show(), Effect::None);
    // And hiding what is already gone destroys nothing.
    assert_eq!(demand.hide(), Effect::Destroy);
    assert_eq!(demand.hide(), Effect::None);
    assert_eq!(demand.hide(), Effect::None);
}

#[test]
fn a_configure_is_remembered_and_acked() {
    let mut demand = Demand::new();
    assert_eq!(demand.show(), Effect::Create);
    assert_eq!(demand.configure(7, 400, 200), Effect::Ack(7));
    assert_eq!(demand.configured_size(), Some(size(400, 200)));
}

#[test]
fn a_configure_after_destroy_is_stale() {
    let mut demand = Demand::new();
    // No surface at all: nothing to ack.
    assert_eq!(demand.configure(1, 100, 100), Effect::None);
    assert_eq!(demand.show(), Effect::Create);
    assert_eq!(demand.configure(2, 100, 100), Effect::Ack(2));
    assert_eq!(demand.hide(), Effect::Destroy);
    // The compositor may have a configure in flight for the destroyed
    // surface (the glue drops those by object identity first; the
    // tracker guards the rest).
    assert_eq!(demand.configure(3, 100, 100), Effect::None);
    assert_eq!(demand.closed(), Effect::None);
    assert_eq!(demand.configured_size(), None);
}

#[test]
fn the_compositor_closing_retries_once_then_gives_up() {
    let mut demand = Demand::new();
    assert_eq!(demand.show(), Effect::Create);
    assert_eq!(demand.closed(), Effect::DestroyAndRetry);
    assert!(!demand.is_live());
    // A show while the retry is scheduled changes nothing: the retry
    // creates.
    assert_eq!(demand.show(), Effect::None);
    assert_eq!(demand.retry(), Effect::Create);
    assert!(demand.is_live());
    // Closed again over the same content: given up until the content
    // clears (one retry per showing, as the bar's).
    assert_eq!(demand.closed(), Effect::DestroyAndGiveUp);
    assert_eq!(demand.retry(), Effect::None);
    assert_eq!(demand.show(), Effect::None);
    // Clearing the content is a fresh start with one chance.
    assert_eq!(demand.hide(), Effect::None);
    assert_eq!(demand.show(), Effect::Create);
    assert_eq!(demand.closed(), Effect::DestroyAndRetry);
}

#[test]
fn an_output_removed_while_mapped_destroys_with_no_retry() {
    let mut demand = Demand::new();
    assert_eq!(demand.show(), Effect::Create);
    assert_eq!(demand.configure(1, 800, 600), Effect::Ack(1));
    assert_eq!(demand.output_removed(), Effect::Destroy);
    assert!(!demand.is_live());
    // The retry callback landing after the output went does nothing.
    assert_eq!(demand.retry(), Effect::None);
    // A new output is a fresh start.
    assert_eq!(demand.show(), Effect::Create);
}

#[test]
fn an_output_removed_while_idle_is_nothing() {
    let mut demand = Demand::new();
    assert_eq!(demand.output_removed(), Effect::None);
    assert_eq!(demand.retry(), Effect::None);
}

#[test]
fn a_past_int32_configure_side_is_taken_as_zero() {
    // A compositor's negative size sent as the protocol's uint (the bar
    // met sway 1.12 configuring 4294966528 wide): no surface can be that
    // size, so it is "yours to choose", resolved against the content.
    let mut demand = Demand::new();
    assert_eq!(demand.show(), Effect::Create);
    assert_eq!(demand.configure(9, u32::MAX, 100), Effect::Ack(9));
    assert_eq!(demand.configured_size(), Some(size(0, 100)));
    // What fits stays.
    assert_eq!(demand.configure(10, i32::MAX as u32, 100), Effect::Ack(10));
    assert_eq!(demand.configured_size(), Some(size(i32::MAX as u32, 100)));
}

#[test]
fn a_zero_axis_stays_zero_for_the_content_to_resolve() {
    let mut demand = Demand::new();
    assert_eq!(demand.show(), Effect::Create);
    assert_eq!(demand.configure(4, 0, 0), Effect::Ack(4));
    assert_eq!(demand.configured_size(), Some(size(0, 0)));
}

#[test]
fn broken_output_scales_are_ignored() {
    let mut demand = Demand::new();
    demand.set_output_scale(0);
    demand.set_output_scale(-2);
    assert_eq!(demand.scale(), Scale::Integer(1));
    demand.set_output_scale(2);
    assert_eq!(demand.scale(), Scale::Integer(2));
}

#[test]
fn plan_draws_commits_and_rests() {
    let mut demand = Demand::new();
    // Not configured: nothing to do, however stale.
    assert_eq!(demand.plan(None, true), Plan::Nothing);
    assert_eq!(demand.show(), Effect::Create);
    assert_eq!(demand.plan(None, true), Plan::Nothing);
    assert_eq!(demand.configure(1, 400, 200), Effect::Ack(1));
    let frame = Frame {
        size: size(400, 200),
        scale: Scale::Integer(1),
    };
    // First draw at a new frame.
    assert_eq!(demand.plan(Some(size(400, 200)), false), Plan::Draw(frame));
    demand.drew(frame);
    // The same pixels: nothing.
    assert_eq!(demand.plan(Some(size(400, 200)), false), Plan::Nothing);
    // New content at the same frame draws again.
    assert_eq!(demand.plan(Some(size(400, 200)), true), Plan::Draw(frame));
    demand.drew(frame);
    // A configure that changes nothing still needs a commit for the ack.
    assert_eq!(demand.configure(2, 400, 200), Effect::Ack(2));
    assert_eq!(demand.plan(Some(size(400, 200)), false), Plan::Commit);
    demand.committed();
    assert_eq!(demand.plan(Some(size(400, 200)), false), Plan::Nothing);
}

#[test]
fn a_failed_draw_retries_then_goes_quiet_until_the_frame_changes() {
    let mut demand = Demand::new();
    assert_eq!(demand.show(), Effect::Create);
    assert_eq!(demand.configure(1, 400, 200), Effect::Ack(1));
    let frame = Frame {
        size: size(400, 200),
        scale: Scale::Integer(1),
    };
    assert_eq!(demand.plan(Some(size(400, 200)), false), Plan::Draw(frame));
    // Three retries, said once, then quiet at this frame.
    assert!(demand.draw_failed(frame));
    assert!(!demand.draw_failed(frame));
    assert!(!demand.draw_failed(frame));
    assert_eq!(demand.plan(Some(size(400, 200)), false), Plan::Draw(frame));
    assert!(!demand.draw_failed(frame));
    assert_eq!(demand.plan(Some(size(400, 200)), false), Plan::Nothing);
    // A new frame (or size) is a new chance.
    let moved = Frame {
        size: size(400, 201),
        scale: Scale::Integer(1),
    };
    assert_eq!(demand.plan(Some(size(400, 201)), false), Plan::Draw(moved));
    assert!(demand.draw_failed(moved));
}

#[test]
fn a_shown_draw_ends_the_failure_streak() {
    let mut demand = Demand::new();
    assert_eq!(demand.show(), Effect::Create);
    assert_eq!(demand.configure(1, 400, 200), Effect::Ack(1));
    let frame = Frame {
        size: size(400, 200),
        scale: Scale::Integer(1),
    };
    assert!(demand.draw_failed(frame));
    demand.drew(frame);
    // Drawing again succeeded, so the next failure starts a new streak
    // (and is said again).
    assert!(demand.draw_failed(frame));
}

#[test]
fn hostile_event_storms_hold_the_lifecycle_invariants() {
    // Every ordering of events the glue can forward, with hostile sizes:
    // never a panic, never two Creates without a destroy between them,
    // an Ack echoing only a live surface's serial, and hiding always
    // leaving nothing live.
    let mut state: u64 = 0x1234_5678_9abc_def0;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let sizes = [0, 1, 2, 100, 1600, i32::MAX as u32, u32::MAX - 1, u32::MAX];
    for round in 0..2000u32 {
        let mut demand = Demand::new();
        let _ = demand.show();
        let mut created = true;
        for _ in 0..20 {
            let op = next() % 6;
            let serial = next() as u32;
            let (w, h) = (
                sizes[next() as usize % sizes.len()],
                sizes[next() as usize % sizes.len()],
            );
            let effect = match op {
                0 => demand.show(),
                1 => demand.hide(),
                2 => demand.configure(serial, w, h),
                3 => demand.closed(),
                4 => demand.output_removed(),
                _ => demand.retry(),
            };
            match effect {
                Effect::Create => {
                    assert!(created, "two Creates without a destroy (round {round})");
                    assert!(demand.is_live());
                    created = false;
                }
                Effect::Ack(echo) => {
                    assert_eq!(echo, serial);
                    assert!(demand.is_live());
                }
                Effect::Destroy | Effect::DestroyAndRetry | Effect::DestroyAndGiveUp => {
                    assert!(!demand.is_live());
                    created = true;
                }
                Effect::None => {}
            }
            if effect == Effect::Destroy
                && let Some(_live) = demand.configured_size()
            {
                panic!("a destroyed surface still reports a size (round {round})");
            }
        }
        let _ = round;
    }
}

#[test]
fn a_hot_turn_allocates_nothing() {
    // What a loop turn does with content on screen: forward a configure
    // (or a repeat one), ask the plan, mark the draw.
    let ((), allocations) = scootbg_mem::count_allocations(|| {
        let mut demand = Demand::new();
        let frame = Frame {
            size: size(400, 200),
            scale: Scale::Integer(1),
        };
        for serial in 0..300u32 {
            let _ = demand.show();
            let _ = demand.configure(serial, 400, 200);
            let _ = demand.plan(Some(size(400, 200)), serial % 7 == 0);
            demand.drew(frame);
            demand.committed();
            let _ = demand.hide();
        }
    });
    assert_eq!(allocations, 0);
}

#[test]
fn buffer_dims_overflow_is_no_buffer() {
    let mut demand = Demand::new();
    // Integer scale 1 until the compositor's own scale events arrive.
    assert_eq!(demand.buffer_dims(size(100, 100)), Some((100, 100)));
    // A hostile size whose scaling overflows: no buffer until the size
    // changes.
    demand.prefer_fractional(240); // Twice the logical size.
    assert_eq!(demand.buffer_dims(size(u32::MAX, 1)), None);
    assert_eq!(
        demand.buffer_dims(size(100, 100)),
        Some((200, 200)),
        "what fits scaled still draws"
    );
}

#[test]
fn pick_reuses_a_free_buffer_of_the_size_and_format() {
    use wl_shm::Format::{Argb8888, Xrgb8888};
    let slots = [
        Some(SlotState {
            dims: (10, 10),
            held: false,
            format: Xrgb8888,
        }),
        Some(SlotState {
            dims: (10, 10),
            held: false,
            format: Argb8888,
        }),
    ];
    assert_eq!(pick(slots.iter().copied(), (10, 10), Xrgb8888), Some(0));
    assert_eq!(pick(slots.iter().copied(), (10, 10), Argb8888), Some(1));
}

#[test]
fn pick_takes_an_empty_slot_before_replacing() {
    use wl_shm::Format::Xrgb8888;
    let slots = [
        Some(SlotState {
            dims: (10, 10),
            held: false,
            format: Xrgb8888,
        }),
        None,
    ];
    assert_eq!(pick(slots.iter().copied(), (20, 20), Xrgb8888), Some(1));
}

#[test]
fn pick_replaces_a_free_slot_of_another_size() {
    use wl_shm::Format::Xrgb8888;
    let slots = [
        Some(SlotState {
            dims: (10, 10),
            held: false,
            format: Xrgb8888,
        }),
        Some(SlotState {
            dims: (30, 30),
            held: false,
            format: Xrgb8888,
        }),
    ];
    assert_eq!(pick(slots.iter().copied(), (20, 20), Xrgb8888), Some(0));
}

#[test]
fn pick_is_none_while_the_compositor_holds_everything() {
    use wl_shm::Format::Xrgb8888;
    let slots = [
        Some(SlotState {
            dims: (20, 20),
            held: true,
            format: Xrgb8888,
        }),
        Some(SlotState {
            dims: (20, 20),
            held: true,
            format: Xrgb8888,
        }),
    ];
    assert_eq!(pick(slots.iter().copied(), (20, 20), Xrgb8888), None);
    let empty: [Option<SlotState>; 2] = [None, None];
    assert_eq!(pick(empty.iter().copied(), (20, 20), Xrgb8888), Some(0));
}
