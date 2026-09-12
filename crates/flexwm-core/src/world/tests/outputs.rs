use super::*;
use crate::{Action, Vertical};

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
