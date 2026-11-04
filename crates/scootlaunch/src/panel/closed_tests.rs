//! The layer surface's `closed`: the compositor took the panel away (an
//! outside click, an unplugged output) and the client exits at once.

use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::Event;

use crate::dmenu::DmenuUi;
use crate::layout::Layout;
use crate::look::Look;
use crate::panel::{App, Exit, LaunchCtx, Model};
use crate::stage::Effects;

fn app() -> App {
    let look = Look::default();
    let layout = Layout::dmenu(look.font_size_px);
    App::new(
        None,
        Model::Dmenu(DmenuUi::open(vec![String::from("a")], 6, false, false)),
        300,
        layout.height(6),
        layout,
        6,
        LaunchCtx {
            state_home: None,
            home: None,
            display_set: false,
        },
        None,
        "",
        String::new(),
        Vec::new(),
        Effects::off(),
    )
}

/// `closed` alone, with no keyboard leave before it (the compositor may
/// send it first, or the seat may be gone), ends the panel: it is not
/// re-created, as a bar's surface would be.
#[test]
fn closed_alone_exits_without_a_keyboard_leave() {
    let mut app = app();
    assert!(!app.seat.has_focus(), "no keyboard focus was ever held");
    app.on_layer_event(true, Event::Closed);
    assert!(matches!(app.exit, Some(Exit::Closed)));
}

/// A `closed` from a role that was already destroyed is stale and ignored.
#[test]
fn a_stale_closed_is_ignored() {
    let mut app = app();
    app.on_layer_event(false, Event::Closed);
    assert!(app.exit.is_none());
}
