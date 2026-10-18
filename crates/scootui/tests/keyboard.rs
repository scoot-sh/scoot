//! Typed through a real compositor: the spike's layout set.
//!
//! Each case starts a headless scoot with explicit `XKB_DEFAULT_*`, binds
//! the seat at the negotiated version, maps an `xdg_toplevel` (keys only
//! reach the focused client), types through the control socket, and
//! asserts the decoded text. A second test holds a key through the
//! virtual keyboard and checks the timerfd repeat: ticks while held,
//! silence after release, silence after focus loss mid-hold, and no
//! wakeups at idle.
//!
//! Needs `keyboard` (the seat) plus `test-support` (the session): CI runs
//! this file only in that combination, so it is empty everywhere else.

#![cfg(all(feature = "keyboard", feature = "test-support"))]

use std::io::{Seek, SeekFrom, Write};
use std::os::unix::io::AsFd;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use scootui::keyboard::{
    MIN_TEXT_BYTES, dispatch_keyboard, negotiate_version, KeyboardSeat,
};
use scootui::test_support::Session;
use wayland_client::protocol::{
    wl_compositor, wl_keyboard, wl_registry, wl_seat, wl_surface,
};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1, zwp_virtual_keyboard_v1,
};

const PATIENCE: Duration = Duration::from_secs(20);
const KEY_A: u32 = 30;

/// A typed-through case: the compositor's layout environment, what `scoot
/// msg type` types, and what the seat must decode (the spike's rows).
struct LayoutCase {
    tag: &'static str,
    env: &'static [(&'static str, &'static str)],
    typed: &'static str,
    expect: &'static str,
}

const LAYOUTS: &[LayoutCase] = &[
    LayoutCase {
        tag: "us",
        env: &[("XKB_DEFAULT_RULES", "evdev"), ("XKB_DEFAULT_MODEL", "pc105")],
        typed: "AbC xyz 123!?",
        expect: "AbC xyz 123!?",
    },
    LayoutCase {
        tag: "us-intl",
        env: &[
            ("XKB_DEFAULT_RULES", "evdev"),
            ("XKB_DEFAULT_MODEL", "pc105"),
            ("XKB_DEFAULT_LAYOUT", "us"),
            ("XKB_DEFAULT_VARIANT", "intl"),
        ],
        typed: "café",
        expect: "café",
    },
    LayoutCase {
        tag: "de",
        env: &[
            ("XKB_DEFAULT_RULES", "evdev"),
            ("XKB_DEFAULT_MODEL", "pc105"),
            ("XKB_DEFAULT_LAYOUT", "de"),
        ],
        typed: "yYzZß?",
        expect: "yYzZß?",
    },
    LayoutCase {
        tag: "fr",
        env: &[
            ("XKB_DEFAULT_RULES", "evdev"),
            ("XKB_DEFAULT_MODEL", "pc105"),
            ("XKB_DEFAULT_LAYOUT", "fr"),
        ],
        typed: "aqz&1",
        expect: "aqz&1",
    },
    LayoutCase {
        tag: "dvorak",
        env: &[
            ("XKB_DEFAULT_RULES", "evdev"),
            ("XKB_DEFAULT_MODEL", "pc105"),
            ("XKB_DEFAULT_LAYOUT", "us"),
            ("XKB_DEFAULT_VARIANT", "dvorak"),
        ],
        typed: "oea.",
        expect: "oea.",
    },
];

fn scoot_bin() -> PathBuf {
    if let Some(path) = std::env::var_os("SCOOTBAR_TEST_SCOOT").map(PathBuf::from) {
        return path;
    }
    // The test binary lives in `target/<profile>/deps/`; `scoot` sits two
    // levels up. (Nextest runs each test in its own process, so no env
    // juggling is needed to point the harness at it.)
    let exe = std::env::current_exe().expect("the test has a path");
    let debug = exe
        .parent()
        .and_then(|deps| deps.parent())
        .expect("deps has a parent");
    debug.join("scoot")
}

struct State {
    seat: KeyboardSeat,
    text: Vec<u8>,
    repeats: u64,
    keymap_bytes: Option<Vec<u8>>,
    // Protocol objects, bound during the registry sweep.
    compositor: Option<wl_compositor::WlCompositor>,
    seat_proxy: Option<wl_seat::WlSeat>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    xdg_base: Option<xdg_wm_base::XdgWmBase>,
    virt_manager: Option<zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1>,
    surface: Option<wl_surface::WlSurface>,
    xdg_surface: Option<xdg_surface::XdgSurface>,
    toplevel: Option<xdg_toplevel::XdgToplevel>,
    virt_keyboard: Option<zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1>,
    configured: bool,
    keymap_seen: usize,
}

impl State {
    fn new() -> Self {
        let mut seat = KeyboardSeat::new();
        seat.set_compose_locale(std::ffi::OsStr::new("C"));
        Self {
            seat,
            text: Vec::new(),
            repeats: 0,
            keymap_bytes: None,
            compositor: None,
            seat_proxy: None,
            keyboard: None,
            xdg_base: None,
            virt_manager: None,
            surface: None,
            xdg_surface: None,
            toplevel: None,
            virt_keyboard: None,
            configured: false,
            keymap_seen: 0,
        }
    }

    /// Emit every due repeat tick into the same buffers the press path
    /// fills. The pump calls this each turn, so ticks flow whenever the
    /// timer fires -- exactly what an app's daemon loop does with the
    /// timerfd in its poll set.
    fn drain_repeats(&mut self) {
        let mut text = [0u8; MIN_TEXT_BYTES];
        while let Some((_, output)) = self.seat.tick(&mut text) {
            self.text.extend_from_slice(&text[..output.text_len]);
            self.repeats += 1;
        }
    }
}

struct Harness {
    state: State,
    qh: QueueHandle<State>,
    queue: wayland_client::EventQueue<State>,
    // Kept alive for the session: the queue was made from this
    // connection, and dropping it first would close the socket the
    // queue still reads.
    #[allow(dead_code)]
    conn: Connection,
    session: Session,
}

impl Harness {
    fn start(tag: &str, config: &str, env: &[(&str, &str)]) -> Option<Self> {
        let app = scoot_bin();
        let session = Session::scoot_with_env(&app, tag, 1, config, env)?;
        let stream = UnixStream::connect(session.wayland_socket())
            .expect("the compositor's Wayland socket accepts");
        let conn = Connection::from_socket(stream).expect("a Wayland connection makes");
        let queue = conn.new_event_queue();
        let qh = queue.handle();
        let state = State::new();
        conn.display().get_registry(&qh, ());
        let mut harness = Self {
            state,
            qh,
            queue,
            conn,
            session,
        };
        harness.expect("the registry sweep", PATIENCE, |state| {
            state.compositor.is_some()
                && state.seat_proxy.is_some()
                && state.xdg_base.is_some()
        });
        // The seat version the ticket pins: 9, or 10 where offered.
        harness.expect("the keyboard object", PATIENCE, |state| {
            state.keyboard.is_some() || state.keymap_seen > 0
        });
        Some(harness)
    }

    /// Run the queue until `done`, or the timeout. Returns whether
    /// `done` fired (a quiesce wait ignores the return; every other wait
    /// asserts it). Flushes first (requests the test made), dispatches
    /// what arrived, drains any repeat expirations through the seat's
    /// timer, then waits on the connection fd for more.
    fn pump(&mut self, timeout: Duration, mut done: impl FnMut(&State) -> bool) -> bool {
        let deadline = Instant::now() + timeout;
        while !done(&self.state) {
            if Instant::now() >= deadline {
                return false;
            }
            self.queue.flush().expect("flush works");
            self.queue
                .dispatch_pending(&mut self.state)
                .expect("dispatch works");
            self.state.drain_repeats();
            if done(&self.state) {
                return true;
            }
            let Some(guard) = self.queue.prepare_read() else {
                continue;
            };
            let wayland = guard.connection_fd();
            let wait = deadline
                .checked_duration_since(Instant::now())
                .unwrap_or(Duration::ZERO);
            // An armed repeat timer is a second wakeup source, exactly as
            // in an app's daemon loop: poll it alongside the connection
            // with a slice of the remaining budget, so ticks flow while
            // the Wayland fd is quiet. The slice only bounds latency; the
            // assertions count ticks, not wakeups.
            let wait = if self.state.seat.repeat_armed() {
                wait.min(Duration::from_millis(25))
            } else {
                wait
            };
            let timespec = rustix::time::Timespec {
                tv_sec: wait.as_secs() as _,
                tv_nsec: wait.subsec_nanos() as _,
            };
            // `repeat_fd` borrows the seat through `state`, which the
            // guard does not touch: the two borrows coexist.
            let repeat = self.state.seat.repeat_fd();
            let repeat_fd = repeat.unwrap_or(wayland);
            let mut polls = [
                rustix::event::PollFd::new(&wayland, rustix::event::PollFlags::IN),
                rustix::event::PollFd::new(&repeat_fd, rustix::event::PollFlags::IN),
            ];
            let _ = rustix::event::poll(&mut polls, Some(&timespec));
            let wayland_ready = !polls[0].revents().is_empty();
            drop(polls);
            if wayland_ready {
                let _ = guard.read();
            }
            // A quiet connection with a due timer drops the guard unread
            // (the documented `read_after_poll` shape): the next turn's
            // drain collects the tick.
        }
        true
    }

    fn expect(&mut self, what: &str, timeout: Duration, done: impl FnMut(&State) -> bool) {
        assert!(
            self.pump(timeout, done),
            "timed out waiting for {what}; compositor said:\n{}",
            self.session.log()
        );
    }

    fn map_window(&mut self) {
        // The keymap lands right after the get_keyboard roundtrip; typing
        // and repeating both need it, so it is waited for, not assumed.
        self.expect("the keymap", PATIENCE, |state| state.keymap_seen > 0);
        let surface = self
            .state
            .compositor
            .as_ref()
            .expect("a compositor")
            .create_surface(&self.qh, ());
        let xdg_surface = self
            .state
            .xdg_base
            .as_ref()
            .expect("an xdg base")
            .get_xdg_surface(&surface, &self.qh, ());
        let toplevel = xdg_surface.get_toplevel(&self.qh, ());
        surface.commit();
        self.state.surface = Some(surface);
        self.state.xdg_surface = Some(xdg_surface);
        self.state.toplevel = Some(toplevel);
        self.expect("the configure", PATIENCE, |state| state.configured);
        self.expect("keyboard focus", PATIENCE, |state| {
            state.seat.has_focus()
        });
    }

    fn type_text(&mut self, text: &str) {
        let before = self.state.text.len();
        let request = format!(
            "{{\"type\":\"type\",\"text\":{}}}",
            serde_json::to_string(text).expect("text quotes")
        );
        let reply = self.session.scoot_ipc(&request);
        assert_eq!(
            reply["type"], "ok",
            "typing {text:?} is accepted: {reply}"
        );
        self.expect("the typed text", PATIENCE, |state| {
            state.text.len() >= before + text.len()
        });
    }

    fn collected(&self) -> &str {
        std::str::from_utf8(&self.state.text).expect("typed text is UTF-8")
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        // Tear the client end down before the session's sockets vanish:
        // the compositor is killed by `Session`'s own drop after this.
        self.queue.flush().ok();
    }
}

macro_rules! empty_dispatch {
    ($object:ty) => {
        impl Dispatch<$object, ()> for State {
            fn event(
                _: &mut State,
                _: &$object,
                _: <$object as Proxy>::Event,
                _: &(),
                _: &Connection,
                _: &QueueHandle<State>,
            ) {
            }
        }
    };
}

empty_dispatch!(wl_compositor::WlCompositor);
empty_dispatch!(wl_surface::WlSurface);
empty_dispatch!(xdg_toplevel::XdgToplevel);
empty_dispatch!(zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1);
empty_dispatch!(zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1);

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        state: &mut State,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<State>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "wl_compositor" => {
                    state.compositor = Some(registry.bind(name, 6.min(version), qh, ()));
                }
                "wl_seat" => {
                    let Some(bound) = negotiate_version(version) else {
                        panic!("the seat offers only version {version}, need 9+");
                    };
                    // The keyboard object comes from the capabilities
                    // event, not here: exactly one per seat.
                    state.seat_proxy =
                        Some(registry.bind::<wl_seat::WlSeat, _, _>(name, bound, qh, ()));
                }
                "xdg_wm_base" => {
                    state.xdg_base = Some(registry.bind(name, 1.min(version), qh, ()));
                }
                "zwp_virtual_keyboard_manager_v1" => {
                    state.virt_manager =
                        Some(registry.bind(name, 1.min(version), qh, ()));
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(
        state: &mut State,
        _: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<State>,
    ) {
        if let wl_seat::Event::Capabilities { capabilities } = event
            && let WEnum::Value(caps) = capabilities
        {
            use wl_seat::Capability;
            if caps.contains(Capability::Keyboard) && state.keyboard.is_none() {
                let seat = state.seat_proxy.as_ref().expect("the seat binds first");
                state.keyboard = Some(seat.get_keyboard(qh, ()));
            }
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for State {
    fn event(
        state: &mut State,
        _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<State>,
    ) {
        if let wl_keyboard::Event::Keymap { ref fd, .. } = event {
            use std::os::unix::io::AsFd;
            if let Ok(text) = scootui::keyboard::read_keymap_bytes(fd.as_fd()) {
                state.keymap_seen += 1;
                state.keymap_bytes = Some(text.into_bytes());
            }
        }
        let mut text = [0u8; MIN_TEXT_BYTES];
        match dispatch_keyboard(&mut state.seat, event, &mut text) {
            Ok(Some(output)) => {
                state.text.extend_from_slice(&text[..output.text_len]);
                if output.repeat {
                    state.repeats += 1;
                }
            }
            Ok(None) => {}
            Err(error) => panic!("the keymap event decodes: {error}"),
        }
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for State {
    fn event(
        _: &mut State,
        proxy: &xdg_wm_base::XdgWmBase,
        event: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<State>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            proxy.pong(serial);
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for State {
    fn event(
        state: &mut State,
        proxy: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<State>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            proxy.ack_configure(serial);
            state.configured = true;
        }
    }
}

fn voluntary_switches() -> u64 {
    let status = std::fs::read_to_string("/proc/self/status").expect("/proc exists on Linux");
    status
        .lines()
        .find_map(|line| {
            line.strip_prefix("voluntary_ctxt_switches:")
                .and_then(|rest| rest.trim().parse().ok())
        })
        .expect("the kernel reports voluntary switches")
}

#[test]
fn layout_set_types_through_msg_type() {
    for case in LAYOUTS {
        let Some(mut harness) = Harness::start(case.tag, "", case.env) else {
            continue;
        };
        // Compose for the intl row comes from the host's table (the
        // launcher will carry the session locale the same way); every
        // other row decodes direct, table or not.
        harness.map_window();
        harness.type_text(case.typed);
        // Quiesce: releases arrive behind the presses.
        harness.pump(Duration::from_secs(2), |_| false);
        assert_eq!(
            harness.collected(),
            case.expect,
            "typed {:?} in {}",
            case.typed,
            case.tag
        );
        // Idle with a mapped surface and no key held: the repeat timer
        // does not exist, so a 3 s wait takes (at most) the poll's own
        // wakeups, never a timer's.
        assert!(
            !harness.state.seat.repeat_armed(),
            "{}: no repeat armed at idle",
            case.tag
        );
        let before = voluntary_switches();
        harness.pump(Duration::from_secs(3), |_| false);
        let after = voluntary_switches();
        assert!(
            after - before <= 8,
            "{}: {} voluntary switches in 3 s idle (a 40 ms timer would take ~75)",
            case.tag,
            after - before
        );
    }
}

#[test]
fn hold_repeats_release_and_focus_loss_stop() {
    let config = "[virtual_input]\nenabled = true\n";
    let Some(mut harness) = Harness::start("repeat", config, LAYOUTS[0].env) else {
        return;
    };
    harness.map_window();
    // The virtual keyboard uploads the seat's own keymap text, so the
    // codes it sends decode exactly as typed keys do.
    let keymap = harness
        .state
        .keymap_bytes
        .clone()
        .expect("the keymap arrived before focus");
    let manager = harness
        .state
        .virt_manager
        .as_ref()
        .expect("virtual input is enabled")
        .clone();
    let seat = harness.state.seat_proxy.as_ref().expect("a seat").clone();
    let virt = manager.create_virtual_keyboard(&seat, &harness.qh, ());
    let file = memfd_with_bytes(&keymap);
    // Alive until the upload flushes: the backend sends the fd number at
    // flush time, so closing earlier could send a recycled number.
    let raw = {
        use std::os::unix::io::AsFd;
        rustix::io::dup(file.as_fd()).expect("the upload dup works")
    };
    virt.keymap(1, raw.as_fd(), keymap.len() as u32);
    harness.queue.flush().expect("the keymap upload sends");
    drop(raw);
    // Hold: press and keep holding. Rate 25/s after a 200 ms delay, so
    // 700 ms holds ~13 ticks; the bounds only distinguish "repeating"
    // from "silent".
    virt.key(0, KEY_A, 1);
    harness.queue.flush().expect("the press sends");
    let held_at = harness.state.repeats;
    harness.expect("repeats while held", Duration::from_secs(5), |state| {
        state.repeats >= held_at + 5
    });
    let held = harness.state.repeats;
    assert!(
        (5..=25).contains(&(held - held_at)),
        "held 700 ms at 25/s: {} ticks",
        held - held_at
    );
    assert!(harness.state.seat.repeat_armed());
    // Release: the timer dies with the key.
    virt.key(0, KEY_A, 0);
    harness.queue.flush().expect("the release sends");
    harness.pump(Duration::from_millis(500), |_| false);
    assert_eq!(
        harness.state.repeats, held,
        "no tick after the release"
    );
    assert!(!harness.state.seat.repeat_armed());
    // Hold again, then lose focus mid-hold: a second client maps a
    // window and takes the seat's focus (what the launcher sees when
    // another app opens). The unmap's leave ends the repeat: the release
    // never arrives for a key held across the loss.
    virt.key(0, KEY_A, 1);
    harness.queue.flush().expect("the second press sends");
    let held_at = harness.state.repeats;
    harness.expect("repeating again", Duration::from_secs(5), |state| {
        state.repeats > held_at
    });
    let mut thief = Thief::map(&harness.session);
    harness.expect("the leave", PATIENCE, |state| {
        !state.seat.has_focus()
    });
    let after_leave = harness.state.repeats;
    harness.pump(Duration::from_millis(500), |_| false);
    assert_eq!(
        harness.state.repeats, after_leave,
        "no tick after focus loss"
    );
    assert!(
        !harness.state.seat.repeat_armed(),
        "focus loss disarms the timer"
    );
    drop(thief);
    harness.state.virt_keyboard = Some(virt);
}

fn memfd_with_bytes(contents: &[u8]) -> std::fs::File {
    let fd = rustix::fs::memfd_create(c"virt-keymap", rustix::fs::MemfdFlags::CLOEXEC)
        .expect("memfd_create works in tests");
    let mut file = std::fs::File::from(fd);
    file.write_all(contents).expect("the fixture writes");
    file.seek(SeekFrom::Start(0)).expect("rewind works");
    file
}

/// A second client that maps one window: its only job is taking the
/// seat's focus, so the first client's `leave` path runs. No keyboard
/// handling of its own.
struct Thief {
    queue: wayland_client::EventQueue<ThiefState>,
    qh: QueueHandle<ThiefState>,
    state: ThiefState,
    // Held so the window stays mapped until the test ends.
    _surface: wl_surface::WlSurface,
    _xdg_surface: xdg_surface::XdgSurface,
    _toplevel: xdg_toplevel::XdgToplevel,
}

struct ThiefState {
    compositor: Option<wl_compositor::WlCompositor>,
    xdg_base: Option<xdg_wm_base::XdgWmBase>,
    configured: bool,
}

macro_rules! thief_empty_dispatch {
    ($object:ty) => {
        impl Dispatch<$object, ()> for ThiefState {
            fn event(
                _: &mut ThiefState,
                _: &$object,
                _: <$object as Proxy>::Event,
                _: &(),
                _: &Connection,
                _: &QueueHandle<ThiefState>,
            ) {
            }
        }
    };
}

thief_empty_dispatch!(wl_compositor::WlCompositor);
thief_empty_dispatch!(wl_surface::WlSurface);
thief_empty_dispatch!(xdg_toplevel::XdgToplevel);

impl Dispatch<wl_registry::WlRegistry, ()> for ThiefState {
    fn event(
        state: &mut ThiefState,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<ThiefState>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "wl_compositor" => {
                    state.compositor = Some(registry.bind(name, 6.min(version), qh, ()));
                }
                "xdg_wm_base" => {
                    state.xdg_base = Some(registry.bind(name, 1.min(version), qh, ()));
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for ThiefState {
    fn event(
        _: &mut ThiefState,
        proxy: &xdg_wm_base::XdgWmBase,
        event: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<ThiefState>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            proxy.pong(serial);
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for ThiefState {
    fn event(
        state: &mut ThiefState,
        proxy: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<ThiefState>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            proxy.ack_configure(serial);
            state.configured = true;
        }
    }
}

impl Thief {
    fn map(session: &Session) -> Self {
        fn pump(queue: &mut wayland_client::EventQueue<ThiefState>, state: &mut ThiefState) {
            queue.flush().expect("flush works");
            queue.dispatch_pending(state).expect("dispatch works");
            if let Some(guard) = queue.prepare_read() {
                let fd = guard.connection_fd();
                let timespec = rustix::time::Timespec {
                    tv_sec: 5,
                    tv_nsec: 0,
                };
                let mut polls =
                    [rustix::event::PollFd::new(&fd, rustix::event::PollFlags::IN)];
                let _ = rustix::event::poll(&mut polls, Some(&timespec));
                drop(polls);
                let _ = guard.read();
            }
        }

        let stream =
            UnixStream::connect(session.wayland_socket()).expect("a second client connects");
        let conn = Connection::from_socket(stream).expect("a second connection makes");
        let mut queue = conn.new_event_queue();
        let qh = queue.handle();
        let mut state = ThiefState {
            compositor: None,
            xdg_base: None,
            configured: false,
        };
        conn.display().get_registry(&qh, ());
        let deadline = Instant::now() + PATIENCE;
        while !(state.compositor.is_some() && state.xdg_base.is_some()) {
            assert!(Instant::now() < deadline, "the thief binds");
            pump(&mut queue, &mut state);
        }
        let surface = state
            .compositor
            .as_ref()
            .expect("a compositor")
            .create_surface(&qh, ());
        let xdg_surface = state
            .xdg_base
            .as_ref()
            .expect("an xdg base")
            .get_xdg_surface(&surface, &qh, ());
        let toplevel = xdg_surface.get_toplevel(&qh, ());
        surface.commit();
        let deadline = Instant::now() + PATIENCE;
        while !state.configured {
            assert!(Instant::now() < deadline, "the thief configures");
            pump(&mut queue, &mut state);
        }
        Self {
            queue,
            qh,
            state,
            _surface: surface,
            _xdg_surface: xdg_surface,
            _toplevel: toplevel,
        }
    }
}
