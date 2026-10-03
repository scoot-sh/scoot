//! A virtual pointer for sway (`zwlr_virtual_pointer_v1`): a headless sway
//! has no input device, so no seat pointer and no click. A client that makes
//! a virtual pointer gives it one, and its buttons reach clients the way a
//! real device's do, through the compositor's own seat, **with the serials
//! and the button count wlroots checks a popup grab against**. That is the
//! point of it here: scoot accepts any recent serial, wlroots needs the
//! serial of a button still held.

#![allow(dead_code)]

use std::os::unix::net::UnixStream;
use std::path::Path;

use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_pointer::ButtonState;
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, delegate_noop};
use wayland_protocols_wlr::virtual_pointer::v1::client::zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1;
use wayland_protocols_wlr::virtual_pointer::v1::client::zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1;

/// `BTN_LEFT`.
const LEFT: u32 = 0x110;

#[derive(Default)]
struct Nothing;

pub struct VirtualPointer {
    queue: EventQueue<Nothing>,
    pointer: ZwlrVirtualPointerV1,
    /// The output's size: absolute motion is a fraction of it.
    extent: (u32, u32),
    time: u32,
    state: Nothing,
}

impl VirtualPointer {
    /// A virtual pointer on the compositor at `socket`, over an output of
    /// `extent` pixels at the layout's origin.
    pub fn new(socket: &Path, extent: (u32, u32)) -> Self {
        let stream = UnixStream::connect(socket).unwrap();
        let conn = Connection::from_socket(stream).unwrap();
        let (globals, mut queue) = registry_queue_init::<Nothing>(&conn).unwrap();
        let qh = queue.handle();
        let seat: WlSeat = globals.bind(&qh, 1..=1, ()).unwrap();
        let manager: ZwlrVirtualPointerManagerV1 = globals
            .bind(&qh, 1..=2, ())
            .expect("the compositor has no zwlr_virtual_pointer_manager_v1");
        let pointer = manager.create_virtual_pointer(Some(&seat), &qh, ());
        let mut state = Nothing;
        queue.roundtrip(&mut state).unwrap();
        Self {
            queue,
            pointer,
            extent,
            time: 0,
            state,
        }
    }

    fn tick(&mut self) -> u32 {
        self.time = self.time.wrapping_add(10);
        self.time
    }

    pub fn move_to(&mut self, x: u32, y: u32) {
        let time = self.tick();
        self.pointer
            .motion_absolute(time, x, y, self.extent.0, self.extent.1);
        self.pointer.frame();
        self.queue.roundtrip(&mut self.state).unwrap();
    }

    pub fn button(&mut self, pressed: bool) {
        let time = self.tick();
        self.pointer.button(
            time,
            LEFT,
            if pressed {
                ButtonState::Pressed
            } else {
                ButtonState::Released
            },
        );
        self.pointer.frame();
        self.queue.roundtrip(&mut self.state).unwrap();
    }
}

impl Dispatch<WlRegistry, GlobalListContents> for Nothing {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: wayland_client::protocol::wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

delegate_noop!(Nothing: ignore WlSeat);
delegate_noop!(Nothing: ZwlrVirtualPointerManagerV1);
delegate_noop!(Nothing: ZwlrVirtualPointerV1);
