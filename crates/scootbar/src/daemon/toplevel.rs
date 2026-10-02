//! `wlr-foreign-toplevel-management-v1` on the daemon's side: the events
//! the compositor sends on the connection fd, turned into calls on the
//! window-title module's shared state (`crate::modules::window_title`).
//!
//! The module cannot see these itself: they arrive on the Wayland
//! connection's fd, which the loop owns and dispatches centrally, so this
//! glue translates proxies to plain calls here, and the module reports the
//! change through [`crate::modules::Module::on_dispatch`] before the next
//! draw. Nothing here allocates: proxies are compared and stored, titles
//! are copied into fixed bytes at event time.
//!
//! Ignored deliberately: `output_enter` for an output the bar never bound
//! (its name never arrives, so it never matches a view),
//! `set_maximized`/`set_minimized` and friends (the bar sends only
//! `activate` and `close`), and a `finished` manager's handles afterwards
//! (the shared state stops the module sending anything on them: a request
//! past `finished` is a protocol error, which would kill the bar;
//! destroying the handles stays legal).

use wayland_client::{Connection, Dispatch, QueueHandle, event_created_child};
use wayland_protocols_wlr::foreign_toplevel::v1::client::zwlr_foreign_toplevel_handle_v1::{
    self, ZwlrForeignToplevelHandleV1,
};
use wayland_protocols_wlr::foreign_toplevel::v1::client::zwlr_foreign_toplevel_manager_v1::{
    self, ZwlrForeignToplevelManagerV1,
};

use super::wayland::State;

/// The `state` array's entries, as the protocol numbers them
/// (`maximized` 0, `minimized` 1, `activated` 2, `fullscreen` 3): the
/// array is raw `u32`s in native byte order.
const ACTIVATED: u32 = 2;
const FULLSCREEN: u32 = 3;

/// Whether the `state` array carries `want`.
fn has_state(state: &[u8], want: u32) -> bool {
    state
        .chunks_exact(4)
        .any(|four| u32::from_ne_bytes(four.try_into().unwrap_or([0; 4])) == want)
}

impl Dispatch<ZwlrForeignToplevelManagerV1, ()> for State {
    event_created_child!(State, ZwlrForeignToplevelManagerV1, [
        zwlr_foreign_toplevel_manager_v1::EVT_TOPLEVEL_OPCODE => (ZwlrForeignToplevelHandleV1, ()),
    ]);

    fn event(
        state: &mut Self,
        manager: &ZwlrForeignToplevelManagerV1,
        event: zwlr_foreign_toplevel_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let mut shared = state.title.0.borrow_mut();
        // A manager let go (`binds`: no window-title module placed), or
        // replaced by a later bind, may still have events on the wire. They
        // are not this run's state: a handle it announces is destroyed
        // unseen, and its `finished` must not mark the new manager dead.
        if shared.manager() != Some(manager) {
            if let zwlr_foreign_toplevel_manager_v1::Event::Toplevel { toplevel } = event {
                toplevel.destroy();
            }
            return;
        }
        match event {
            zwlr_foreign_toplevel_manager_v1::Event::Toplevel { toplevel } => {
                shared.on_toplevel(toplevel);
            }
            // No more windows will be announced; the ones known keep
            // reporting until they close.
            zwlr_foreign_toplevel_manager_v1::Event::Finished => shared.on_finished(),
            _ => {}
        }
    }
}

impl Dispatch<ZwlrForeignToplevelHandleV1, ()> for State {
    fn event(
        state: &mut Self,
        handle: &ZwlrForeignToplevelHandleV1,
        event: zwlr_foreign_toplevel_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let mut shared = state.title.0.borrow_mut();
        match event {
            zwlr_foreign_toplevel_handle_v1::Event::Title { title } => {
                shared.on_title(handle, &title);
            }
            zwlr_foreign_toplevel_handle_v1::Event::AppId { app_id } => {
                shared.on_app_id(handle, &app_id);
            }
            zwlr_foreign_toplevel_handle_v1::Event::OutputEnter { output } => {
                shared.on_output_enter(handle, output);
            }
            zwlr_foreign_toplevel_handle_v1::Event::OutputLeave { output } => {
                shared.on_output_leave(handle, &output);
            }
            zwlr_foreign_toplevel_handle_v1::Event::State { state } => {
                shared.on_state(
                    handle,
                    has_state(&state, ACTIVATED),
                    has_state(&state, FULLSCREEN),
                );
            }
            // The batch is whole; the state applied at once above, so
            // there is nothing more to commit.
            zwlr_foreign_toplevel_handle_v1::Event::Done => {}
            zwlr_foreign_toplevel_handle_v1::Event::Closed => {
                shared.on_closed(handle);
            }
            _ => {}
        }
    }
}
