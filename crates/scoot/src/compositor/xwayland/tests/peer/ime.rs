//! The peer's input-method half: an `input-method-v2` client (fcitx5, an
//! on-screen keyboard) taking the keyboard grab, and the keys that reach it.

use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle};
use wayland_protocols_misc::zwp_input_method_v2::client::{
    zwp_input_method_keyboard_grab_v2 as keyboard_grab, zwp_input_method_manager_v2 as manager,
    zwp_input_method_v2 as input_method,
};

use super::{Ack, Peer};

/// The input-method steps.
#[derive(Debug)]
pub(in crate::compositor::xwayland::tests) enum ImeStep {
    /// Create an input method on the seat and take its keyboard grab.
    Grab,
    /// The evdev key codes pressed on the grab so far, in order.
    Keys,
}

#[derive(Default)]
pub(super) struct Ime {
    pub(super) manager: Option<manager::ZwpInputMethodManagerV2>,
    input_method: Option<input_method::ZwpInputMethodV2>,
    grab: Option<keyboard_grab::ZwpInputMethodKeyboardGrabV2>,
    pressed: Vec<u32>,
}

pub(super) fn step(
    peer: &mut Peer,
    queue: &mut EventQueue<Peer>,
    step: ImeStep,
) -> Result<Ack, String> {
    let qh = queue.handle();
    match step {
        ImeStep::Grab => {
            let seat = peer.seat.clone().ok_or("no wl_seat")?;
            let manager = peer
                .ime
                .manager
                .clone()
                .ok_or("no zwp_input_method_manager_v2")?;
            let method = manager.get_input_method(&seat, &qh, ());
            peer.ime.grab = Some(method.grab_keyboard(&qh, ()));
            peer.ime.input_method = Some(method);
            queue.roundtrip(peer).map_err(|e| e.to_string())?;
            queue.roundtrip(peer).map_err(|e| e.to_string())?;
            Ok(Ack::Done)
        }
        ImeStep::Keys => {
            queue.roundtrip(peer).map_err(|e| e.to_string())?;
            Ok(Ack::Keys(peer.ime.pressed.clone()))
        }
    }
}

impl Dispatch<keyboard_grab::ZwpInputMethodKeyboardGrabV2, ()> for Peer {
    fn event(
        peer: &mut Self,
        _: &keyboard_grab::ZwpInputMethodKeyboardGrabV2,
        event: keyboard_grab::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let keyboard_grab::Event::Key { key, state, .. } = event
            && state
                == wayland_client::WEnum::Value(
                    wayland_client::protocol::wl_keyboard::KeyState::Pressed,
                )
        {
            peer.ime.pressed.push(key);
        }
    }
}

wayland_client::delegate_noop!(Peer: ignore manager::ZwpInputMethodManagerV2);
wayland_client::delegate_noop!(Peer: ignore input_method::ZwpInputMethodV2);
