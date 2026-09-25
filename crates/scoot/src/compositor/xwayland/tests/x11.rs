//! The X11 side of the live suites: a real X client, over `x11rb`, against
//! the session's own XWayland server -- creating windows with exactly the
//! properties a toolkit would set, and reading back what the X server says.

use std::time::Instant;

use x11rb::COPY_DEPTH_FROM_PARENT;
use x11rb::connection::Connection as _;
use x11rb::properties::{WmSizeHints, WmSizeHintsSpecification};
use x11rb::protocol::Event as XEvent;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ClientMessageEvent, ConnectionExt as _, CreateWindowAux, EventMask, PropMode,
    Window, WindowClass,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

use super::XWAYLAND_PATIENCE;

/// What a test window says about itself before it maps.
#[derive(Clone, Debug)]
pub(super) struct Props {
    /// Where the window is created, and its size.
    pub(super) rect: (i16, i16, u16, u16),
    /// The background pixel: what XWayland draws the window as.
    pub(super) pixel: u32,
    /// `WM_CLASS` (instance, class).
    pub(super) class: Option<(&'static str, &'static str)>,
    /// `WM_NAME` (and `_NET_WM_NAME`).
    pub(super) title: Option<&'static str>,
    /// `WM_TRANSIENT_FOR`.
    pub(super) transient_for: Option<Window>,
    /// `_NET_WM_WINDOW_TYPE_DIALOG`.
    pub(super) dialog: bool,
    /// `_NET_WM_STATE_FULLSCREEN`, set before mapping.
    pub(super) fullscreen: bool,
    /// `WM_NORMAL_HINTS` with `USPosition` set.
    pub(super) us_position: bool,
    /// `WM_NORMAL_HINTS` minimum and maximum.
    pub(super) min_max: Option<((i32, i32), (i32, i32))>,
    /// `_NET_STARTUP_ID`.
    pub(super) startup_id: Option<String>,
    /// An override-redirect window.
    pub(super) override_redirect: bool,
    /// `_GTK_FRAME_EXTENTS` (left, right, top, bottom), set before mapping.
    pub(super) frame_extents: Option<[u32; 4]>,
}

impl Props {
    pub(super) fn new(pixel: u32) -> Self {
        Self {
            rect: (0, 0, 120, 90),
            pixel,
            class: None,
            title: None,
            transient_for: None,
            dialog: false,
            fullscreen: false,
            us_position: false,
            min_max: None,
            startup_id: None,
            override_redirect: false,
            frame_extents: None,
        }
    }
}

/// One X client connection, with the atoms the suites use interned once.
pub(super) struct XClient {
    pub(super) conn: RustConnection,
    pub(super) root: Window,
    visual: u32,
    atoms: Atoms,
}

#[derive(Clone, Copy)]
struct Atoms {
    utf8: Atom,
    net_wm_name: Atom,
    window_type: Atom,
    window_type_dialog: Atom,
    net_state: Atom,
    net_state_fullscreen: Atom,
    net_active_window: Atom,
    startup_id: Atom,
    wm_protocols: Atom,
    wm_delete_window: Atom,
}

impl XClient {
    pub(super) fn connect(display: u32) -> Self {
        let name = super::display_value(display);
        let (conn, screen_num) = x11rb::connect(Some(name.as_str())).expect("an X connection");
        let screen = &conn.setup().roots[screen_num];
        let (root, visual) = (screen.root, screen.root_visual);
        let intern = |name: &str| {
            conn.intern_atom(false, name.as_bytes())
                .expect("an intern request")
                .reply()
                .expect("an atom")
                .atom
        };
        let atoms = Atoms {
            utf8: intern("UTF8_STRING"),
            net_wm_name: intern("_NET_WM_NAME"),
            window_type: intern("_NET_WM_WINDOW_TYPE"),
            window_type_dialog: intern("_NET_WM_WINDOW_TYPE_DIALOG"),
            net_state: intern("_NET_WM_STATE"),
            net_state_fullscreen: intern("_NET_WM_STATE_FULLSCREEN"),
            net_active_window: intern("_NET_ACTIVE_WINDOW"),
            startup_id: intern("_NET_STARTUP_ID"),
            wm_protocols: intern("WM_PROTOCOLS"),
            wm_delete_window: intern("WM_DELETE_WINDOW"),
        };
        Self {
            conn,
            root,
            visual,
            atoms,
        }
    }

    /// Creates a window with `props` and maps it. Selects key presses and
    /// focus changes, so a test can read what reached it.
    pub(super) fn map(&self, props: &Props) -> Window {
        let window = self.conn.generate_id().expect("an X window id");
        let (x, y, w, h) = props.rect;
        self.conn
            .create_window(
                COPY_DEPTH_FROM_PARENT,
                window,
                self.root,
                x,
                y,
                w,
                h,
                0,
                WindowClass::INPUT_OUTPUT,
                self.visual,
                &CreateWindowAux::new()
                    .background_pixel(props.pixel)
                    .override_redirect(u32::from(props.override_redirect))
                    .event_mask(
                        EventMask::KEY_PRESS
                            | EventMask::FOCUS_CHANGE
                            | EventMask::STRUCTURE_NOTIFY,
                    ),
            )
            .expect("a create request")
            .check()
            .expect("the X server accepted the window");
        if let Some((instance, class)) = props.class {
            let value = format!("{instance}\0{class}\0");
            self.string(
                window,
                AtomEnum::WM_CLASS.into(),
                AtomEnum::STRING.into(),
                &value,
            );
        }
        if let Some(title) = props.title {
            self.string(
                window,
                AtomEnum::WM_NAME.into(),
                AtomEnum::STRING.into(),
                title,
            );
            self.string(window, self.atoms.net_wm_name, self.atoms.utf8, title);
        }
        if let Some(parent) = props.transient_for {
            self.conn
                .change_property32(
                    PropMode::REPLACE,
                    window,
                    AtomEnum::WM_TRANSIENT_FOR,
                    AtomEnum::WINDOW,
                    &[parent],
                )
                .expect("a property request");
        }
        if props.dialog {
            self.conn
                .change_property32(
                    PropMode::REPLACE,
                    window,
                    self.atoms.window_type,
                    AtomEnum::ATOM,
                    &[self.atoms.window_type_dialog],
                )
                .expect("a property request");
        }
        if props.fullscreen {
            self.conn
                .change_property32(
                    PropMode::REPLACE,
                    window,
                    self.atoms.net_state,
                    AtomEnum::ATOM,
                    &[self.atoms.net_state_fullscreen],
                )
                .expect("a property request");
        }
        if props.us_position || props.min_max.is_some() {
            let mut hints = WmSizeHints::new();
            if props.us_position {
                hints.position = Some((
                    WmSizeHintsSpecification::UserSpecified,
                    i32::from(x),
                    i32::from(y),
                ));
            }
            if let Some((min, max)) = props.min_max {
                hints.min_size = Some(min);
                hints.max_size = Some(max);
            }
            hints
                .set_normal_hints(&self.conn, window)
                .expect("a hints request");
        }
        if let Some(extents) = props.frame_extents {
            self.set_frame_extents(window, extents);
        }
        if let Some(startup) = &props.startup_id {
            self.string(window, self.atoms.startup_id, self.atoms.utf8, startup);
        }
        self.conn
            .change_property32(
                PropMode::REPLACE,
                window,
                self.atoms.wm_protocols,
                AtomEnum::ATOM,
                &[self.atoms.wm_delete_window],
            )
            .expect("a property request");
        self.conn
            .map_window(window)
            .expect("a map request")
            .check()
            .expect("the X server accepted the map");
        self.conn.flush().expect("the requests hit the wire");
        window
    }

    fn string(&self, window: Window, property: Atom, kind: Atom, value: &str) {
        self.conn
            .change_property8(PropMode::REPLACE, window, property, kind, value.as_bytes())
            .expect("a property request");
    }

    /// Retitles a mapped window, the way a terminal does on every prompt.
    pub(super) fn retitle(&self, window: Window, title: &str) {
        self.string(
            window,
            AtomEnum::WM_NAME.into(),
            AtomEnum::STRING.into(),
            title,
        );
        self.string(window, self.atoms.net_wm_name, self.atoms.utf8, title);
        self.conn.flush().expect("the retitle hit the wire");
    }

    /// Sends `_NET_ACTIVE_WINDOW` for `window` to the root, as a pager (or
    /// `xdotool windowactivate`) does: source indication 2, current time.
    pub(super) fn request_activation(&self, window: Window) {
        let event =
            ClientMessageEvent::new(32, window, self.atoms.net_active_window, [2, 0, 0, 0, 0]);
        self.conn
            .send_event(
                false,
                self.root,
                EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
                event,
            )
            .expect("a send request");
        self.conn.flush().expect("the request hit the wire");
    }

    /// Adds `_NET_WM_STATE_FULLSCREEN` to a mapped window the EWMH way: a
    /// client message to the root, which the window manager answers.
    pub(super) fn request_fullscreen(&self, window: Window, fullscreen: bool) {
        let event = ClientMessageEvent::new(
            32,
            window,
            self.atoms.net_state,
            [
                u32::from(fullscreen),
                self.atoms.net_state_fullscreen,
                0,
                1,
                0,
            ],
        );
        self.conn
            .send_event(
                false,
                self.root,
                EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
                event,
            )
            .expect("a send request");
        self.conn.flush().expect("the request hit the wire");
    }

    /// Whether the window's `_NET_WM_STATE` holds fullscreen, as the X server
    /// reads it -- what the window manager last set.
    pub(super) fn is_fullscreen(&self, window: Window) -> bool {
        self.conn
            .get_property(false, window, self.atoms.net_state, AtomEnum::ATOM, 0, 64)
            .expect("a property request")
            .reply()
            .expect("the property")
            .value32()
            .is_some_and(|mut atoms| atoms.any(|atom| atom == self.atoms.net_state_fullscreen))
    }

    /// The X server's input focus: which X window keys go to.
    pub(super) fn input_focus(&self) -> Window {
        self.conn
            .get_input_focus()
            .expect("a focus request")
            .reply()
            .expect("the focus")
            .focus
    }

    /// Every event queued for this client, drained.
    pub(super) fn drain(&self) -> Vec<XEvent> {
        let mut events = Vec::new();
        while let Some(event) = self.conn.poll_for_event().expect("an event poll") {
            events.push(event);
        }
        events
    }

    /// Unmaps a window, the way a client withdraws it.
    pub(super) fn unmap(&self, window: Window) {
        self.conn
            .unmap_window(window)
            .expect("an unmap request")
            .check()
            .expect("the X server accepted the unmap");
        self.conn.flush().expect("the unmap hit the wire");
    }
}

/// Polls `ready` against the X client's view, driving the compositor between
/// polls, until it holds or the deadline passes.
pub(super) fn eventually<S, A>(
    fixture: &mut crate::compositor::test_support::Harness<S, A>,
    what: &str,
    mut ready: impl FnMut(&mut crate::compositor::test_support::Harness<S, A>) -> bool,
) {
    let deadline = Instant::now() + XWAYLAND_PATIENCE;
    loop {
        fixture.settle();
        if ready(fixture) {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
    }
}

impl XClient {
    /// Sets `_GTK_FRAME_EXTENTS` (left, right, top, bottom) on a window, as
    /// a GTK client with client-side shadows does -- or a hostile one, with
    /// whatever numbers it likes.
    pub(super) fn set_frame_extents(&self, window: Window, extents: [u32; 4]) {
        let atom = self
            .conn
            .intern_atom(false, b"_GTK_FRAME_EXTENTS")
            .expect("an intern request")
            .reply()
            .expect("an atom")
            .atom;
        self.conn
            .change_property32(
                PropMode::REPLACE,
                window,
                atom,
                AtomEnum::CARDINAL,
                &extents,
            )
            .expect("a property request");
        self.conn.flush().expect("the property hit the wire");
    }
}
