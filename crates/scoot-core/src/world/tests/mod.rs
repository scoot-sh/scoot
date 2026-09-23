mod bench;
mod columns;
mod frames;
mod fullscreen;
mod invariants;
mod outputs;
mod reload;
mod workspaces;

use crate::{Config, Event, OutputId, Placement, Rect, WindowId, WindowInfo, World};

// 1000x600 with a 10px gap leaves a 980x580 usable area at (10, 10); a
// half-width column is 485px.
const SCREEN: Rect = Rect::new(0, 0, 1000, 600);

fn config() -> Config {
    Config {
        gap: 10,
        column_widths: vec![0.5, 1.0],
        default_column_width: 0,
    }
}

fn world() -> World {
    let mut world = World::new(config());
    world.handle_event(Event::OutputAdded {
        id: OutputId(1),
        area: SCREEN,
    });
    world
}

fn open(world: &mut World, id: u64) {
    open_with(world, id, WindowInfo::default());
}

fn open_with(world: &mut World, id: u64, info: WindowInfo) {
    world.handle_event(Event::WindowOpened {
        id: WindowId(id),
        info,
        output: None,
        focus: true,
    });
}

fn placement(world: &World, id: u64) -> Placement {
    *world.arrange().get(WindowId(id)).expect("window is placed")
}

fn focused(world: &World) -> Option<u64> {
    world.focused_window().map(|id| id.0)
}

fn workspace_count(world: &World) -> usize {
    world.outputs[world.focused_output].workspaces.len()
}
