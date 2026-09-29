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

mod common;

use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use common::{Reaper, Session, Shot, rgb, testfont};
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

const BAR: &str = "#102030";
const FG: &str = "#f0f0f0";
const ACCENT: &str = "#f9e2af";
const HEIGHT: u32 = 60;
const EM: u32 = 50;
/// A 50-pixel line centered in 60 rows: 5 above, baseline at 45.
const BASELINE: i64 = 45;

/// The workspaces module's flags: on the left, nothing else placed.
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
fn pill_cols(shot: &Shot) -> Vec<bool> {
    let accent = rgb(ACCENT);
    (0..shot.width)
        .map(|x| (0..HEIGHT).any(|y| shot.at(x, y) == accent))
        .collect()
}

/// Whether `(x, y)` is text: a text-colored pixel anywhere, or a
/// background-colored pixel inside a pill column (the active number is
/// drawn in the bar's background on the pill).
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

#[test]
fn each_output_shows_its_own_workspace_active_in_a_pill() {
    let Some((_session, _bar)) = two_outputs("ws-each") else {
        return;
    };
}

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
