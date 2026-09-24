//! `wayland_client::Dispatch` impls for every host-side protocol object
//! `nested.rs` creates. Split out of `nested.rs` the same way `handlers.rs`
//! holds this compositor's own (server-side) protocol handlers separately
//! from `state.rs` -- this file is purely "what happens when the host sends
//! us an event", nothing else.

use scoot_ipc::PointerButton;
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

#[cfg(feature = "gpu-scanout")]
use wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_buffer_params_v1::{
    self, ZwpLinuxBufferParamsV1,
};
#[cfg(feature = "gpu-scanout")]
use wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1;

use super::State;
#[cfg(feature = "gpu-scanout")]
use super::nested::ParamsTag;
use super::nested::{ConfigureAction, Host, configure_action};

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
        // restarting a service) aren't handled -- scoot connects once, to
        // whatever globals exist at startup. A scope boundary about the
        // host's *globals* only: a host resizing scoot's window afterwards
        // is followed, through `xdg_surface::Configure` below.
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
        let wayland_client::protocol::wl_buffer::Event::Release = event else {
            return;
        };
        // Scoped so the mutable borrow of `state.host` ends before the
        // possible `state.request_render()` call below, which needs `state`
        // whole again.
        let should_retry = if let Some(host) = &mut state.host {
            host.mark_released(buffer);
            host.take_present_skipped()
        } else {
            false
        };
        if should_retry {
            state.request_render();
        }
    }
}

// The host's dma-buf global has no events a version-4 client is sent (the
// `format`/`modifier` pair is deprecated from 4 on); its feedback is read on
// a private queue at startup (`nested/gpu/feedback.rs`).
#[cfg(feature = "gpu-scanout")]
wayland_client::delegate_noop!(State: ignore ZwpLinuxDmabufV1);

/// The host's answer to a dma-buf `create` (`nested/gpu.rs`). Either way the
/// params object has done its job and is destroyed, as the protocol asks.
#[cfg(feature = "gpu-scanout")]
impl Dispatch<ZwpLinuxBufferParamsV1, ParamsTag> for State {
    fn event(
        state: &mut Self,
        params: &ZwpLinuxBufferParamsV1,
        event: zwp_linux_buffer_params_v1::Event,
        tag: &ParamsTag,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwp_linux_buffer_params_v1::Event::Created { buffer } => {
                params.destroy();
                Host::buffer_created(state, *tag, buffer);
            }
            zwp_linux_buffer_params_v1::Event::Failed => {
                params.destroy();
                Host::buffer_failed(state, *tag);
            }
            _ => {}
        }
    }

    // `created` carries a new `wl_buffer`; it gets the same `()` user data,
    // and so the same `release` handling, as every `wl_shm` buffer.
    wayland_client::event_created_child!(State, ZwpLinuxBufferParamsV1, [
        zwp_linux_buffer_params_v1::EVT_CREATED_OPCODE => (HostBuffer, ()),
    ]);
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
        // The host pings periodically to check scoot is still alive; not
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
        // ack_configure is mandatory on every configure, acted on or not, per
        // xdg-shell -- do this before any of the decisions below, or a host
        // that sends a second configure before the first render lands would
        // never get it acked and would stall the surface.
        proxy.ack_configure(serial);
        let Some(host) = &mut state.host else {
            return;
        };
        // Taken on every configure, not only the first: the proposal has been
        // reconciled against what scoot is at once it has been compared,
        // whichever way the comparison went.
        let (width, height) = host.take_pending_size();
        let action = configure_action(host.is_configured(), (width, height), host.size());
        // Which failure policy applies is the entry point, not a flag read in
        // here: a first configure that cannot be built is fatal, a resize
        // that cannot be built keeps the session at its old size. See each
        // function's doc for why they differ. A resize is queued for the
        // next render tick rather than applied here: a drag sends a
        // configure per pixel step, and rebuilding the pool and the render
        // target per step is what `Host::drain_pending_resize` exists to
        // avoid. The render request is what gets it drained.
        match action {
            ConfigureAction::FirstConfigure => Host::apply_first_configure(state, width, height),
            ConfigureAction::Resize => {
                if let Some(host) = &mut state.host {
                    host.queue_resize(width, height);
                }
                state.request_render();
            }
            ConfigureAction::Nothing => {}
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
                // Recorded, not acted on: the paired `xdg_surface::Configure`
                // above carries the serial to ack and is where the size is
                // applied -- xdg-shell delivers the two separately by design.
                // Which proposals are usable at all (0x0's "you choose", a
                // size out of range) is `set_pending_size`'s own decision.
                if let Some(host) = &mut state.host {
                    host.set_pending_size(width, height);
                }
            }
            xdg_toplevel::Event::Close => {
                tracing::info!("host closed scoot's window; exiting");
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
        tracing::debug!(wants_keyboard, wants_pointer, "host seat capabilities");
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
        // held before scoot's window gained focus wouldn't be reflected),
        // and the keyboard this compositor sets up (state.rs) always uses
        // the default US layout rather than adopting the host's (this
        // `Keymap` event, ignored below, is how the host would offer one).
        //
        // That last point matters for how real input was bug-bashed: tools
        // like `wtype` don't use the seat's regular keyboard at all -- they
        // create a `zwp_virtual_keyboard_v1` and upload their own ad hoc xkb
        // keymap, so the host relays a real `Keymap` event carrying keycodes
        // that mean whatever that tool made up, not evdev positions. Since
        // that event is ignored here, scoot interprets the raw keycodes
        // that follow against its own default US layout instead -- garbage
        // in, garbage out, by design, not a bug in the forwarding below. A
        // real physical keyboard never uploads a keymap, so this doesn't
        // affect it; it only affects synthetic input tools until host
        // keymap adoption is implemented (still out of scope).
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
