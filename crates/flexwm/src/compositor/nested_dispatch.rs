//! `wayland_client::Dispatch` impls for every host-side protocol object
//! `nested.rs` creates. Split out of `nested.rs` the same way `handlers.rs`
//! holds this compositor's own (server-side) protocol handlers separately
//! from `state.rs` -- this file is purely "what happens when the host sends
//! us an event", nothing else.

use flexwm_ipc::PointerButton;
use smithay::backend::input::KeyState;
use smithay::input::keyboard::Keycode;
use wayland_client::globals::GlobalListContents;
use wayland_client::protocol::wl_buffer::WlBuffer as HostBuffer;
use wayland_client::protocol::wl_compositor::WlCompositor as HostCompositor;
use wayland_client::protocol::wl_keyboard::{self, WlKeyboard as HostKeyboard};
use wayland_client::protocol::wl_pointer::{self, WlPointer as HostPointer};
use wayland_client::protocol::wl_registry::WlRegistry as HostRegistry;
use wayland_client::protocol::wl_seat::{self, WlSeat as HostSeat};
use wayland_client::protocol::wl_shm::WlShm as HostShm;
use wayland_client::protocol::wl_shm_pool::WlShmPool as HostShmPool;
use wayland_client::protocol::wl_surface::WlSurface as HostSurface;
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols::xdg::shell::client::xdg_surface::{self, XdgSurface as HostXdgSurface};
use wayland_protocols::xdg::shell::client::xdg_toplevel::{self, XdgToplevel as HostToplevel};
use wayland_protocols::xdg::shell::client::xdg_wm_base::{self, XdgWmBase as HostWmBase};

use super::State;
use super::nested::Host;

// Objects whose events this compositor has no use for at all: the registry
// bootstrap (handled by registry_queue_init's GlobalListContents), and two
// objects (compositor, shm pool) the protocol defines with no events.
wayland_client::delegate_noop!(State: ignore HostCompositor);
wayland_client::delegate_noop!(State: ignore HostShmPool);
wayland_client::delegate_noop!(State: ignore HostShm);
// wl_surface::Enter/Leave (which output the surface is on) matter for
// per-output scale, which is out of v1 scope -- see nested.rs's module doc.
wayland_client::delegate_noop!(State: ignore HostSurface);

impl Dispatch<HostRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &HostRegistry,
        _: wayland_client::protocol::wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // Globals arriving/leaving after startup (a host compositor
        // restarting a service) aren't handled -- v1 connects once, to
        // whatever globals exist at startup, matching this project's stated
        // scope boundary of not chasing host-side reconfiguration.
    }
}

impl Dispatch<HostBuffer, ()> for State {
    fn event(
        state: &mut Self,
        buffer: &HostBuffer,
        event: wayland_client::protocol::wl_buffer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wayland_client::protocol::wl_buffer::Event::Release = event
            && let Some(host) = &mut state.host
        {
            host.buffers_mut().mark_released(buffer);
        }
    }
}

impl Dispatch<HostWmBase, ()> for State {
    fn event(
        _: &mut Self,
        proxy: &HostWmBase,
        event: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // The host pings periodically to check flexwm is still alive; not
        // answering gets this window (and, per some compositors, the whole
        // connection) killed as unresponsive.
        if let xdg_wm_base::Event::Ping { serial } = event {
            proxy.pong(serial);
        }
    }
}

impl Dispatch<HostXdgSurface, ()> for State {
    fn event(
        state: &mut Self,
        proxy: &HostXdgSurface,
        event: xdg_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let xdg_surface::Event::Configure { serial } = event else {
            return;
        };
        // ack_configure is mandatory on every configure, first or not, per
        // xdg-shell -- do this before the early-return below, or a host that
        // sends a second configure before the first render lands would never
        // get it acked and would stall the surface.
        proxy.ack_configure(serial);
        let Some(host) = &mut state.host else {
            return;
        };
        if host.is_configured() {
            // Scope boundary (v1): only the first configure is acted on.
            // Later ones (a host-side resize) are acked above and otherwise
            // ignored -- the window keeps its original size instead of
            // matching the host's new one.
            return;
        }
        let (width, height) = host.take_pending_size();
        Host::apply_size(state, width, height);
        if let Some(host) = &mut state.host {
            host.mark_configured();
        }
    }
}

impl Dispatch<HostToplevel, ()> for State {
    fn event(
        state: &mut Self,
        _: &HostToplevel,
        event: xdg_toplevel::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            xdg_toplevel::Event::Configure { width, height, .. } => {
                // 0x0 means "you choose"; keep whatever's already pending
                // (the size flexwm was started with) rather than resizing to
                // nothing.
                if width > 0
                    && height > 0
                    && let Some(host) = &mut state.host
                {
                    host.set_pending_size(width, height);
                }
            }
            xdg_toplevel::Event::Close => {
                tracing::info!("host closed flexwm's window; exiting");
                state.loop_signal.stop();
            }
            _ => {}
        }
    }
}

impl Dispatch<HostSeat, ()> for State {
    fn event(
        state: &mut Self,
        seat: &HostSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let wl_seat::Event::Capabilities { capabilities } = event else {
            return;
        };
        let capabilities = match capabilities {
            wayland_client::WEnum::Value(c) => c,
            wayland_client::WEnum::Unknown(_) => return,
        };
        let Some(host) = &mut state.host else {
            return;
        };
        let wants_keyboard = capabilities.contains(wl_seat::Capability::Keyboard);
        let wants_pointer = capabilities.contains(wl_seat::Capability::Pointer);
        if wants_keyboard && host.keyboard().is_none() {
            host.set_keyboard(seat.get_keyboard(qh, ()));
        }
        if wants_pointer && host.pointer().is_none() {
            host.set_pointer(seat.get_pointer(qh, ()));
        }
    }
}

impl Dispatch<HostKeyboard, ()> for State {
    fn event(
        state: &mut Self,
        _: &HostKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // Modifiers and the host's keymap are both out of v1 scope: modifier
        // state isn't tracked separately from whatever the raw key events
        // themselves carry (fine for plain typing; a host-side modifier
        // held before flexwm's window gained focus wouldn't be reflected),
        // and the keyboard this compositor sets up (state.rs) always uses
        // the default US layout rather than adopting the host's.
        if let wl_keyboard::Event::Key {
            key,
            state: key_state,
            ..
        } = event
        {
            let pressed = matches!(
                key_state,
                wayland_client::WEnum::Value(wl_keyboard::KeyState::Pressed)
            );
            let code = Keycode::new(key + 8); // evdev -> xkb keycode offset
            let smithay_state = if pressed {
                KeyState::Pressed
            } else {
                KeyState::Released
            };
            state.key(code, smithay_state);
        }
    }
}

impl Dispatch<HostPointer, ()> for State {
    fn event(
        state: &mut Self,
        _: &HostPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            // Entering carries the initial position; treat it as a motion so
            // focus-follows-pointer style clicks right after entering still
            // know where the pointer is.
            wl_pointer::Event::Enter {
                surface_x,
                surface_y,
                ..
            }
            | wl_pointer::Event::Motion {
                surface_x,
                surface_y,
                ..
            } => {
                state.pointer_move(surface_x, surface_y);
            }
            wl_pointer::Event::Button {
                button,
                state: button_state,
                ..
            } => {
                let Some(button) = linux_button(button) else {
                    return;
                };
                let pressed = matches!(
                    button_state,
                    wayland_client::WEnum::Value(wl_pointer::ButtonState::Pressed)
                );
                state.pointer_button(button, pressed);
            }
            wl_pointer::Event::Axis { axis, value, .. } => {
                let (dx, dy) = match axis {
                    wayland_client::WEnum::Value(wl_pointer::Axis::HorizontalScroll) => {
                        (value, 0.0)
                    }
                    wayland_client::WEnum::Value(wl_pointer::Axis::VerticalScroll) => (0.0, value),
                    _ => return,
                };
                state.scroll(dx, dy);
            }
            // Each event above already ends its own pointer frame (input.rs
            // calls pointer.frame() per call, not batched), so there's
            // nothing to do with the host's own frame boundary -- but the
            // event is matched explicitly, not folded into a catch-all, so
            // it's clear this was considered rather than missed.
            wl_pointer::Event::Frame => {}
            _ => {}
        }
    }
}

/// Linux input event `BTN_*` codes, matching input.rs's own `code()` in
/// reverse.
fn linux_button(code: u32) -> Option<PointerButton> {
    match code {
        0x110 => Some(PointerButton::Left),
        0x111 => Some(PointerButton::Right),
        0x112 => Some(PointerButton::Middle),
        _ => None,
    }
}
