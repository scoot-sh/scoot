//! The event side of windows mode: the manager announces windows, each
//! handle reports its title, app id and whether it closed. Everything
//! else the protocol sends (state, outputs, parent) is ignored: the
//! list shows what a window is called and which app it belongs to.

use wayland_client::protocol::wl_callback::{self, WlCallback};
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::{Connection, Dispatch, QueueHandle, delegate_noop, event_created_child};
use wayland_protocols_wlr::foreign_toplevel::v1::client::zwlr_foreign_toplevel_handle_v1::{
    self, ZwlrForeignToplevelHandleV1,
};
use wayland_protocols_wlr::foreign_toplevel::v1::client::zwlr_foreign_toplevel_manager_v1::{
    self, ZwlrForeignToplevelManagerV1,
};

use super::{Item, MAX_WINDOWS, State};

delegate_noop!(State: ignore WlSeat);

impl Dispatch<WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        _: &WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "zwlr_foreign_toplevel_manager_v1" => state.manager_global = Some((name, version)),
                // The first seat is the one to activate on.
                "wl_seat" if state.seat_global.is_none() => {
                    state.seat_global = Some((name, version));
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<WlCallback, ()> for State {
    fn event(
        state: &mut Self,
        _: &WlCallback,
        event: wl_callback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { .. } = event {
            state.synced = true;
        }
    }
}

impl Dispatch<ZwlrForeignToplevelManagerV1, ()> for State {
    event_created_child!(State, ZwlrForeignToplevelManagerV1, [
        zwlr_foreign_toplevel_manager_v1::EVT_TOPLEVEL_OPCODE => (ZwlrForeignToplevelHandleV1, ()),
    ]);

    fn event(
        state: &mut Self,
        _: &ZwlrForeignToplevelManagerV1,
        event: zwlr_foreign_toplevel_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_foreign_toplevel_manager_v1::Event::Toplevel { toplevel } => {
                if state.items.len() >= MAX_WINDOWS {
                    // Destroying a handle is legal at any time; its later
                    // events find no item and are ignored.
                    toplevel.destroy();
                    return;
                }
                state.items.push(Item {
                    handle: toplevel,
                    title: String::new(),
                    app_id: String::new(),
                    closed: false,
                });
            }
            // No more windows will be announced, and a request on a
            // handle past this is a protocol error.
            zwlr_foreign_toplevel_manager_v1::Event::Finished => state.finished = true,
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
        let Some(item) = state.items.iter_mut().find(|item| item.handle == *handle) else {
            return;
        };
        match event {
            zwlr_foreign_toplevel_handle_v1::Event::Title { title } => item.title = title,
            zwlr_foreign_toplevel_handle_v1::Event::AppId { app_id } => item.app_id = app_id,
            zwlr_foreign_toplevel_handle_v1::Event::Closed => item.closed = true,
            _ => {}
        }
    }
}
