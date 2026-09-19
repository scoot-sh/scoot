//! Unit tests for the two decisions `--nested` makes about a host configure.
//!
//! [`Host`](super::Host) itself needs a live host compositor to construct, so
//! neither entry point (`apply_first_configure`, `apply_resize`) nor
//! `present`'s buffer handling can be driven from here -- the buffer pool's
//! own logic is tested in `buffers.rs`, where it needs no connection, and the
//! two entry points are pinned live by the `--nested` resize run recorded on
//! the PR. What *is* testable is what this module deliberately keeps as free
//! functions over plain values: which of the two entry points a configure
//! goes to, and which proposed sizes are acted on at all.

use super::{ConfigureAction, configure_action, usable_size};
use crate::cli::MAX_OUTPUT_DIMENSION;

const STARTED_AT: (i32, i32) = (1280, 800);

#[test]
fn the_first_configure_is_the_first_configure_even_at_the_starting_size() {
    // A host that proposes exactly what scoot asked for still has to build
    // the render target: nothing exists yet. Returning `Nothing` here would
    // leave the surface unconfigured forever, and xdg-shell forbids attaching
    // a buffer until it is -- a window that never shows anything.
    assert_eq!(
        configure_action(false, STARTED_AT, STARTED_AT),
        ConfigureAction::FirstConfigure
    );
}

#[test]
fn the_first_configure_at_a_different_size_is_still_the_first_configure() {
    assert_eq!(
        configure_action(false, (1920, 1080), STARTED_AT),
        ConfigureAction::FirstConfigure
    );
}

#[test]
fn a_later_configure_at_a_new_size_resizes() {
    // Issue #144: this is the case that used to return early.
    assert_eq!(
        configure_action(true, (1920, 1080), STARTED_AT),
        ConfigureAction::Resize
    );
}

#[test]
fn one_axis_moving_is_enough_to_resize() {
    assert_eq!(
        configure_action(true, (1280, 801), STARTED_AT),
        ConfigureAction::Resize
    );
    assert_eq!(
        configure_action(true, (1281, 800), STARTED_AT),
        ConfigureAction::Resize
    );
}

#[test]
fn a_later_configure_at_the_same_size_does_nothing() {
    // Load-bearing, not an optimisation: hosts re-send a configure on every
    // state change that is not a resize (activation, maximize, a tiling-edge
    // update), so without this every focus change in the host would throw
    // away a working render target and buffer pool to build an identical
    // pair -- a whole new EGL context under `--renderer gles`.
    assert_eq!(
        configure_action(true, STARTED_AT, STARTED_AT),
        ConfigureAction::Nothing
    );
}

#[test]
fn an_ordinary_size_is_usable() {
    assert_eq!(usable_size(1280, 800), Some((1280, 800)));
    assert_eq!(usable_size(1, 1), Some((1, 1)));
    assert_eq!(
        usable_size(MAX_OUTPUT_DIMENSION, MAX_OUTPUT_DIMENSION),
        Some((MAX_OUTPUT_DIMENSION, MAX_OUTPUT_DIMENSION))
    );
}

#[test]
fn a_zero_axis_is_the_hosts_you_choose_and_is_not_usable() {
    // xdg-shell's way of saying "pick that dimension yourself". Dropping the
    // whole proposal keeps the size scoot is already at; the alternative
    // (half-applying it) is not what this has ever done.
    assert_eq!(usable_size(0, 0), None);
    assert_eq!(usable_size(0, 800), None);
    assert_eq!(usable_size(1280, 0), None);
}

#[test]
fn a_negative_axis_is_not_usable() {
    // Nothing a correct host sends, but the wire carries `int`s and a
    // `width as usize` on a negative one is how a buffer pool asks for
    // sixteen exabytes.
    assert_eq!(usable_size(-1, 800), None);
    assert_eq!(usable_size(1280, -1), None);
    assert_eq!(usable_size(i32::MIN, i32::MIN), None);
}

#[test]
fn an_axis_past_the_output_bound_is_not_usable() {
    // The same `1..=65535` `--width`/`--height` are parsed into: DRM reports
    // a mode axis in a `u16`, so a bigger one is not a mode any client could
    // believe. Refused rather than clamped, matching `cli::dimension`.
    assert_eq!(usable_size(MAX_OUTPUT_DIMENSION + 1, 800), None);
    assert_eq!(usable_size(1280, MAX_OUTPUT_DIMENSION + 1), None);
    assert_eq!(usable_size(i32::MAX, i32::MAX), None);
}
