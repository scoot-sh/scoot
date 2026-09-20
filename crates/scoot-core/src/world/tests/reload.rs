//! Replacing the layout tunables on a live world: what a config reload drives.

use super::*;
use crate::{Config, Event, OutputId};

#[test]
fn set_config_validates_like_new_and_moves_the_next_arrangement() {
    let mut world = world();
    open(&mut world, 1);
    let before = placement(&world, 1);
    world.set_config(Config {
        gap: 60,
        ..config()
    });
    assert_eq!(world.config().gap, 60);
    let after = placement(&world, 1);
    assert_ne!(
        (before.rect.w, before.rect.h),
        (after.rect.w, after.rect.h),
        "a wider gap should inset the placed frame"
    );
    assert_eq!(after.rect.x, 60);
    assert_eq!(after.rect.y, 60);
}

#[test]
fn set_config_clamps_an_absurd_gap_instead_of_overflowing() {
    let mut world = world();
    world.set_config(Config {
        gap: i32::MAX,
        ..config()
    });
    assert_eq!(world.config().gap, Config::MAX_GAP);
    // Must not panic: the capped gap keeps every gap-derived sum in range.
    let _ = world.arrange();
}

#[test]
fn set_config_repairs_an_unusable_width_list_the_way_new_does() {
    let mut world = world();
    world.set_config(Config {
        column_widths: vec![0.0, f64::NAN],
        default_column_width: 9,
        ..config()
    });
    assert_eq!(
        world.config().column_widths,
        Config::default().column_widths
    );
    assert!(world.config().default_column_width < world.config().column_widths.len());
}
#[test]
fn set_config_keeps_existing_columns_placeable() {
    // The compositor refuses a `column_widths` shrink past live presets (see
    // `reload`), but `set_config` itself must never leave `arrange` indexing
    // out of range for whatever it *is* handed: validation keeps the list
    // non-empty, and presets below the old length stay valid.
    let mut world = world();
    open(&mut world, 1);
    world.set_config(Config {
        column_widths: vec![0.25, 0.5, 0.75, 1.0],
        ..config()
    });
    let _ = world.arrange();
    world.handle_event(Event::OutputAdded {
        id: OutputId(2),
        area: SCREEN,
    });
    let _ = world.arrange();
}

// The backstop for `set_config`'s `debug_assert!`: a caller handing a list
// shorter than a live preset is a bug the compositor's refusal path must
// have caught first, so this fails loudly instead of letting `arrange`
// index out of range. Gated on `debug_assertions` because release compiles
// the assert out -- without the gate this test would fail a release run by
// succeeding.
#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "live column preset")]
fn set_config_shouts_when_a_live_preset_outruns_the_incoming_list() {
    let mut world = World::new(Config {
        column_widths: vec![0.25, 0.5, 0.75, 1.0],
        default_column_width: 3,
        ..config()
    });
    world.handle_event(Event::OutputAdded {
        id: OutputId(1),
        area: SCREEN,
    });
    open(&mut world, 1);
    world.set_config(Config {
        column_widths: vec![0.5],
        ..config()
    });
}
