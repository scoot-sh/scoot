//! A virtual keyboard for sway (`zwp_virtual_keyboard_v1`): a headless
//! sway has no input device, so its seat has no keyboard and a layer
//! surface asking for one is never handed keys. A client that makes a
//! virtual keyboard gives the seat one, and its keys reach the focused
//! client through the compositor's own seat. It uploads the keymap
//! itself, so this carries a minimal one with the two keys the windows
//! tests press (Return and Down).

#![allow(dead_code)]

use std::io::{Seek, SeekFrom, Write};
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::{Duration, Instant};

use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, delegate_noop};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1;
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1;

/// evdev `KEY_ENTER` and `KEY_DOWN`.
pub const RETURN: u32 = 28;
pub const DOWN: u32 = 108;

/// XKB keycodes are evdev + 8.
const KEYMAP: &str = r#"xkb_keymap {
  xkb_keycodes "k" {
    minimum = 8;
    maximum = 255;
    <RTRN> = 36;
    <DOWN> = 116;
  };
  xkb_types "t" {
    type "ONE_LEVEL" {
      modifiers = None;
      map[None] = Level1;
      level_name[Level1] = "Any";
    };
  };
  xkb_compat "c" {
  };
  xkb_symbols "s" {
    key <RTRN> { type = "ONE_LEVEL", symbols[Group1] = [ Return ] };
    key <DOWN> { type = "ONE_LEVEL", symbols[Group1] = [ Down ] };
  };
};
"#;

#[derive(Default)]
struct Nothing;

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
delegate_noop!(Nothing: ignore ZwpVirtualKeyboardManagerV1);
delegate_noop!(Nothing: ignore ZwpVirtualKeyboardV1);

pub struct VirtualKeyboard {
    queue: EventQueue<Nothing>,
    keyboard: ZwpVirtualKeyboardV1,
    state: Nothing,
    started: Instant,
}

impl VirtualKeyboard {
    /// A virtual keyboard on the compositor at `socket`, with the
    /// keymap uploaded and the seat's keyboard capability announced.
    pub fn new(socket: &Path) -> Self {
        let stream = UnixStream::connect(socket).unwrap();
        let conn = Connection::from_socket(stream).unwrap();
        let (globals, mut queue) = registry_queue_init::<Nothing>(&conn).unwrap();
        let qh = queue.handle();
        let seat: WlSeat = globals.bind(&qh, 1..=1, ()).unwrap();
        let manager: ZwpVirtualKeyboardManagerV1 = globals
            .bind(&qh, 1..=1, ())
            .expect("the compositor has no zwp_virtual_keyboard_manager_v1");
        let keyboard = manager.create_virtual_keyboard(&seat, &qh, ());
        let fd = rustix::fs::memfd_create(c"vk-keymap", rustix::fs::MemfdFlags::CLOEXEC).unwrap();
        let mut file = std::fs::File::from(fd);
        // The format wants a NUL-terminated string.
        file.write_all(KEYMAP.as_bytes()).unwrap();
        file.write_all(&[0]).unwrap();
        file.seek(SeekFrom::Start(0)).unwrap();
        keyboard.keymap(1, file.as_fd(), KEYMAP.len() as u32 + 1);
        let mut state = Nothing;
        queue.roundtrip(&mut state).unwrap();
        Self {
            queue,
            keyboard,
            state,
            started: Instant::now(),
        }
    }

    /// Presses and releases the evdev key `code`, and waits until the
    /// compositor has taken both.
    pub fn tap(&mut self, code: u32) {
        let time = |this: &Self| this.started.elapsed().as_millis() as u32;
        self.keyboard.key(time(self), code, 1);
        self.keyboard.key(time(self), code, 0);
        self.queue.roundtrip(&mut self.state).unwrap();
        // Let the focused client see the events before the next key.
        std::thread::sleep(Duration::from_millis(50));
    }
}
