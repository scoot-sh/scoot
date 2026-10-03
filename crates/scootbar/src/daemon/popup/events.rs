//! The popup's events: the protocol objects' (`xdg_wm_base`'s ping, the
//! surface role's configure and `popup_done`, the buffers' release), and the
//! seat's, routed here while the popup is the pointer's or the keyboard's
//! focus.
//!
//! Each popup object carries its [`PopupId`] as user data, and every event
//! is checked against the open popup's: an event of one already destroyed
//! (a `popup_done` or a release in flight when it was closed) names nobody
//! and is dropped, whatever it was.

use wayland_client::protocol::wl_buffer::{self, WlBuffer};
use wayland_client::protocol::wl_keyboard::{self, KeyState, WlKeyboard};
use wayland_client::protocol::wl_pointer;
use wayland_client::protocol::wl_surface::{self, WlSurface};
use wayland_client::{Connection, Dispatch, QueueHandle, WEnum, delegate_noop};
use wayland_protocols::xdg::shell::client::xdg_popup::{self, XdgPopup};
use wayland_protocols::xdg::shell::client::xdg_positioner::XdgPositioner;
use wayland_protocols::xdg::shell::client::xdg_surface::{self, XdgSurface};
use wayland_protocols::xdg::shell::client::xdg_wm_base::{self, XdgWmBase};

use crate::daemon::wayland::State;
use crate::density::Scale;
use crate::outputs::Size;

/// Which popup an object belongs to: a counter, never reused within a run,
/// so a stale event cannot be taken for the open popup's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PopupId(pub(super) u64);

/// `KEY_ESC` (`linux/input-event-codes.h`): matched as the code it is, so
/// no keymap is read and the daemon needs no xkb.
const KEY_ESC: u32 = 1;
/// `BTN_LEFT`: the only button a popup answers.
const BTN_LEFT: u32 = 0x110;

/// A logical, surface-local coordinate in device pixels, signed (a drag
/// goes past the popup's edges); a non-finite one is 0.
pub(super) fn device(coordinate: f64, scale: Scale) -> i64 {
    if !coordinate.is_finite() {
        return 0;
    }
    (coordinate * scale.factor()).clamp(-1.0e9, 1.0e9) as i64
}

impl State {
    /// Closes the popup if it is `id`'s.
    fn close_popup_if(&mut self, id: PopupId) {
        if self.popup.open.as_ref().is_some_and(|open| open.id == id) {
            self.popup.close();
        }
    }

    /// A `wl_pointer` event while a popup is open: whether it was the
    /// popup's (and so is not the bar's). `enter` on the popup's surface
    /// makes it the focus; `leave` ends that; motion, buttons and scroll
    /// while it is the focus are its own.
    pub fn popup_pointer(&mut self, event: &wl_pointer::Event) -> bool {
        let Some(open) = self.popup.open.as_mut() else {
            return false;
        };
        match event {
            wl_pointer::Event::Enter {
                surface,
                surface_x,
                surface_y,
                ..
            } => {
                if *surface != open.surface {
                    if open.focused {
                        open.focused = false;
                        open.interaction.clear();
                        open.dirty = true;
                    }
                    return false;
                }
                open.focused = true;
                open.pointer = (*surface_x, *surface_y);
                // The bar's pointer is elsewhere: no hover, nothing armed.
                self.input.leave();
                self.popup_motion();
                true
            }
            wl_pointer::Event::Leave { surface, .. } => {
                if !open.focused || *surface != open.surface {
                    return false;
                }
                open.focused = false;
                open.dirty |= open.interaction.clear();
                true
            }
            _ if !open.focused => false,
            wl_pointer::Event::Motion {
                surface_x,
                surface_y,
                ..
            } => {
                open.pointer = (*surface_x, *surface_y);
                self.popup_motion();
                true
            }
            wl_pointer::Event::Button {
                button,
                state: WEnum::Value(state),
                ..
            } => {
                if *button == BTN_LEFT {
                    self.popup_button(*state == wl_pointer::ButtonState::Pressed);
                }
                true
            }
            // Scroll, axis and frame over the popup are not the bar's.
            _ => true,
        }
    }

    fn popup_motion(&mut self) {
        let Some(open) = self.popup.open.as_mut() else {
            return;
        };
        let x = device(open.pointer.0, open.scale);
        let y = device(open.pointer.1, open.scale);
        let outcome = open.interaction.motion(&open.content, &open.layout, x, y);
        open.dirty |= outcome.redraw;
        if let Some(activate) = outcome.activate {
            self.activate_popup(activate);
        }
    }

    fn popup_button(&mut self, pressed: bool) {
        let Some(open) = self.popup.open.as_mut() else {
            return;
        };
        let x = device(open.pointer.0, open.scale);
        let y = device(open.pointer.1, open.scale);
        let outcome = if pressed {
            open.interaction.press(&open.content, &open.layout, x, y)
        } else {
            open.interaction.release(&open.content, &open.layout, x, y)
        };
        open.dirty |= outcome.redraw;
        if let Some(activate) = outcome.activate {
            self.activate_popup(activate);
        }
    }
}

impl Dispatch<XdgWmBase, ()> for State {
    fn event(
        _: &mut Self,
        base: &XdgWmBase,
        event: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // A compositor that pings and hears nothing calls the client
        // unresponsive.
        if let xdg_wm_base::Event::Ping { serial } = event {
            base.pong(serial);
        }
    }
}

impl Dispatch<XdgSurface, PopupId> for State {
    fn event(
        state: &mut Self,
        xdg: &XdgSurface,
        event: xdg_surface::Event,
        id: &PopupId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let xdg_surface::Event::Configure { serial } = event else {
            return;
        };
        let Some(open) = state.popup.open.as_mut().filter(|open| open.id == *id) else {
            return;
        };
        if &open.xdg != xdg {
            return;
        }
        xdg.ack_configure(serial);
        if open.mapped {
            return;
        }
        open.mapped = true;
        // The compositor's size wins over the one asked, once: the buffer
        // is made at it (never resized after).
        if let Some(size) = open.configured.filter(|size| {
            size.width > 0
                && size.height > 0
                && *size != open.requested
                && size.width <= super::MAX_SIDE
                && size.height <= super::MAX_SIDE
        }) {
            if let Some(dims) = open.scale.buffer(size) {
                open.requested = size;
                open.dims = dims;
                open.layout.width = dims.0;
                open.layout.height = dims.1;
            }
        }
        open.dirty = true;
    }
}

impl Dispatch<XdgPopup, PopupId> for State {
    fn event(
        state: &mut Self,
        popup: &XdgPopup,
        event: xdg_popup::Event,
        id: &PopupId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            xdg_popup::Event::Configure { width, height, .. } => {
                if let Some(open) = state.popup.open.as_mut().filter(|open| open.id == *id) {
                    if &open.popup == popup {
                        open.configured = Some(Size {
                            width: u32::try_from(width).unwrap_or(0),
                            height: u32::try_from(height).unwrap_or(0),
                        });
                    }
                }
            }
            // A click outside, the session locking, or the compositor's own
            // decision: the popup is over.
            xdg_popup::Event::PopupDone => state.close_popup_if(*id),
            _ => {}
        }
    }
}

impl Dispatch<WlBuffer, PopupId> for State {
    fn event(
        state: &mut Self,
        buffer: &WlBuffer,
        event: wl_buffer::Event,
        id: &PopupId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_buffer::Event::Release = event {
            if let Some(open) = state.popup.open.as_mut().filter(|open| open.id == *id) {
                open.pool.released(buffer);
            }
        }
    }
}

/// The popup's surface events say nothing it needs: it is drawn at the
/// scale of the bar it hangs off.
impl Dispatch<WlSurface, PopupId> for State {
    fn event(
        _: &mut Self,
        _: &WlSurface,
        _: wl_surface::Event,
        _: &PopupId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

/// The keyboard is taken for Escape only: a key press that is Escape closes
/// the popup, whatever else is typed is ignored, and the keymap (its fd is
/// closed as the event drops) is never read.
impl Dispatch<WlKeyboard, PopupId> for State {
    fn event(
        state: &mut Self,
        _: &WlKeyboard,
        event: wl_keyboard::Event,
        id: &PopupId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_keyboard::Event::Key {
            key,
            state: WEnum::Value(KeyState::Pressed),
            ..
        } = event
        {
            if key == KEY_ESC {
                state.close_popup_if(*id);
            }
        }
    }
}

delegate_noop!(State: XdgPositioner);
