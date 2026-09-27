//! The XDND half of the suites' X client: just enough of the protocol
//! (freedesktop XDND, version 5) to be a drag source that finds its target
//! the way a toolkit does -- the top-level window under the X pointer -- and
//! a drop target that answers and reads what was dropped.

use std::collections::VecDeque;

use x11rb::protocol::Event as XEvent;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ClientMessageEvent, ConnectionExt as _, EventMask, PropMode,
    SelectionNotifyEvent, Window,
};
use x11rb::wrapper::ConnectionExt as _;

use super::live::Fixture;
use super::x11::{XClient, eventually};

/// The XDND version both halves speak.
pub(super) const XDND_VERSION: u32 = 5;

/// What the window manager names its full-screen drop proxy (Smithay's
/// `xwm/dnd.rs`): what an X drag finds under the pointer while scoot is
/// relaying it to Wayland.
pub(super) const PROXY_NAME: &str = "Smithay XDND proxy";

/// XDND packs a root position into one word, x high.
pub(super) fn packed(x: i16, y: i16) -> u32 {
    (u32::from(x as u16) << 16) | u32::from(y as u16)
}

impl XClient {
    /// Marks `window` a drop target (`XdndAware`, [`XDND_VERSION`]).
    pub(super) fn xdnd_aware(&self, window: Window) {
        let aware = self.atom("XdndAware");
        self.conn
            .change_property32(
                PropMode::REPLACE,
                window,
                aware,
                AtomEnum::ATOM,
                &[XDND_VERSION],
            )
            .expect("a property request")
            .check()
            .expect("the X server accepted XdndAware");
    }

    /// Where the X server has the pointer, and the drop target under it the
    /// way a drag source looks for one: down the window tree from the root
    /// (`QueryPointer` at each level) to the first `XdndAware` window --
    /// the proxy while it is mapped, which covers the screen above every
    /// window; else the X window itself, inside the frame the window
    /// manager reparents it into. With nothing aware on the way, the
    /// deepest window found (`NONE` over bare root).
    pub(super) fn pointer(&self) -> (i16, i16, Window) {
        let aware = self.atom("XdndAware");
        let query = |window| {
            self.conn
                .query_pointer(window)
                .expect("a pointer query")
                .reply()
                .expect("the pointer")
        };
        let at_root = query(self.root);
        let (x, y) = (at_root.root_x, at_root.root_y);
        let mut window = at_root.child;
        // Bounded: a window tree is finite, and no real one is this deep.
        for _ in 0..16 {
            if window == x11rb::NONE {
                break;
            }
            let is_aware = self
                .conn
                .get_property(false, window, aware, AtomEnum::ATOM, 0, 1)
                .expect("a property request")
                .reply()
                .is_ok_and(|reply| reply.value_len > 0);
            if is_aware {
                return (x, y, window);
            }
            let child = query(window).child;
            if child == x11rb::NONE {
                break;
            }
            window = child;
        }
        (x, y, window)
    }

    /// `WM_NAME`, for naming a window in a failure.
    pub(super) fn name_of(&self, window: Window) -> String {
        if window == x11rb::NONE {
            return "no window".to_owned();
        }
        self.conn
            .get_property(false, window, AtomEnum::WM_NAME, AtomEnum::ANY, 0, 256)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .map(|reply| String::from_utf8_lossy(&reply.value).into_owned())
            .unwrap_or_default()
    }

    /// Adds pointer crossings to what `window` reports, beside what
    /// [`XClient::map`] selected.
    pub(super) fn select_crossings(&self, window: Window) {
        self.conn
            .change_window_attributes(
                window,
                &x11rb::protocol::xproto::ChangeWindowAttributesAux::new().event_mask(
                    EventMask::KEY_PRESS
                        | EventMask::FOCUS_CHANGE
                        | EventMask::STRUCTURE_NOTIFY
                        | EventMask::ENTER_WINDOW
                        | EventMask::LEAVE_WINDOW,
                ),
            )
            .expect("an attributes request")
            .check()
            .expect("the X server accepted the event mask");
    }

    /// Sends the XDND client message `kind` to `window`, the way a source
    /// or target does: straight to the window, no event mask.
    pub(super) fn xdnd_send(&self, window: Window, kind: &str, data: [u32; 5]) {
        let kind = self.atom(kind);
        self.conn
            .send_event(
                false,
                window,
                EventMask::NO_EVENT,
                ClientMessageEvent::new(32, window, kind, data),
            )
            .expect("a send request")
            .check()
            .expect("the X server accepted the message");
    }

    /// Asks for `selection` as `target`, into `property` on `requestor`.
    pub(super) fn convert(&self, selection: &str, target: Atom, property: &str, requestor: Window) {
        let (selection, property) = (self.atom(selection), self.atom(property));
        self.conn
            .convert_selection(requestor, selection, target, property, x11rb::CURRENT_TIME)
            .expect("a convert request")
            .check()
            .expect("the X server accepted the conversion");
    }

    /// Property `property` of `window`, whole.
    pub(super) fn read_property(&self, window: Window, property: &str) -> Vec<u8> {
        let property = self.atom(property);
        self.conn
            .get_property(false, window, property, AtomEnum::ANY, 0, u32::MAX / 4)
            .expect("a property request")
            .reply()
            .expect("the property")
            .value
    }
}

/// What one X client has been sent, kept across polls: a wait for one
/// message must not discard the others that arrived in the same batch (a
/// target gets `XdndEnter` and `XdndPosition` together).
pub(super) struct Inbox<'a> {
    client: &'a XClient,
    pending: VecDeque<XEvent>,
}

impl<'a> Inbox<'a> {
    pub(super) fn new(client: &'a XClient) -> Self {
        Self {
            client,
            pending: VecDeque::new(),
        }
    }

    /// Takes the first queued event `pick` accepts, polling the client
    /// first.
    fn take<T>(&mut self, mut pick: impl FnMut(&XEvent) -> Option<T>) -> Option<T> {
        self.pending.extend(self.client.drain());
        let index = self
            .pending
            .iter()
            .position(|event| pick(event).is_some())?;
        self.pending.remove(index).and_then(|event| pick(&event))
    }

    /// Waits for the next client message of `kind`, driving the compositor
    /// meanwhile, and keeps every other event queued.
    pub(super) fn message(&mut self, fixture: &mut Fixture, kind: &str) -> ClientMessageEvent {
        let atom = self.client.atom(kind);
        let mut found = None;
        eventually(fixture, kind, |_| {
            found = self.take(|event| match event {
                XEvent::ClientMessage(message) if message.type_ == atom => Some(*message),
                _ => None,
            });
            found.is_some()
        });
        found.expect("just waited for it")
    }

    /// Whether a client message of `kind` is queued now, without waiting.
    pub(super) fn has_message(&mut self, kind: &str) -> bool {
        let atom = self.client.atom(kind);
        self.pending.extend(self.client.drain());
        self.pending
            .iter()
            .any(|event| matches!(event, XEvent::ClientMessage(message) if message.type_ == atom))
    }

    /// Waits for the answer to a conversion.
    pub(super) fn selection_notify(&mut self, fixture: &mut Fixture) -> SelectionNotifyEvent {
        let mut found = None;
        eventually(fixture, "SelectionNotify", |_| {
            found = self.take(|event| match event {
                XEvent::SelectionNotify(notify) => Some(*notify),
                _ => None,
            });
            found.is_some()
        });
        found.expect("just waited for it")
    }

    /// The crossing events queued now: (enters, leaves).
    pub(super) fn crossings(&mut self) -> (usize, usize) {
        self.pending.extend(self.client.drain());
        let enters = self
            .pending
            .iter()
            .filter(|event| matches!(event, XEvent::EnterNotify(_)))
            .count();
        let leaves = self
            .pending
            .iter()
            .filter(|event| matches!(event, XEvent::LeaveNotify(_)))
            .count();
        (enters, leaves)
    }
}
