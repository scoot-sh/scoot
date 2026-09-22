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

// The clamp pins for `set_config`'s width-list handling: a caller handing a
// list shorter than a live preset clamps the preset into range instead of
// leaving `arrange` indexing out of range (the refusal path that used to sit
// in front of this is gone -- a reload now applies width changes live).
#[test]
fn set_config_clamps_a_live_preset_past_a_shorter_list() {
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
    // The open window sits on the default preset, the last of four.
    world.set_config(Config {
        column_widths: vec![0.5],
        ..config()
    });
    assert_eq!(world.config().column_widths, vec![0.5]);
    // Must not panic: the clamped preset keeps every `arrange` index in
    // range, and the column lands on the nearest surviving width -- the
    // 0.5 entry, 485px of the 980px usable width (see `tests::config`).
    let placed = placement(&world, 1);
    assert_eq!(placed.rect.w, 485);
}

#[test]
fn set_config_leaves_presets_alone_when_the_list_grows_or_holds() {
    let mut world = world();
    open(&mut world, 1);
    open(&mut world, 2);
    world.handle_action(crate::Action::SetColumnWidth(1));
    world.set_config(Config {
        column_widths: vec![0.25, 0.5, 0.75, 1.0],
        ..config()
    });
    // Window 2's column keeps preset 1 (the 0.5 entry, 485px); window 1's
    // keeps the default preset 0 (the 0.25 entry, 238px).
    assert_eq!(placement(&world, 2).rect.w, 485);
    assert_eq!(placement(&world, 1).rect.w, 238);
    let _ = world.arrange();
}

#[test]
fn set_config_clamps_the_default_for_windows_opened_after() {
    let mut world = world();
    world.set_config(Config {
        column_widths: vec![0.25],
        default_column_width: 9,
        ..config()
    });
    assert_eq!(world.config().default_column_width, 0);
    open(&mut world, 1);
    let _ = world.arrange();
}

#[test]
fn set_config_repairs_an_emptied_width_list_before_clamping() {
    // `validated()` replaces an empty list with the defaults first, so the
    // clamp below always has a non-empty list -- `len - 1` cannot underflow.
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
        column_widths: vec![0.0, f64::NAN],
        ..config()
    });
    assert_eq!(
        world.config().column_widths,
        Config::default().column_widths
    );
    let _ = world.arrange();
}
