//! The workspaces module on headless scoot with two outputs: each output's
//! numbers with the active one in a pill, the pill following a switch, a
//! click switching, and workspaces appearing and disappearing as windows
//! come and go. (A manager that finishes mid-batch is never half-drawn and
//! a `done`-less batch never redraws: those go through the harness in
//! `src/modules/workspaces/tests.rs`, where the protocol side is injected
//! as data. Hotplug on sway is below; scoot's headless outputs are fixed at
//! start-up.)
//!
//! The bar draws with the seven-segment test font (`src/testfont.rs`,
//! passed as `--font` by the harness), 50 pixels to the em on a 60-pixel
//! bar, so the numbers are read back by sampling their segments
//! ([`common::testfont::decode`]) and the pill by its accent color.
//!
//! There is no window client on the test machines (no `foot`), so the
//! windows these tests switch between are mapped by a tiny `xdg_toplevel`
//! client that runs in a helper thread ([`Toy`]).

// Most tests here place the workspaces module (gated per test); the bind
// tests also place the clock, and one needs only the clock. This file
// exists where either does (dev/research/scootbar-testing.md: the feature matrix);
// helpers used only by workspaces tests carry that gate too.
#![cfg(any(feature = "clock", feature = "workspaces"))]

mod common;

use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use common::{Reaper, Session};
#[cfg(feature = "workspaces")]
use common::{Shot, rgb, testfont};
use rustix::event::{PollFd, PollFlags};
use rustix::time::Timespec;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_client::protocol::wl_compositor::WlCompositor;
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_shm::{self, WlShm};
use wayland_client::protocol::wl_shm_pool::WlShmPool;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, QueueHandle, delegate_noop};
use wayland_protocols::xdg::shell::client::xdg_surface::{self, XdgSurface};
use wayland_protocols::xdg::shell::client::xdg_toplevel::{self, XdgToplevel};
use wayland_protocols::xdg::shell::client::xdg_wm_base::{self, XdgWmBase};

#[cfg(feature = "workspaces")]
const BAR: &str = "#102030";
#[cfg(feature = "workspaces")]
const FG: &str = "#f0f0f0";
#[cfg(feature = "workspaces")]
const ACCENT: &str = "#f9e2af";
#[cfg(feature = "workspaces")]
const HEIGHT: u32 = 60;
#[cfg(feature = "workspaces")]
const EM: u32 = 50;
/// A 50-pixel line centered in 60 rows: 5 above, baseline at 45.
#[cfg(feature = "workspaces")]
const BASELINE: i64 = 45;

/// The workspaces module's flags: on the left, nothing else placed.
#[cfg(feature = "workspaces")]
fn workspaces_args() -> Vec<&'static str> {
    vec![
        "--background",
        BAR,
        "--foreground",
        FG,
        "--height",
        "60",
        "--font-size",
        "50",
        "--left",
        "workspaces",
    ]
}

/// Which columns hold the pill's accent color.
#[cfg(feature = "workspaces")]
fn pill_cols(shot: &Shot) -> Vec<bool> {
    let accent = rgb(ACCENT);
    (0..shot.width)
        .map(|x| (0..HEIGHT).any(|y| shot.at(x, y) == accent))
        .collect()
}

/// Whether `(x, y)` is text: a text-colored pixel anywhere, or a
/// background-colored pixel inside a pill column (the active number is
/// drawn in the bar's background on the pill).
#[cfg(feature = "workspaces")]
fn is_ink(shot: &Shot, pills: &[bool], x: i64, y: i64) -> bool {
    if x < 0 || y < 0 || (x as u32) >= shot.width || (y as u32) >= HEIGHT {
        return false;
    }
    let pixel = shot.at(x as u32, y as u32);
    if pixel != rgb(BAR) && pixel != rgb(ACCENT) {
        return true;
    }
    pixel == rgb(BAR) && pills[x as usize]
}

/// The bar's text read back from the top `HEIGHT` rows of `shot`, spaces
/// left out (`decode` skips them); empty when there is no ink yet.
#[cfg(feature = "workspaces")]
fn read(shot: &Shot) -> String {
    let pills = pill_cols(shot);
    let ink = |x: i64, y: i64| is_ink(shot, &pills, x, y);
    let columns: Vec<i64> = (0..i64::from(shot.width))
        .filter(|&x| (0..i64::from(HEIGHT)).any(|y| ink(x, y)))
        .collect();
    let (Some(&left), Some(&right)) = (columns.first(), columns.last()) else {
        return String::new();
    };
    testfont::decode(ink, left, right, BASELINE, f64::from(EM))
}

/// The contiguous ink column groups across the bar's rows: one per digit
/// (spaces split them). The `n`th group's middle is where a click on the
/// `n`th workspace lands, inside its pill.
#[cfg(feature = "workspaces")]
fn groups(shot: &Shot) -> Vec<(u32, u32)> {
    let pills = pill_cols(shot);
    let mut groups = Vec::new();
    let mut start = None;
    for x in 0..shot.width {
        let inked = (0..HEIGHT).any(|y| is_ink(shot, &pills, x as i64, y as i64));
        match (inked, start) {
            (true, None) => start = Some(x),
            (false, Some(first)) => {
                groups.push((first, x - 1));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(first) = start {
        groups.push((first, shot.width - 1));
    }
    groups
}

/// The columns holding the pill's accent color, if any.
#[cfg(feature = "workspaces")]
fn pill(shot: &Shot) -> Option<(u32, u32)> {
    let pills = pill_cols(shot);
    let mut first = None;
    let mut last = 0;
    for (x, &held) in pills.iter().enumerate() {
        if held {
            first.get_or_insert(x as u32);
            last = x as u32;
        }
    }
    first.map(|first| (first, last))
}

/// Whether the pill covers the `n`th digit group and no other.
#[cfg(feature = "workspaces")]
fn pill_on(shot: &Shot, n: usize) -> bool {
    let (Some((lo, hi)), groups) = (pill(shot), groups(shot)) else {
        return false;
    };
    groups.len() > n
        && lo <= groups[n].0
        && hi >= groups[n].1
        && groups
            .iter()
            .enumerate()
            .all(|(m, &(first, last))| m == n || hi < first || lo > last)
}

/// A 64x64 `xdg_toplevel` window: mapped while its thread runs, gone when
/// it stops. The compositor tiles it; the tests never look at it, only at
/// what its workspace does to the bar.
struct Toy {
    surface: WlSurface,
    buffer: WlBuffer,
    /// Kept open: the buffer reads from it.
    _file: std::fs::File,
    attached: bool,
}

impl Dispatch<WlRegistry, GlobalListContents> for Toy {
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

impl Dispatch<XdgWmBase, ()> for Toy {
    fn event(
        _: &mut Self,
        wm_base: &XdgWmBase,
        event: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm_base.pong(serial);
        }
    }
}

impl Dispatch<XdgSurface, ()> for Toy {
    fn event(
        state: &mut Self,
        xdg_surface: &XdgSurface,
        event: xdg_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            xdg_surface.ack_configure(serial);
            if !state.attached {
                // The first commit went out with no buffer (the compositor
                // refuses a buffer before the initial configure is acked);
                // attach it now.
                state.surface.attach(Some(&state.buffer), 0, 0);
                state.attached = true;
            }
            state.surface.commit();
        }
    }
}

impl Dispatch<XdgToplevel, ()> for Toy {
    fn event(
        _: &mut Self,
        _: &XdgToplevel,
        event: xdg_toplevel::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let _ = event;
    }
}

delegate_noop!(Toy: ignore WlCompositor);
delegate_noop!(Toy: ignore WlShm);
delegate_noop!(Toy: ignore WlShmPool);
delegate_noop!(Toy: ignore WlSurface);
delegate_noop!(Toy: ignore WlBuffer);

/// A mapped window on `socket`, gone on [`ToyWindow::close`].
struct ToyWindow {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl ToyWindow {
    fn open(socket: PathBuf) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let stop = stop.clone();
            thread::spawn(move || toy_main(&socket, &stop))
        };
        Self {
            stop,
            thread: Some(thread),
        }
    }

    fn close(mut self, session: &Session, bar: &mut Reaper) {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.take().unwrap().join().unwrap();
        session.wait_for(&mut bar.0, "the window closed", |session| {
            let reply = session.scoot_ipc(r#"{"type":"windows"}"#);
            reply["windows"].as_array()?.is_empty().then_some(())
        });
    }
}

fn toy_main(socket: &PathBuf, stop: &AtomicBool) {
    let stream = UnixStream::connect(socket).expect("toy: no compositor socket");
    let conn = Connection::from_socket(stream).expect("toy: no connection");
    let (globals, mut queue) = registry_queue_init::<Toy>(&conn).expect("toy: no globals");
    let qh = queue.handle();
    let compositor: WlCompositor = globals.bind(&qh, 4..=6, ()).expect("toy: no compositor");
    let shm: WlShm = globals.bind(&qh, 1..=1, ()).expect("toy: no shm");
    let wm_base: XdgWmBase = globals.bind(&qh, 1..=3, ()).expect("toy: no xdg_wm_base");
    let surface = compositor.create_surface(&qh, ());
    let xdg_surface = wm_base.get_xdg_surface(&surface, &qh, ());
    let toplevel = xdg_surface.get_toplevel(&qh, ());
    toplevel.set_title("toy".to_owned());
    // One small buffer: the window is never looked at.
    let len: u64 = 64 * 64 * 4;
    let fd = rustix::fs::memfd_create(c"toy", rustix::fs::MemfdFlags::CLOEXEC).unwrap();
    rustix::fs::ftruncate(&fd, len).unwrap();
    let mut file = std::fs::File::from(fd);
    std::io::Write::write_all(&mut file, &vec![0x33u8; len as usize]).unwrap();
    let pool = shm.create_pool(file.as_fd(), len as i32, &qh, ());
    let buffer = pool.create_buffer(0, 64, 64, 64 * 4, wl_shm::Format::Xrgb8888, &qh, ());
    // No buffer yet: the compositor sends the initial configure for the
    // empty commit, which the loop acks before attaching anything.
    surface.commit();
    conn.flush().unwrap();
    let mut toy = Toy {
        surface,
        buffer,
        _file: file,
        attached: false,
    };
    let timeout = Timespec {
        tv_sec: 0,
        tv_nsec: 50_000_000,
    };
    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        if conn.flush().is_err() {
            break;
        }
        let Some(guard) = queue.prepare_read() else {
            // Events arrived meanwhile: dispatch them.
            if queue.dispatch_pending(&mut toy).is_err() {
                break;
            }
            continue;
        };
        let mut fds = [PollFd::from_borrowed_fd(
            guard.connection_fd(),
            PollFlags::IN,
        )];
        if rustix::event::poll(&mut fds, Some(&timeout)).is_err() {
            drop(guard);
            continue;
        }
        let readable = fds[0]
            .revents()
            .intersects(PollFlags::IN | PollFlags::HUP | PollFlags::ERR);
        if readable {
            if guard.read().is_err() {
                break;
            }
        } else {
            drop(guard);
        }
        if queue.dispatch_pending(&mut toy).is_err() {
            break;
        }
    }
}

/// Starts scoot with two outputs and a workspaces bar. Each output shows
/// its own single workspace, active, in a pill.
#[cfg(feature = "workspaces")]
fn two_outputs(tag: &str) -> Option<(Session, Reaper)> {
    let session = Session::scoot(tag, 2, "")?;
    let mut bar = Reaper(session.bar(&workspaces_args()));
    for id in [1, 2] {
        session.wait_for(&mut bar.0, "the workspace shown", |session| {
            let shot = session.scoot_screenshot(id);
            (read(&shot) == "1" && pill_on(&shot, 0)).then_some(())
        });
    }
    assert!(session.bar_stderr().is_empty(), "{}", session.bar_stderr());
    Some((session, bar))
}

#[cfg(feature = "workspaces")]
#[test]
fn each_output_shows_its_own_workspace_active_in_a_pill() {
    let Some((_session, _bar)) = two_outputs("ws-each") else {
        return;
    };
}

#[cfg(feature = "workspaces")]
#[test]
fn the_pill_follows_a_switch() {
    let Some((session, mut bar)) = two_outputs("ws-switch") else {
        return;
    };
    // A window on the focused output's workspace grows its list with the
    // trailing empty workspace.
    let toy = ToyWindow::open(session.wayland_socket());
    session.wait_for(&mut bar.0, "a window mapped", |session| {
        let reply = session.scoot_ipc(r#"{"type":"windows"}"#);
        (!reply["windows"].as_array()?.is_empty()).then_some(())
    });
    session.wait_for(&mut bar.0, "the grown list", |session| {
        (read(&session.scoot_screenshot(1)) == "12").then_some(())
    });
    // To the trailing empty workspace: the pill follows it there.
    let reply =
        session.scoot_ipc(r#"{"type":"action","action":"focus_workspace_index","index":1}"#);
    assert_eq!(reply["type"], "ok", "{reply}");
    session.wait_for(&mut bar.0, "the pill followed", |session| {
        let shot = session.scoot_screenshot(1);
        (read(&shot) == "12" && pill_on(&shot, 1)).then_some(())
    });
    // The other output is untouched.
    let shot = session.scoot_screenshot(2);
    assert_eq!(read(&shot), "1", "output 2 moved too");
    assert!(pill_on(&shot, 0));
    toy.close(&session, &mut bar);
}

#[cfg(feature = "workspaces")]
#[test]
fn a_click_switches() {
    let Some((session, mut bar)) = two_outputs("ws-click") else {
        return;
    };
    let toy = ToyWindow::open(session.wayland_socket());
    session.wait_for(&mut bar.0, "a window mapped", |session| {
        let reply = session.scoot_ipc(r#"{"type":"windows"}"#);
        (!reply["windows"].as_array()?.is_empty()).then_some(())
    });
    session.wait_for(&mut bar.0, "the grown list", |session| {
        (read(&session.scoot_screenshot(1)) == "12").then_some(())
    });
    let reply =
        session.scoot_ipc(r#"{"type":"action","action":"focus_workspace_index","index":1}"#);
    assert_eq!(reply["type"], "ok", "{reply}");
    session.wait_for(&mut bar.0, "workspace 2 active", |session| {
        let shot = session.scoot_screenshot(1);
        (read(&shot) == "12" && pill_on(&shot, 1)).then_some(())
    });
    // Click the middle of workspace 1's digit, mid-bar: global pixels, the
    // first output starting at x 0.
    let shot = session.scoot_screenshot(1);
    let (lo, hi) = groups(&shot)[0];
    let x = (lo + hi) / 2;
    let reply = session.scoot_ipc(&format!(
        r#"{{"type":"click","x":{x},"y":{},"button":"left"}}"#,
        HEIGHT / 2
    ));
    assert_eq!(reply["type"], "ok", "{reply}");
    session.wait_for(&mut bar.0, "the click switched", |session| {
        let shot = session.scoot_screenshot(1);
        (read(&shot) == "12" && pill_on(&shot, 0)).then_some(())
    });
    toy.close(&session, &mut bar);
}

#[cfg(feature = "workspaces")]
#[test]
fn appearing_and_disappearing_redraw() {
    let Some((session, mut bar)) = two_outputs("ws-appear") else {
        return;
    };
    let toy = ToyWindow::open(session.wayland_socket());
    session.wait_for(&mut bar.0, "the workspace appeared", |session| {
        (read(&session.scoot_screenshot(1)) == "12").then_some(())
    });
    toy.close(&session, &mut bar);
    session.wait_for(&mut bar.0, "the workspace disappeared", |session| {
        let shot = session.scoot_screenshot(1);
        (read(&shot) == "1" && pill_on(&shot, 0)).then_some(())
    });
    // Opening windows one after another never shows a half-drawn list: the
    // text is always one of the whole states.
    for _ in 0..3 {
        let toy = ToyWindow::open(session.wayland_socket());
        session.wait_for(&mut bar.0, "grown again", |session| {
            (read(&session.scoot_screenshot(1)) == "12").then_some(())
        });
        toy.close(&session, &mut bar);
        session.wait_for(&mut bar.0, "shrunk again", |session| {
            (read(&session.scoot_screenshot(1)) == "1").then_some(())
        });
    }
    // Nothing was said on stderr throughout.
    assert!(session.bar_stderr().is_empty(), "{}", session.bar_stderr());
}

/// On sway: the workspaces on an output plugged in while the bar runs.
#[cfg(feature = "workspaces")]
#[test]
fn workspaces_follow_a_hotplugged_output_on_sway() {
    let Some(session) = Session::sway("swayws", 1) else {
        return;
    };
    let mut bar = Reaper(session.bar(&workspaces_args()));
    let shows = |name: &str| !read(&session.screencopy(name)).is_empty();
    let first = session.wait_for(&mut bar.0, "outputs", |session| {
        let names = session.swaymsg(&["-t", "get_outputs", "-r"]);
        let value: serde_json::Value = serde_json::from_str(&names).unwrap();
        value.as_array()?.first()?["name"]
            .as_str()
            .map(str::to_owned)
    });
    session.wait_for(&mut bar.0, "workspaces on the first output", |_| {
        shows(&first).then_some(())
    });
    session.swaymsg(&["create_output"]);
    let second = session.wait_for(&mut bar.0, "a second output", |session| {
        let names = session.swaymsg(&["-t", "get_outputs", "-r"]);
        let value: serde_json::Value = serde_json::from_str(&names).unwrap();
        let list = value.as_array()?;
        (list.len() == 2).then(|| list[1]["name"].as_str().map(str::to_owned))?
    });
    session.wait_for(&mut bar.0, "workspaces on the new output", |_| {
        shows(&second).then_some(())
    });
    std::thread::sleep(Duration::from_millis(200));
    assert!(session.bar_stderr().is_empty(), "{}", session.bar_stderr());
}

/// How many rows of column `x` hold the pill's accent.
#[cfg(feature = "workspaces")]
fn accent_rows(shot: &Shot, x: u32) -> u32 {
    (0..HEIGHT)
        .filter(|&y| shot.at(x, y) == rgb(ACCENT))
        .count() as u32
}

/// The columns holding text-colored pixels (an inactive number).
#[cfg(feature = "workspaces")]
fn text_cols(shot: &Shot) -> Vec<u32> {
    (0..shot.width)
        .filter(|&x| (0..HEIGHT).any(|y| shot.at(x, y) == rgb(FG)))
        .collect()
}

/// `[workspaces] pill-shape = "circle"` round through the real bar, and a
/// click on another workspace's digit still switches to it: the hit test
/// follows the shape the draw paints. (`read` is not used: it takes the
/// background-colored corners inside a round pill's columns for ink.)
#[cfg(feature = "workspaces")]
#[test]
fn a_circle_pill_is_round_and_a_click_still_switches() {
    let Some(session) = Session::scoot("ws-circle", 1, "") else {
        return;
    };
    let path = session.runtime_dir().join("circle.toml");
    std::fs::write(
        &path,
        "[bar]\npadding = 16\n[workspaces]\npill-shape = \"circle\"\n",
    )
    .unwrap();
    let mut args = workspaces_args();
    let path = path.to_string_lossy().into_owned();
    args.extend(["--center=", "--config", &path]);
    let mut bar = Reaper(session.bar(&args));
    let shot = session.wait_for(&mut bar.0, "the workspace shown", |session| {
        let shot = session.scoot_screenshot(1);
        pill(&shot).map(|_| shot)
    });
    // The pill is a disc: its leftmost column holds only a few rows of
    // accent (a square pill holds all 60; the padding is wide enough for the
    // module's span to hold a 60-wide disc), it reaches the bar's top and
    // bottom in the middle, and it
    // is at least as wide as it is tall.
    let (lo, hi) = pill(&shot).unwrap();
    let edge = accent_rows(&shot, lo);
    assert!(edge < HEIGHT / 2, "{edge} accent rows at the pill's edge");
    let mid = (lo + hi) / 2;
    // (Within a few levels of the accent: the disc's top row is antialiased.)
    for (y, what) in [(0, "top"), (HEIGHT - 1, "bottom")] {
        for (got, want) in shot.at(mid, y).iter().zip(rgb(ACCENT)) {
            assert!(
                got.abs_diff(want) <= 8,
                "the pill's {what}: {:?}",
                shot.at(mid, y)
            );
        }
    }
    assert!(hi - lo + 1 >= HEIGHT - 2, "{lo}..{hi}");

    let toy = ToyWindow::open(session.wayland_socket());
    session.wait_for(&mut bar.0, "a window mapped", |session| {
        let reply = session.scoot_ipc(r#"{"type":"windows"}"#);
        (!reply["windows"].as_array()?.is_empty()).then_some(())
    });
    // Two workspaces: the inactive number is text-colored.
    session.wait_for(&mut bar.0, "the grown list", |session| {
        (!text_cols(&session.scoot_screenshot(1)).is_empty()).then_some(())
    });
    let reply =
        session.scoot_ipc(r#"{"type":"action","action":"focus_workspace_index","index":1}"#);
    assert_eq!(reply["type"], "ok", "{reply}");
    // Workspace 2 active: the pill is right of the text-colored digit.
    let shot = session.wait_for(&mut bar.0, "workspace 2 active", |session| {
        let shot = session.scoot_screenshot(1);
        let text = text_cols(&shot);
        let (lo, _) = pill(&shot)?;
        (!text.is_empty() && text.iter().all(|&x| x < lo)).then_some(shot)
    });
    // Workspace 1's digit is the only text-colored ink: click its middle.
    let text = text_cols(&shot);
    let x = (text[0] + text[text.len() - 1]) / 2;
    let reply = session.scoot_ipc(&format!(
        r#"{{"type":"click","x":{x},"y":{},"button":"left"}}"#,
        HEIGHT / 2
    ));
    assert_eq!(reply["type"], "ok", "{reply}");
    // Workspace 1 active: the pill now covers where the click landed.
    session.wait_for(&mut bar.0, "the click switched", |session| {
        let (lo, hi) = pill(&session.scoot_screenshot(1))?;
        (lo <= x && x <= hi).then_some(())
    });
    toy.close(&session, &mut bar);
}

// What the bar holds while no workspaces module is placed: a bar that does
// not show workspaces must not be woken for them, so it must not bind
// `ext_workspace_manager_v1` (or the seat, which only that module's clicks
// need), and a reload adding or removing the module binds and releases them
// live. Read from the bar's own `WAYLAND_DEBUG` trace.

/// How many times the trace binds `interface` from the registry.
#[cfg(feature = "clock")]
fn bound(trace: &str, interface: &str) -> usize {
    trace
        .lines()
        .filter(|line| {
            line.contains("-> wl_registry@") && line.contains(&format!("\"{interface}\""))
        })
        .count()
}

/// The trace lines that are the compositor's events on a workspace
/// protocol object (the manager, a group or a workspace): `<-` marks an
/// event in the trace.
#[cfg(feature = "clock")]
fn workspace_events(trace: &str) -> Vec<&str> {
    trace
        .lines()
        .filter(|line| {
            line.contains("<- ")
                && [
                    "ext_workspace_manager_v1@",
                    "ext_workspace_group_handle_v1@",
                    "ext_workspace_handle_v1@",
                ]
                .iter()
                .any(|object| line.contains(object))
        })
        .collect()
}

/// A config file placing `left` in the session's font, a clock in the
/// center, and the bar 28 high.
#[cfg(feature = "clock")]
fn placing(session: &Session, left: &str) -> PathBuf {
    let path = session.runtime_dir().join("binds.toml");
    std::fs::write(
        &path,
        format!(
            "left = [{left}]\ncenter = [\"clock\"]\n\n[bar]\nheight = 28\nfont = \"{}\"\n",
            session.font().display()
        ),
    )
    .unwrap();
    path
}

/// `scootbar daemon --config PATH` with a protocol trace in the bar log.
#[cfg(feature = "clock")]
fn traced_daemon(session: &Session, path: &std::path::Path) -> Reaper {
    let log = std::fs::File::create(session.bar_log()).unwrap();
    let child = session
        .scootbar()
        .arg("daemon")
        .arg("--config")
        .arg(path)
        .env("WAYLAND_DEBUG", "1")
        .stdout(std::process::Stdio::null())
        .stderr(log)
        .spawn()
        .unwrap();
    Reaper(child)
}

#[cfg(all(feature = "clock", feature = "workspaces"))]
fn reload(session: &Session) {
    let out = session.scootbar().args(["msg", "reload"]).output().unwrap();
    assert!(
        out.status.success(),
        "reload: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Opens a window, switches to the workspace after it and closes the
/// window: a batch of workspace changes a bound manager would be sent.
#[cfg(feature = "clock")]
fn churn(session: &Session, bar: &mut Reaper) {
    let toy = ToyWindow::open(session.wayland_socket());
    session.wait_for(&mut bar.0, "a window mapped", |session| {
        let reply = session.scoot_ipc(r#"{"type":"windows"}"#);
        (!reply["windows"].as_array()?.is_empty()).then_some(())
    });
    let reply =
        session.scoot_ipc(r#"{"type":"action","action":"focus_workspace_index","index":1}"#);
    assert_eq!(reply["type"], "ok", "{reply}");
    toy.close(session, bar);
    // A round trip through the bar's own connection: what scoot sent has
    // been read by now.
    std::thread::sleep(Duration::from_millis(300));
}

#[cfg(feature = "clock")]
#[test]
fn a_bar_without_the_module_binds_no_workspace_protocol_and_hears_nothing() {
    let Some(session) = Session::scoot("ws-unplaced", 1, "") else {
        return;
    };
    let path = placing(&session, "");
    let mut bar = traced_daemon(&session, &path);
    session.wait_for(&mut bar.0, "the bar up", |session| {
        session.bar_stderr().contains(".configure,").then_some(())
    });
    churn(&session, &mut bar);
    let trace = session.bar_stderr();
    assert!(trace.contains("\"wl_output\""), "no trace: {trace}");
    assert_eq!(bound(&trace, "ext_workspace_manager_v1"), 0);
    assert_eq!(bound(&trace, "wl_seat"), 0);
    let events = workspace_events(&trace);
    assert!(events.is_empty(), "{events:?}");
}

#[cfg(all(feature = "clock", feature = "workspaces"))]
#[test]
fn a_bar_with_the_module_binds_the_protocol_once_and_hears_the_changes() {
    let Some(session) = Session::scoot("ws-placed", 1, "") else {
        return;
    };
    let path = placing(&session, "\"workspaces\"");
    let mut bar = traced_daemon(&session, &path);
    session.wait_for(&mut bar.0, "the first batch", |session| {
        session.bar_stderr().contains(".done, ").then_some(())
    });
    churn(&session, &mut bar);
    let trace = session.bar_stderr();
    assert_eq!(bound(&trace, "ext_workspace_manager_v1"), 1);
    assert_eq!(bound(&trace, "wl_seat"), 1);
    assert!(workspace_events(&trace).len() > 4);
}

#[cfg(all(feature = "clock", feature = "workspaces"))]
#[test]
fn a_reload_binds_and_releases_the_protocol_with_the_module() {
    let Some(session) = Session::scoot("ws-reload", 1, "") else {
        return;
    };
    let mut bar = traced_daemon(&session, &placing(&session, ""));
    session.wait_for(&mut bar.0, "the bar up", |session| {
        session.bar_stderr().contains(".configure,").then_some(())
    });
    let pid = bar.0.id();
    let fds = common::settled_fds(pid);
    assert_eq!(bound(&session.bar_stderr(), "ext_workspace_manager_v1"), 0);

    // Added: bound now, and the first batch arrives.
    placing(&session, "\"workspaces\"");
    reload(&session);
    session.wait_for(&mut bar.0, "the manager's first batch", |session| {
        session.bar_stderr().contains(".done, ").then_some(())
    });
    let trace = session.bar_stderr();
    assert_eq!(bound(&trace, "ext_workspace_manager_v1"), 1, "{trace}");
    assert_eq!(bound(&trace, "wl_seat"), 1);

    // Removed: stopped and released, every handle destroyed, and the
    // descriptors back to where they were.
    placing(&session, "");
    reload(&session);
    session.wait_for(&mut bar.0, "the manager stopped", |session| {
        let trace = session.bar_stderr();
        (trace.contains(".stop()") && trace.contains(".finished, ")).then_some(())
    });
    let trace = session.bar_stderr();
    assert!(
        trace
            .lines()
            .any(|l| l.contains("-> ext_workspace_handle_v1@") && l.contains(".destroy()"))
    );
    assert!(
        trace
            .lines()
            .any(|l| l.contains("-> wl_seat@") && l.contains(".release()")),
        "the seat was not released"
    );
    assert_eq!(common::settled_fds(pid), fds, "a release leaked fds");
    // Nothing more is heard: whatever comes after the stop is `finished`
    // and the handles that raced it.
    let stopped = trace.find(".stop()").unwrap();
    let mark = session.bar_stderr().len();
    churn(&session, &mut bar);
    let after = session.bar_stderr();
    assert!(
        workspace_events(&after[mark.max(stopped)..])
            .iter()
            .all(|line| line.contains(".finished, ")),
        "{}",
        &after[mark..]
    );

    // Added again: a fresh bind, working.
    placing(&session, "\"workspaces\"");
    reload(&session);
    session.wait_for(&mut bar.0, "the second bind", |session| {
        (bound(&session.bar_stderr(), "ext_workspace_manager_v1") == 2).then_some(())
    });
    churn(&session, &mut bar);
    // Alive throughout and never a protocol error: the bar still answers.
    assert!(bar.0.try_wait().unwrap().is_none());
    assert!(
        !session.bar_stderr().contains("error"),
        "{}",
        session.bar_stderr()
    );
    assert_eq!(common::settled_fds(pid), fds, "a second bind leaked fds");
}
