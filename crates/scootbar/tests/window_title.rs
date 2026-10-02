//! The window-title module on headless scoot with two outputs: the title
//! follows focus across outputs, a click activates, `invoke close` is
//! gated by `allow-close`, control characters and CJK render safely, a
//! title flood costs a bounded redraw rate, and a long title is cut to
//! `max-width` with an ellipsis. (Batches, pills and hit tests go through
//! the harness in `src/modules/window_title/tests.rs`, where the protocol
//! side is injected as data.)
//!
//! There is no window client on the test machines (no `foot`), so the
//! windows these tests focus are mapped by a tiny `xdg_toplevel` client
//! that runs in a helper thread ([`Toy`]), with its title settable from
//! the test.

mod common;

use std::io::{BufRead, BufReader};
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use common::{Reaper, Session, rgb};
use rustix::event::{PollFd, PollFlags};
use rustix::time::Timespec;
use serde_json::Value;
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
const HEIGHT: u32 = 40;

/// A mapped `xdg_toplevel` window, retitleable from the test: mapped while
/// its thread runs, gone on [`ToyWindow::close`].
enum ToyCmd {
    Title(String),
    Stop,
}

struct Toy {
    surface: WlSurface,
    xdg_toplevel: XdgToplevel,
    buffer: WlBuffer,
    /// Kept open: the buffer reads from it.
    _file: std::fs::File,
    attached: bool,
    inbox: Receiver<ToyCmd>,
    /// The compositor asked this window to close.
    gone: bool,
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
                state.surface.attach(Some(&state.buffer), 0, 0);
                state.attached = true;
            }
            state.surface.commit();
        }
    }
}

impl Dispatch<XdgToplevel, ()> for Toy {
    fn event(
        state: &mut Self,
        _: &XdgToplevel,
        event: xdg_toplevel::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // Asked to close: go away, like a client that honors it, so the
        // window is gone once the connection drops.
        if let xdg_toplevel::Event::Close = event {
            state.gone = true;
        }
    }
}

delegate_noop!(Toy: ignore WlCompositor);
delegate_noop!(Toy: ignore WlShm);
delegate_noop!(Toy: ignore WlShmPool);
delegate_noop!(Toy: ignore WlSurface);
delegate_noop!(Toy: ignore WlBuffer);

/// A mapped window on `socket`, gone on [`ToyWindow::close`].
struct ToyWindow {
    tx: Sender<ToyCmd>,
    thread: Option<JoinHandle<()>>,
}

impl ToyWindow {
    fn open(socket: PathBuf, title: &str) -> Self {
        let (tx, inbox) = mpsc::channel();
        let title = title.to_owned();
        let thread = thread::spawn(move || toy_main(&socket, title, inbox));
        Self {
            tx,
            thread: Some(thread),
        }
    }

    fn retitle(&self, title: &str) {
        self.tx.send(ToyCmd::Title(title.to_owned())).unwrap();
    }

    fn close(self, session: &Session, bar: &mut Reaper) {
        let before = window_ids(session).len();
        self.stop();
        session.wait_for(&mut bar.0, "the window closed", |session| {
            (window_ids(session).len() + 1 == before).then_some(())
        });
    }

    /// Stops the client without waiting for the compositor to agree (for a
    /// window the compositor already closed).
    fn stop(mut self) {
        // The thread may already have gone (asked to close): then there
        // is nothing to tell it.
        let _ = self.tx.send(ToyCmd::Stop);
        self.thread.take().unwrap().join().unwrap();
    }
}

fn toy_main(socket: &PathBuf, title: String, inbox: Receiver<ToyCmd>) {
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
    toplevel.set_title(title);
    let len: u64 = 64 * 64 * 4;
    let fd = rustix::fs::memfd_create(c"toy", rustix::fs::MemfdFlags::CLOEXEC).unwrap();
    rustix::fs::ftruncate(&fd, len).unwrap();
    let mut file = std::fs::File::from(fd);
    std::io::Write::write_all(&mut file, &vec![0x33u8; len as usize]).unwrap();
    let pool = shm.create_pool(file.as_fd(), len as i32, &qh, ());
    let buffer = pool.create_buffer(0, 64, 64, 64 * 4, wl_shm::Format::Xrgb8888, &qh, ());
    surface.commit();
    conn.flush().unwrap();
    let mut toy = Toy {
        surface,
        xdg_toplevel: toplevel,
        buffer,
        _file: file,
        attached: false,
        inbox,
        gone: false,
    };
    let timeout = Timespec {
        tv_sec: 0,
        tv_nsec: 50_000_000,
    };
    loop {
        while let Ok(cmd) = toy.inbox.try_recv() {
            match cmd {
                ToyCmd::Title(title) => {
                    toy.xdg_toplevel.set_title(title);
                    toy.surface.commit();
                }
                ToyCmd::Stop => return,
            }
        }
        if conn.flush().is_err() {
            break;
        }
        let Some(guard) = queue.prepare_read() else {
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
        // Asked to close: the connection going away takes the window.
        if toy.gone {
            break;
        }
    }
}

/// The bar's control assertions below go through `msg`/`json`.
fn msg(session: &Session, args: &[&str]) -> std::process::Output {
    session.scootbar().arg("msg").args(args).output().unwrap()
}

fn json(session: &Session, args: &[&str]) -> Value {
    let out = msg(session, args);
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("{args:?}: {e}: {}", String::from_utf8_lossy(&out.stdout)))
}

fn refused(session: &Session, args: &[&str]) -> String {
    let out = msg(session, args);
    assert!(!out.status.success(), "{args:?} was accepted");
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// Starts scoot with two outputs and a window-title bar on the left.
fn two_outputs(tag: &str, tables: &str) -> Option<(Session, Reaper)> {
    let session = Session::scoot(tag, 2, "")?;
    let file = session.runtime_dir().join("bar.toml");
    std::fs::write(
        &file,
        format!(
            "left = [\"window-title\"]\n[bar]\nheight = {HEIGHT}\n[colors]\nbackground = \"{BAR}\"\n{tables}"
        ),
    )
    .unwrap();
    let mut bar = Reaper(session.bar(&["--config", file.to_str().unwrap()]));
    // Both bars drawn before anything is asserted.
    for id in [1, 2] {
        session.wait_for(&mut bar.0, "the bar drawn", |session| {
            (session.scoot_screenshot(id).at(0, 0) == rgb(BAR)).then_some(())
        });
    }
    assert!(session.bar_stderr().is_empty(), "{}", session.bar_stderr());
    Some((session, bar))
}

/// The window-title entries of `query`: `(output, text)` per output that
/// places the module.
fn titles(session: &Session) -> Vec<(String, String)> {
    json(session, &["query", "window-title"])["modules"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            (
                entry["output"].as_str().unwrap_or("").to_owned(),
                entry["text"].as_str().unwrap_or("").to_owned(),
            )
        })
        .collect()
}

/// Output names in layout order.
fn output_names(session: &Session) -> Vec<String> {
    json(session, &["layout"])["outputs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|output| output["output"].as_str().unwrap_or("").to_owned())
        .collect()
}

/// The window ids scoot reports, in order.
fn window_ids(session: &Session) -> Vec<i64> {
    session.scoot_ipc(r#"{"type":"windows"}"#)["windows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|window| window["id"].as_i64().unwrap())
        .collect()
}

#[test]
fn title_follows_focus_across_outputs() {
    let Some((session, mut bar)) = two_outputs("title-focus", "") else {
        return;
    };
    let names = output_names(&session);
    assert_eq!(names.len(), 2);
    let toy = ToyWindow::open(session.wayland_socket(), "one");
    // The new window is focused on its output; the other shows nothing.
    let shown = session.wait_for(&mut bar.0, "the title shown", |session| {
        let shown: Vec<_> = titles(session)
            .into_iter()
            .filter(|(_, text)| !text.is_empty())
            .collect();
        (shown.len() == 1 && shown[0].1 == "one").then_some(shown)
    });
    let home = shown[0].0.clone();
    let away = names.iter().find(|name| **name != home).unwrap().clone();
    // To the other output, following focus: the title moves with it, and
    // the first output goes back to nothing (focus is singular: exactly
    // one window is activated).
    let id = window_ids(&session)[0];
    let reply = session.scoot_ipc(&format!(
        r#"{{"type":"action","action":"move_focused_window_to_output_index","index":{}}}"#,
        names.iter().position(|name| *name == away).unwrap()
    ));
    assert_eq!(reply["type"], "ok", "{reply}");
    session.wait_for(&mut bar.0, "the title followed", |session| {
        let shown = titles(session);
        let here = shown.iter().find(|(output, _)| *output == away).unwrap();
        let there = shown.iter().find(|(output, _)| *output == home).unwrap();
        (here.1 == "one" && there.1.is_empty()).then_some(())
    });
    // A second window takes focus: the title switches to it ...
    let second = ToyWindow::open(session.wayland_socket(), "two");
    session.wait_for(&mut bar.0, "the second title", |session| {
        titles(session)
            .iter()
            .any(|(_, text)| text == "two")
            .then_some(())
    });
    // ... and focusing the first by id switches it back.
    let reply = session.scoot_ipc(&format!(
        r#"{{"type":"action","action":"focus_window_id","id":{id}}}"#
    ));
    assert_eq!(reply["type"], "ok", "{reply}");
    session.wait_for(&mut bar.0, "the title came back", |session| {
        let shown = titles(session);
        let here = shown.iter().find(|(output, _)| *output == away).unwrap();
        (here.1 == "one").then_some(())
    });
    second.close(&session, &mut bar);
    toy.close(&session, &mut bar);
    session.wait_for(&mut bar.0, "the title cleared", |session| {
        titles(session)
            .iter()
            .all(|(_, text)| text.is_empty())
            .then_some(())
    });
    assert!(session.bar_stderr().is_empty(), "{}", session.bar_stderr());
}

#[test]
fn a_click_activates_what_it_lands_on() {
    let Some((session, mut bar)) = two_outputs("title-click", "") else {
        return;
    };
    let toy = ToyWindow::open(session.wayland_socket(), "first");
    session.wait_for(&mut bar.0, "the first title", |session| {
        titles(session)
            .iter()
            .any(|(_, text)| text == "first")
            .then_some(())
    });
    // The title is laid out on the focused window's output, with a real
    // rectangle, and on no other output (an empty view takes no space and
    // is not listed).
    let layout = json(&session, &["layout"]);
    let outputs = layout["outputs"].as_array().unwrap();
    let here = outputs
        .iter()
        .find(|output| {
            output["modules"]
                .as_array()
                .unwrap()
                .iter()
                .any(|module| module["id"] == "window-title")
        })
        .expect("the title placed, and shown");
    assert_eq!(outputs.len(), 2);
    let module = here["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|module| module["id"] == "window-title")
        .unwrap();
    assert!(module["width"].as_i64().unwrap() > 0);
    // Click the middle of it: focus is already there (a title is only
    // ever shown for the focused window), so the click re-activates it:
    // the title stands, and nothing warns.
    let origin = here["origin"]["x"].as_i64().unwrap();
    let x = origin + module["x"].as_i64().unwrap() + module["width"].as_i64().unwrap() / 2;
    let y = module["y"].as_i64().unwrap() + module["height"].as_i64().unwrap() / 2;
    let reply = session.scoot_ipc(&format!(
        r#"{{"type":"click","x":{x},"y":{y},"button":"left"}}"#
    ));
    assert_eq!(reply["type"], "ok", "{reply}");
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        titles(&session).iter().any(|(_, text)| text == "first"),
        "the click moved focus away"
    );
    // The same through the agent interface.
    let out = msg(&session, &["invoke", "window-title", "activate"]);
    assert!(
        out.status.success(),
        "invoke activate: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    // A click on the other output's bar, where no title shows, reaches no
    // module and changes nothing.
    let other = outputs
        .iter()
        .find(|output| output["output"].as_str() != here["output"].as_str())
        .unwrap();
    let ox = other["origin"]["x"].as_i64().unwrap() + 50;
    let reply = session.scoot_ipc(&format!(
        r#"{{"type":"click","x":{ox},"y":20,"button":"left"}}"#
    ));
    assert_eq!(reply["type"], "ok", "{reply}");
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        titles(&session).iter().any(|(_, text)| text == "first"),
        "a click on the bare bar moved focus"
    );
    toy.close(&session, &mut bar);
    assert!(session.bar_stderr().is_empty(), "{}", session.bar_stderr());
}

#[test]
fn invoke_close_needs_allow_close() {
    let Some((session, mut bar)) = two_outputs("title-close", "") else {
        return;
    };
    let toy = ToyWindow::open(session.wayland_socket(), "doomed");
    session.wait_for(&mut bar.0, "the title shown", |session| {
        titles(session)
            .iter()
            .any(|(_, text)| text == "doomed")
            .then_some(())
    });
    // Closing is off: the invoke is refused and the window stands.
    let said = refused(&session, &["invoke", "window-title", "close"]);
    assert!(said.contains("allow-close"), "{said}");
    assert_eq!(window_ids(&session).len(), 1);
    // On with a reload: the same invoke closes it.
    let file = session.runtime_dir().join("bar.toml");
    std::fs::write(
        &file,
        format!(
            "left = [\"window-title\"]\n[bar]\nheight = {HEIGHT}\n[colors]\nbackground = \"{BAR}\"\n[window-title]\nallow-close = true\n"
        ),
    )
    .unwrap();
    let out = msg(&session, &["reload"]);
    assert!(
        out.status.success(),
        "reload: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    session.wait_for(&mut bar.0, "the reload applied", |session| {
        json(session, &["query", "window-title"])["modules"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["text"] == "doomed")
            .then_some(())
    });
    let out = msg(&session, &["invoke", "window-title", "close"]);
    assert!(
        out.status.success(),
        "invoke close: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    session.wait_for(&mut bar.0, "the window closed", |session| {
        let reply = session.scoot_ipc(r#"{"type":"windows"}"#);
        reply["windows"].as_array()?.is_empty().then_some(())
    });
    toy.stop();
}

#[test]
fn control_characters_and_cjk_render_safely() {
    let Some((session, mut bar)) = two_outputs("title-odd", "") else {
        return;
    };
    let toy = ToyWindow::open(session.wayland_socket(), "plain");
    session.wait_for(&mut bar.0, "the title shown", |session| {
        titles(session)
            .iter()
            .any(|(_, text)| text == "plain")
            .then_some(())
    });
    toy.retitle("a\tb\nc\x1bd\x7f 日本");
    session.wait_for(&mut bar.0, "the odd title shown", |session| {
        titles(session)
            .iter()
            .any(|(_, text)| text == "abcd 日本")
            .then_some(())
    });
    // The value agrees, and the bar drew it without a word: CJK without a
    // CJK font is the primary's missing-glyph boxes, never a panic.
    let entries = json(&session, &["query", "window-title"])["modules"]
        .as_array()
        .unwrap()
        .clone();
    let shown = entries
        .iter()
        .find(|entry| entry["text"] == "abcd 日本")
        .unwrap();
    assert_eq!(shown["value"]["title"], "abcd 日本");
    for id in [1, 2] {
        let _ = session.scoot_screenshot(id);
    }
    assert!(session.bar_stderr().is_empty(), "{}", session.bar_stderr());
    toy.close(&session, &mut bar);
}

#[test]
fn a_title_flood_costs_a_bounded_redraw_rate() {
    let Some((session, mut bar)) = two_outputs("title-flood", "") else {
        return;
    };
    let toy = ToyWindow::open(session.wayland_socket(), "start");
    session.wait_for(&mut bar.0, "the title shown", |session| {
        titles(session)
            .iter()
            .any(|(_, text)| text == "start")
            .then_some(())
    });
    // Count the module's change events while the title floods.
    let mut subscriber: Child = session
        .scootbar()
        .arg("msg")
        .args(["subscribe", "module"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let stdout = subscriber.stdout.take().unwrap();
    let reader = thread::spawn(move || {
        let mut lines = 0;
        for line in BufReader::new(stdout).lines() {
            if line.is_ok() {
                lines += 1;
            }
        }
        lines
    });
    // Hundreds of retitles as fast as the client can send them.
    for n in 0..300 {
        toy.retitle(&format!("{n}/300"));
    }
    toy.retitle("final");
    session.wait_for(&mut bar.0, "the final title", |session| {
        titles(session)
            .iter()
            .any(|(_, text)| text == "final")
            .then_some(())
    });
    // One flush interval past the last change: every held title told.
    std::thread::sleep(Duration::from_millis(300));
    subscriber.kill().unwrap();
    subscriber.wait().unwrap();
    let events = reader.join().unwrap();
    // Three hundred titles, told a frame at a time: far fewer events than
    // titles (the first line is the subscription itself).
    assert!(
        events <= 40,
        "{events} module events for 300 titles: the flood was not coalesced"
    );
    toy.close(&session, &mut bar);
    assert!(session.bar_stderr().is_empty(), "{}", session.bar_stderr());
}

#[test]
fn a_long_title_is_cut_to_max_width() {
    let Some((session, mut bar)) = two_outputs("title-wide", "[window-title]\nmax-width = 200\n")
    else {
        return;
    };
    let toy = ToyWindow::open(session.wayland_socket(), &"w".repeat(300));
    session.wait_for(&mut bar.0, "the long title", |session| {
        titles(session)
            .iter()
            .any(|(_, text)| !text.is_empty())
            .then_some(())
    });
    // The view holds the whole title (bounded at 256 bytes); the span is
    // capped at max-width plus padding either side.
    let text = titles(&session)
        .into_iter()
        .find(|(_, text)| !text.is_empty())
        .unwrap()
        .1;
    assert_eq!(text.len(), 256, "the view bounds the title, not the span");
    let layout = json(&session, &["layout"]);
    let width = layout["outputs"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|output| output["modules"].as_array().unwrap().clone())
        .find(|module| module["id"] == "window-title")
        .expect("the title placed, and shown")["width"]
        .as_i64()
        .unwrap();
    assert!(
        width <= 200 + 2 * 8,
        "the span is capped at max-width plus padding: {width}"
    );
    toy.close(&session, &mut bar);
}
