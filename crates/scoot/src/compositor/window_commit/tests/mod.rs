//! [`super::State::note_window_commit`] and [`super::State::flush_window_commits`]:
//! the pending set's dedup and drain, without any clients. The batch
//! behaviour with real windows lives in [`batch`].

use scoot_core::{Config, WindowId};
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::wayland_server::Display;

use crate::compositor::decorations::Appearance;
use crate::compositor::keybindings::Keybindings;
use crate::compositor::state::State;
use crate::compositor::test_support::test_renderer;

mod batch;

/// A compositor with no clients and no windows: enough to exercise the
/// pending set, which never touches a window until the flush.
fn bare_state() -> (EventLoop<'static, State>, State) {
    let mut event_loop: EventLoop<'static, State> = EventLoop::try_new().expect("an event loop");
    let display: Display<State> = Display::new().expect("a wayland display");
    let state = State::new(
        &mut event_loop,
        display,
        Config::default(),
        Keybindings::default(),
        Appearance::default(),
        1.0,
        test_renderer(),
    )
    .expect("a compositor state with a wayland socket");
    (event_loop, state)
}

#[test]
fn repeated_marks_for_one_window_coalesce_to_one_slot() {
    let (_loop, mut state) = bare_state();
    assert!(state.pending_window_commits.is_empty());
    for _ in 0..5 {
        state.note_window_commit(WindowId(7));
    }
    assert_eq!(state.pending_window_commits, vec![WindowId(7)]);
    state.note_window_commit(WindowId(3));
    state.note_window_commit(WindowId(7));
    // First-dirtied order, each window once.
    assert_eq!(state.pending_window_commits, vec![WindowId(7), WindowId(3)]);
}

#[test]
fn flush_drains_the_set_and_skips_windows_that_are_gone() {
    let (_loop, mut state) = bare_state();
    let capacity = {
        for id in 0..8 {
            state.note_window_commit(WindowId(id));
        }
        state.pending_window_commits.capacity()
    };
    // No windows exist, so nothing is recomputed -- and in particular a
    // window closed between its commit and the flush (every id here) is
    // skipped rather than resurrected.
    assert_eq!(state.flush_window_commits(), 0);
    assert_eq!(state.last_window_commit_flush, 0);
    assert!(state.pending_window_commits.is_empty());
    // Pooled: the drain keeps the capacity, so the next batch does not
    // reallocate for the same shape.
    assert_eq!(state.pending_window_commits.capacity(), capacity);
}

#[test]
fn a_mark_after_a_flush_starts_a_new_set() {
    let (_loop, mut state) = bare_state();
    state.note_window_commit(WindowId(1));
    assert_eq!(state.flush_window_commits(), 0);
    state.note_window_commit(WindowId(1));
    assert_eq!(state.pending_window_commits, vec![WindowId(1)]);
}
