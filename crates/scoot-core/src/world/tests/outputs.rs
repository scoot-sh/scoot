use super::*;
use crate::{Action, Vertical, Workspaces};

const SECOND: Rect = Rect::new(1000, 0, 800, 600);

fn add_output(world: &mut World, id: u64, area: Rect) {
    world.handle_event(Event::OutputAdded {
        id: OutputId(id),
        area,
    });
}

fn open_on(world: &mut World, id: u64, output: u64, focus: bool) {
    world.handle_event(Event::WindowOpened {
        id: WindowId(id),
        info: WindowInfo::default(),
        output: Some(OutputId(output)),
        focus,
    });
}

#[test]
fn windows_wait_for_an_output() {
    let mut world = World::new(config());
    open(&mut world, 1);
    assert!(world.arrange().placements.is_empty());
    add_output(&mut world, 7, SCREEN);
    let placed = placement(&world, 1);
    assert_eq!(placed.output, OutputId(7));
    assert!(placed.visible);
}

#[test]
fn windows_open_on_the_output_they_ask_for() {
    let mut world = world();
    add_output(&mut world, 2, SECOND);
    open_on(&mut world, 1, 2, true);
    let placed = placement(&world, 1);
    assert_eq!(placed.output, OutputId(2));
    assert_eq!(placed.rect.x, 1010);
    assert_eq!(world.focused_output(), Some(OutputId(2)));
}

#[test]
fn removing_an_unfocused_output_moves_its_workspaces_and_keeps_focus() {
    let mut world = world();
    add_output(&mut world, 2, SECOND);
    open(&mut world, 1);
    open_on(&mut world, 2, 2, false);
    open_on(&mut world, 3, 2, false);

    world.handle_event(Event::OutputRemoved { id: OutputId(2) });
    assert_eq!(focused(&world), Some(1));
    assert!(placement(&world, 1).visible);
    assert_eq!(placement(&world, 2).output, OutputId(1));
    assert!(!placement(&world, 2).visible);

    world.handle_action(Action::FocusWorkspace(Vertical::Down));
    assert!(placement(&world, 2).visible);
    assert!(placement(&world, 3).visible);
}

#[test]
fn removing_the_focused_output_moves_focus_to_what_remains() {
    let mut world = world();
    add_output(&mut world, 2, SECOND);
    open(&mut world, 1);
    open_on(&mut world, 2, 2, true);
    assert_eq!(world.focused_output(), Some(OutputId(2)));

    world.handle_event(Event::OutputRemoved { id: OutputId(2) });
    assert_eq!(world.focused_output(), Some(OutputId(1)));
    assert_eq!(focused(&world), Some(1));
    assert_eq!(placement(&world, 2).output, OutputId(1));
}

#[test]
fn removing_the_last_output_parks_windows_until_another_appears() {
    let mut world = world();
    open(&mut world, 1);
    world.handle_event(Event::OutputRemoved { id: OutputId(1) });
    assert!(world.arrange().placements.is_empty());
    add_output(&mut world, 2, SCREEN);
    assert_eq!(placement(&world, 1).output, OutputId(2));
}

#[test]
fn a_resized_output_rescrolls_to_keep_focus_in_view() {
    let mut world = world();
    for id in 1..=3 {
        open(&mut world, id);
    }
    world.handle_event(Event::OutputChanged {
        id: OutputId(1),
        area: Rect::new(0, 0, 1600, 600),
    });
    let placed = placement(&world, 3);
    assert!(placed.visible);
    assert!(placed.rect.right() <= 1590, "{placed:?}");
}

#[test]
fn the_arrangement_reports_the_focused_output() {
    let world = world();
    assert_eq!(world.arrange().focused_output, Some(OutputId(1)));
}

// -- reserved edges (layer-shell exclusive zones) -------------------------

fn reserve(world: &mut World, id: u64, area: Rect) {
    world.handle_event(Event::OutputUsableAreaChanged {
        id: OutputId(id),
        area,
    });
}

/// The headline case: a 30px bar across the top of a 1000x600 screen. The
/// window keeps the gap it always had, measured from the bar instead of from
/// the screen edge, and loses exactly the bar's height.
#[test]
fn a_reserved_edge_shrinks_where_windows_go() {
    let mut world = world();
    open(&mut world, 1);
    // A half-width column in a 980x580 usable area (see `SCREEN`).
    let before = placement(&world, 1).rect;
    assert_eq!(before, Rect::new(10, 10, 485, 580));

    reserve(&mut world, 1, Rect::new(0, 30, 1000, 570));
    let after = placement(&world, 1).rect;
    assert_eq!(after, Rect::new(10, 40, 485, 550));
    // The output itself is unchanged -- only what windows may use is.
    assert_eq!(world.outputs(), vec![(OutputId(1), SCREEN)]);
    assert_eq!(world.usable_areas(), vec![Rect::new(0, 30, 1000, 570)]);
}

/// Reservations on both axes at once (a top bar and a left dock), which is
/// where using `area`'s origin instead of `usable`'s would show up.
#[test]
fn reserved_edges_on_two_sides_move_the_origin_and_shrink_both_axes() {
    let mut world = world();
    open(&mut world, 1);
    reserve(&mut world, 1, Rect::new(40, 30, 960, 570));
    // 940x550 usable from (50, 40); half of it, gaps included, is 465.
    assert_eq!(placement(&world, 1).rect, Rect::new(50, 40, 465, 550));
}

/// A zone bigger than the output (a client asking for more than exists, or a
/// stale one from a larger mode) can never hand out space the screen doesn't
/// have.
#[test]
fn an_oversized_usable_area_is_clamped_to_the_output() {
    let mut world = world();
    open(&mut world, 1);
    reserve(&mut world, 1, Rect::new(-500, -500, 100_000, 100_000));
    assert_eq!(world.usable_areas(), vec![SCREEN]);
    assert_eq!(placement(&world, 1).rect, Rect::new(10, 10, 485, 580));
}

/// A reservation that swallows the whole output must not panic or produce a
/// zero/negative window: every placement still gets at least 1x1, the same
/// floor `layout::distribute` guarantees everywhere else.
#[test]
fn a_zone_that_leaves_nothing_still_places_windows() {
    let mut world = world();
    open(&mut world, 1);
    open(&mut world, 2);
    reserve(&mut world, 1, Rect::new(0, 0, 0, 0));
    for id in 1..=2 {
        let rect = placement(&world, id).rect;
        assert!(rect.w >= 1 && rect.h >= 1, "{rect:?}");
    }
}

/// An i32-extreme zone reaches `Rect::intersection`'s `i64` math, `inset`'s
/// `2 * gap`, and from there the layout arithmetic -- a debug build panics on
/// any overflow along the way.
#[test]
fn an_extreme_zone_does_not_overflow_the_layout() {
    let mut world = world();
    open(&mut world, 1);
    for zone in [
        Rect::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX),
        Rect::new(i32::MAX, i32::MAX, i32::MAX, i32::MAX),
        Rect::new(0, 0, i32::MAX, i32::MAX),
        Rect::new(-1, -1, 1, 1),
    ] {
        reserve(&mut world, 1, zone);
        let rect = placement(&world, 1).rect;
        assert!(rect.w >= 1 && rect.h >= 1, "{zone:?} gave {rect:?}");
        // Whatever came in, what came out is inside the screen's own bounds.
        let usable = world.usable_areas()[0];
        assert_eq!(usable, usable.intersection(SCREEN), "{zone:?}");
    }
}

/// A resize re-clamps the reservation instead of forgetting it: a bar is
/// still drawing over that strip until it says otherwise.
#[test]
fn resizing_the_output_keeps_a_reservation_that_still_fits() {
    let mut world = world();
    open(&mut world, 1);
    reserve(&mut world, 1, Rect::new(0, 30, 1000, 570));

    world.handle_event(Event::OutputChanged {
        id: OutputId(1),
        area: Rect::new(0, 0, 800, 400),
    });
    assert_eq!(world.usable_areas(), vec![Rect::new(0, 30, 800, 370)]);
    assert_eq!(placement(&world, 1).rect, Rect::new(10, 40, 385, 350));
}

/// ...and a reservation that no longer fits at all shrinks to nothing rather
/// than describing space off the end of the new mode.
#[test]
fn resizing_past_a_reservation_shrinks_it_rather_than_keeping_it() {
    let mut world = world();
    reserve(&mut world, 1, Rect::new(0, 500, 1000, 100));
    world.handle_event(Event::OutputChanged {
        id: OutputId(1),
        area: Rect::new(0, 0, 1000, 200),
    });
    let usable = world.usable_areas()[0];
    assert_eq!(usable, usable.intersection(Rect::new(0, 0, 1000, 200)));
    assert_eq!(usable.size(), crate::Size::new(1000, 0));
}

/// A reservation for an output that doesn't exist is ignored, not applied to
/// whichever one happens to be focused.
#[test]
fn a_zone_for_an_unknown_output_is_ignored() {
    let mut world = world();
    open(&mut world, 1);
    reserve(&mut world, 99, Rect::new(0, 300, 1000, 300));
    assert_eq!(world.usable_areas(), vec![SCREEN]);
    assert_eq!(placement(&world, 1).rect, Rect::new(10, 10, 485, 580));
}

/// Each output keeps its own reservation; a bar on one doesn't shrink the
/// other.
#[test]
fn reservations_are_per_output() {
    let mut world = world();
    add_output(&mut world, 2, SECOND);
    open(&mut world, 1);
    open_on(&mut world, 2, 2, false);

    reserve(&mut world, 1, Rect::new(0, 30, 1000, 570));
    assert_eq!(placement(&world, 1).rect, Rect::new(10, 40, 485, 550));
    assert_eq!(placement(&world, 2).rect, Rect::new(1010, 10, 385, 580));
}

/// An absurd configured proportion saturates `column_width`'s float-to-int
/// cast to `i32::MAX`, and two such columns saturate the strip: arranging
/// them must saturate `usable.x + start` rather than panic (debug) or wrap
/// (release). Operator-supplied, like the flags -- a config file can spell
/// any finite positive proportion -- so this is the same LOW family, found
/// by the ticket's sibling audit rather than the flags themselves.
#[test]
fn absurd_column_proportions_saturate_instead_of_overflowing() {
    let mut world = World::new(Config {
        gap: 10,
        column_widths: vec![1e18],
        default_column_width: 0,
    });
    add_output(&mut world, 1, SCREEN);
    open(&mut world, 1);
    open(&mut world, 2);
    let arrangement = world.arrange();
    assert_eq!(arrangement.placements.len(), 2);
    for placed in &arrangement.placements {
        assert!(placed.rect.w >= 1 && placed.rect.h >= 1, "{placed:?}");
    }
}

/// A newly added output starts with everything usable, so a platform that
/// reserves nothing behaves exactly as it did before reservations existed.
#[test]
fn a_new_output_is_usable_edge_to_edge() {
    let world = world();
    assert_eq!(
        world.usable_areas(),
        world
            .outputs()
            .iter()
            .map(|(_, area)| *area)
            .collect::<Vec<_>>()
    );
}

// -- cross-output moves + output focus (milestone 19, phase F) -------------

/// Two side-by-side outputs with one window each, focused on the first.
fn two_output_world() -> World {
    let mut world = world();
    add_output(&mut world, 2, SECOND);
    open(&mut world, 1);
    open_on(&mut world, 2, 2, false);
    world.handle_action(Action::FocusWindowId(WindowId(1)));
    assert_eq!(focused(&world), Some(1));
    assert_eq!(world.focused_output(), Some(OutputId(1)));
    world
}

/// The column preset holding `id`, read white-box: the move must carry the
/// window's width choice across, not reset it to the default.
fn preset_of(world: &World, id: u64) -> usize {
    world
        .outputs
        .iter()
        .flat_map(|output| &output.workspaces)
        .flat_map(|ws| &ws.columns)
        .find(|column| column.windows.contains(&WindowId(id)))
        .map(|column| column.preset)
        .expect("window is in the tree")
}

#[test]
fn moving_the_focused_window_to_another_output_carries_it_and_follows() {
    let mut world = two_output_world();
    // A second window on the source output, so the fallback below has a
    // neighbour to land on rather than an empty output.
    open(&mut world, 3);
    world.handle_action(Action::FocusWindowId(WindowId(1)));
    world.handle_action(Action::MoveFocusedWindowToOutput(OutputId(2)));
    // The window is on the target's active workspace, visible, focused --
    // and focus followed it there.
    let placed = placement(&world, 1);
    assert_eq!(placed.output, OutputId(2));
    assert!(placed.visible);
    assert_eq!(focused(&world), Some(1));
    assert_eq!(world.focused_output(), Some(OutputId(2)));
    assert_eq!(world.arrange().focused_output, Some(OutputId(2)));
    // The window that stayed put is untouched...
    let stayed = placement(&world, 2);
    assert_eq!(stayed.output, OutputId(2));
    // ...and the source output keeps working: its remaining window is
    // focused there (the neighbour `take` leaves behind), so focusing back
    // lands somewhere sane.
    world.handle_action(Action::FocusOutput(OutputId(1)));
    assert_eq!(focused(&world), Some(3));
}

#[test]
fn the_move_carries_the_column_preset() {
    let mut world = two_output_world();
    world.handle_action(Action::CycleColumnWidth);
    assert_eq!(preset_of(&world, 1), 1);
    world.handle_action(Action::MoveFocusedWindowToOutput(OutputId(2)));
    assert_eq!(preset_of(&world, 1), 1);
}

#[test]
fn moving_to_the_window_s_current_output_changes_nothing() {
    let mut world = two_output_world();
    let before = world.arrange();
    world.handle_action(Action::MoveFocusedWindowToOutput(OutputId(1)));
    assert_eq!(world.arrange(), before);
    assert_eq!(focused(&world), Some(1));
}

#[test]
fn moving_to_an_unknown_output_leaves_the_window_where_it_is() {
    let mut world = two_output_world();
    let before = world.arrange();
    // A stale id off a wire, and the extremes a client can spell --
    // mirroring the out-of-range workspace-index case, including the
    // promise the window is never lost: it stays where it was.
    for id in [99, u64::MAX / 2, u64::MAX] {
        world.handle_action(Action::MoveFocusedWindowToOutput(OutputId(id)));
        assert_eq!(world.arrange(), before, "output {id}");
        assert_eq!(focused(&world), Some(1), "output {id}");
        assert_eq!(world.focused_output(), Some(OutputId(1)), "output {id}");
    }
}

#[test]
fn moving_with_no_focused_window_does_nothing() {
    // Two empty outputs: no window focused, so nothing to carry -- to the
    // active output (the already-there shape) or anywhere else.
    let mut world = world();
    add_output(&mut world, 2, SECOND);
    world.handle_action(Action::MoveFocusedWindowToOutput(OutputId(1)));
    world.handle_action(Action::MoveFocusedWindowToOutput(OutputId(2)));
    world.handle_action(Action::MoveFocusedWindowToOutput(OutputId(99)));
    let empty = Some(Workspaces {
        count: 1,
        active: 0,
    });
    assert_eq!(world.workspaces(OutputId(1)), empty);
    assert_eq!(world.workspaces(OutputId(2)), empty);
    assert_eq!(world.focused_window(), None);
}

#[test]
fn moving_the_only_window_empties_but_keeps_the_source_output() {
    let mut world = world();
    add_output(&mut world, 2, SECOND);
    open(&mut world, 1);
    world.handle_action(Action::MoveFocusedWindowToOutput(OutputId(2)));
    assert_eq!(placement(&world, 1).output, OutputId(2));
    assert_eq!(focused(&world), Some(1));
    // The source output is left empty -- valid, still exactly one empty
    // workspace, still focusable back to (focusing nothing, not garbage).
    assert_eq!(
        world.workspaces(OutputId(1)),
        Some(Workspaces {
            count: 1,
            active: 0
        })
    );
    world.handle_action(Action::FocusOutput(OutputId(1)));
    assert_eq!(world.focused_output(), Some(OutputId(1)));
    assert_eq!(focused(&world), None);
}

#[test]
fn focus_output_moves_focus_to_that_output() {
    let mut world = two_output_world();
    world.handle_action(Action::FocusOutput(OutputId(2)));
    assert_eq!(world.focused_output(), Some(OutputId(2)));
    assert_eq!(focused(&world), Some(2));
    assert!(placement(&world, 2).visible);
    // ...and back: focusing is symmetric, not a one-way trip.
    world.handle_action(Action::FocusOutput(OutputId(1)));
    assert_eq!(focused(&world), Some(1));
}

#[test]
fn focus_output_on_an_empty_output_focuses_nothing() {
    // Defined behavior, pinned: an output with no windows has no focused
    // window -- the same `None` an emptied focused output already reads as
    // everywhere else -- and the output itself is still the focused one, so
    // a subsequent workspace action resolves against it, not against stale
    // state.
    let mut world = world();
    add_output(&mut world, 2, SECOND);
    open(&mut world, 1);
    world.handle_action(Action::FocusOutput(OutputId(2)));
    assert_eq!(world.focused_output(), Some(OutputId(2)));
    assert_eq!(focused(&world), None);
    assert_eq!(world.arrange().focused, None);
    assert_eq!(world.arrange().focused_output, Some(OutputId(2)));
}

#[test]
fn focus_output_on_an_unknown_output_is_ignored() {
    let mut world = two_output_world();
    for id in [99, u64::MAX] {
        world.handle_action(Action::FocusOutput(OutputId(id)));
        assert_eq!(world.focused_output(), Some(OutputId(1)), "output {id}");
        assert_eq!(focused(&world), Some(1), "output {id}");
    }
}

#[test]
fn rapid_move_focus_move_sequences_stay_consistent() {
    // The wedge/shape this guards: a move that loses a window (in neither
    // tree) or strands focus on a nonexistent output. After every step each
    // window is placed exactly once and focus names a live window or none.
    let mut world = two_output_world();
    let steps = [
        Action::MoveFocusedWindowToOutput(OutputId(2)),
        Action::FocusOutput(OutputId(1)),
        Action::MoveFocusedWindowToOutput(OutputId(2)),
        Action::FocusOutput(OutputId(2)),
        Action::MoveFocusedWindowToOutput(OutputId(1)),
        Action::FocusWindowId(WindowId(2)),
        Action::MoveFocusedWindowToOutput(OutputId(1)),
        Action::FocusOutput(OutputId(99)),
        Action::MoveFocusedWindowToOutput(OutputId(99)),
    ];
    for step in steps {
        world.handle_action(step.clone());
        let arrangement = world.arrange();
        let mut placed: Vec<u64> = arrangement.placements.iter().map(|p| p.id.0).collect();
        placed.sort();
        assert_eq!(placed, vec![1, 2], "a move lost a window: {step:?}");
        assert!(world.focused_output().is_some(), "focus stranded: {step:?}");
        if let Some(focus) = focused(&world) {
            assert!(
                placed.contains(&focus),
                "focus names a window that is nowhere: {step:?}"
            );
        }
    }
    // Both windows end on output 1, window 2 focused there.
    assert_eq!(placement(&world, 1).output, OutputId(1));
    assert_eq!(placement(&world, 2).output, OutputId(1));
    assert_eq!(focused(&world), Some(2));
}
