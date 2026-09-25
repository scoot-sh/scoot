//! Phase 3: the focus gate and the keyboard. Each refusal here was first
//! run against a build whose gate said yes (see the PR's fail-first record):
//! a test that cannot fail says nothing about a security gate.

use smithay::backend::input::KeyState;
use smithay::input::keyboard::Keycode;
use x11rb::protocol::Event as XEvent;

use super::live::{RED, id_of_xid, live};
use super::peer::{Ack, Step};
use super::x11::{Props, eventually};
use crate::compositor::keyboard_focus::KeyboardFocus;

/// evdev `KEY_A` (30) as the XKB keycode Smithay's seat takes.
const KEY_A: u32 = 30 + 8;

/// With nothing focused, a mapping X window takes focus -- and the
/// keyboard actually reaches it: the X server's input focus names it, and a
/// key injected into the seat arrives as an X `KeyPress` on that window.
#[test]
fn an_x_window_mapping_with_nothing_focused_takes_focus_and_keys_reach_it() {
    let Some(mut live) =
        live("an_x_window_mapping_with_nothing_focused_takes_focus_and_keys_reach_it")
    else {
        return;
    };
    let xid = live.x.map(&Props::new(RED));
    let id = live.managed(xid);
    live.drain();
    assert_eq!(live.fixture.state.focus, Some(id));
    assert!(
        matches!(live.keyboard(), Some(KeyboardFocus::X11 { window, .. }) if window.window_id() == xid),
        "the keyboard is not on the X window: {:?}",
        live.keyboard()
    );
    assert_eq!(
        live.x.input_focus(),
        xid,
        "the X server's input focus does not name the focused X window"
    );
    live.x.drain();
    live.fixture
        .state
        .key(Keycode::new(KEY_A), KeyState::Pressed);
    live.fixture
        .state
        .key(Keycode::new(KEY_A), KeyState::Released);
    live.drain();
    let pressed = live.x.drain().into_iter().any(|event| {
        matches!(event, XEvent::KeyPress(press) if press.event == xid && u32::from(press.detail) == KEY_A)
    });
    assert!(
        pressed,
        "a key typed into the session never reached the X window"
    );
}

/// The gate's first refusal: an X window mapping while a Wayland window has
/// focus is announced (in the layout, on the taskbar) but takes neither
/// focus nor the keyboard.
#[test]
fn an_x_window_mapping_does_not_steal_focus_from_a_wayland_window() {
    let Some(mut live) = live("an_x_window_mapping_does_not_steal_focus_from_a_wayland_window")
    else {
        return;
    };
    let wayland = live.map_peer("wayland");
    assert_eq!(live.fixture.state.focus, Some(wayland));
    assert!(matches!(live.fixture.run(Step::BindTaskbar), Ack::Done));
    let mut props = Props::new(RED);
    props.title = Some("thief");
    let xid = live.x.map(&props);
    live.managed(xid);
    live.drain();
    assert_eq!(
        live.fixture.state.focus,
        Some(wayland),
        "a mapping X window stole focus"
    );
    assert!(
        matches!(live.keyboard(), Some(KeyboardFocus::Surface(_))),
        "the keyboard left the Wayland window"
    );
    assert!(
        live.taskbar().iter().any(|(title, _)| title == "thief"),
        "a refused window must still be announced"
    );
}

/// The gate's second refusal: `_NET_ACTIVE_WINDOW` for an unfocused X
/// window -- what `xdotool windowactivate` sends from any background client
/// -- does nothing while a Wayland window has focus.
#[test]
fn net_active_window_from_a_background_client_is_refused() {
    let Some(mut live) = live("net_active_window_from_a_background_client_is_refused") else {
        return;
    };
    let wayland = live.map_peer("wayland");
    let xid = live.x.map(&Props::new(RED));
    live.managed(xid);
    live.x.request_activation(xid);
    live.drain();
    assert_eq!(
        live.fixture.state.focus,
        Some(wayland),
        "_NET_ACTIVE_WINDOW took focus from a Wayland window"
    );
    assert!(matches!(live.keyboard(), Some(KeyboardFocus::Surface(_))));
}

/// `_NET_ACTIVE_WINDOW` between two windows of the focused X client is an
/// application moving focus between its own windows: honoured. The same
/// request for a window of a *different* X client process -- `xeyes`,
/// started outside the session so it carries no spawn token -- is refused.
/// "Client" is the process the X server names through X-Resource, so the
/// stranger has to be another process: a second connection from this one
/// is, correctly, the same application. The stranger half is skipped where
/// `xeyes` is not on `PATH`; the own-window half always runs.
#[test]
fn net_active_window_is_honoured_only_within_the_focused_client() {
    let Some(mut live) = live("net_active_window_is_honoured_only_within_the_focused_client")
    else {
        return;
    };
    let first = live.x.map(&Props::new(RED));
    let first = live.managed(first);
    assert_eq!(live.fixture.state.focus, Some(first));

    let xeyes = std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join("xeyes").is_file()));
    if xeyes {
        let mut stranger = std::process::Command::new("xeyes")
            .env("DISPLAY", super::display_value(live.display))
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("xeyes starts");
        eventually(&mut live.fixture, "xeyes mapping", |fixture| {
            fixture.state.windows.len() == 2
        });
        let (&strangers, window) = live
            .fixture
            .state
            .windows
            .iter()
            .find(|(id, _)| **id != first)
            .expect("xeyes' window");
        let xid = window.x11_surface().expect("an X window").window_id();
        assert_eq!(
            live.fixture.state.focus,
            Some(first),
            "xeyes took focus on map"
        );
        live.x.request_activation(xid);
        live.drain();
        assert_eq!(
            live.fixture.state.focus,
            Some(first),
            "another X client took focus with _NET_ACTIVE_WINDOW"
        );
        assert_ne!(live.fixture.state.focus, Some(strangers));
        let _ = stranger.kill();
        let _ = stranger.wait();
    } else {
        eprintln!(
            "net_active_window_is_honoured_only_within_the_focused_client: stranger half skipped -- no xeyes on PATH"
        );
    }

    let own = live.x.map(&Props::new(RED));
    let own_id = live.managed(own);
    assert_eq!(
        live.fixture.state.focus,
        Some(first),
        "the second window took focus on map"
    );
    live.x.request_activation(own);
    eventually(
        &mut live.fixture,
        "the client's own window taking focus",
        |fixture| fixture.state.focus == Some(own_id),
    );
}

/// The spawn chain by startup id: a window whose `_NET_STARTUP_ID` names a
/// live spawn token takes focus from a Wayland window -- once. The token is
/// spent, so a second window naming it does not.
#[test]
fn a_spawn_tokens_startup_id_lets_an_x_window_take_focus_once() {
    let Some(mut live) = live("a_spawn_tokens_startup_id_lets_an_x_window_take_focus_once") else {
        return;
    };
    let wayland = live.map_peer("wayland");
    let token = live
        .fixture
        .state
        .mint_spawn_token("xprobe")
        .expect("a spawn token");
    let mut props = Props::new(RED);
    props.startup_id = Some(token.as_str().to_owned());
    let xid = live.x.map(&props);
    let id = live.managed(xid);
    assert_eq!(
        live.fixture.state.focus,
        Some(id),
        "the spawned window did not take focus"
    );
    assert!(
        live.fixture
            .state
            .xdg_activation
            .data_for_token(&token)
            .is_none(),
        "the token was not spent"
    );

    live.fixture
        .state
        .act(scoot_core::Action::FocusWindowId(wayland));
    let again = live.x.map(&props);
    live.managed(again);
    assert_eq!(
        live.fixture.state.focus,
        Some(wayland),
        "a spent token focused a second window"
    );
}

/// The spawn chain by process, for an X client that sets no startup id:
/// `xclock` spawned through `State::spawn` takes focus from a Wayland
/// window, matched by its X-Resource pid. Skipped where `xclock` is not on
/// `PATH`.
#[test]
fn a_spawned_x_client_takes_focus_by_its_process() {
    let on_path = std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join("xclock").is_file()));
    if !on_path {
        eprintln!("a_spawned_x_client_takes_focus_by_its_process: skipped -- no xclock on PATH");
        return;
    }
    // The whole session inside the capture, not just the spawn: spans the
    // XWM creates at start are entered on every X event, and a span made
    // outside a capture panics the registry when entered inside one under
    // `cargo test` (see `capture_logs`).
    let (outcome, logs) = crate::compositor::test_support::capture_logs(|| {
        let mut live = live("a_spawned_x_client_takes_focus_by_its_process")?;
        let wayland = live.map_peer("wayland");
        assert!(live.fixture.state.spawn(&["xclock".to_owned()]));
        eventually(&mut live.fixture, "xclock mapping", |fixture| {
            fixture
                .state
                .windows
                .values()
                .any(|window| window.x11_surface().is_some())
        });
        let id = live
            .fixture
            .state
            .windows
            .iter()
            .find(|(_, window)| window.x11_surface().is_some())
            .map(|(&id, _)| id)
            .expect("xclock's window");
        let outcome = (wayland, id, live.fixture.state.focus);
        // Don't leave the clock running past the test.
        for pid in live.fixture.state.spawned_children.clone() {
            let _ = std::process::Command::new("kill")
                .arg(pid.to_string())
                .status();
        }
        Some(outcome)
    });
    let Some((wayland, id, focus)) = outcome else {
        return;
    };
    assert_ne!(wayland, id);
    assert_eq!(
        focus,
        Some(id),
        "a window spawned from the session did not take focus"
    );
    // By process, not by a startup id: Xt sets none, and this is the path
    // the test is about.
    assert!(
        logs.contains("redeemed its spawn's token by process"),
        "xclock was not matched to its spawn by process: {logs}"
    );
}

/// A click focuses an unfocused X window -- the path a refused window is
/// reached by -- and the keyboard follows, X-side too.
#[test]
fn a_click_focuses_an_x_window() {
    let Some(mut live) = live("a_click_focuses_an_x_window") else {
        return;
    };
    live.map_peer("wayland");
    let xid = live.x.map(&Props::new(RED));
    let id = live.managed(xid);
    assert_ne!(live.fixture.state.focus, Some(id));
    let rect = live.placement(id).rect;
    let (x, y) = (
        f64::from(rect.x + rect.w / 2),
        f64::from(rect.y + rect.h / 2),
    );
    live.fixture.state.pointer_move(x, y);
    live.fixture
        .state
        .pointer_button(scoot_ipc::PointerButton::Left, true);
    live.fixture
        .state
        .pointer_button(scoot_ipc::PointerButton::Left, false);
    live.drain();
    assert_eq!(
        live.fixture.state.focus,
        Some(id),
        "the click did not focus the X window"
    );
    assert_eq!(live.x.input_focus(), xid);
}

/// A taskbar can activate and close an X window through the wlr
/// foreign-toplevel protocol, the same as any window.
#[test]
fn the_taskbar_activates_and_closes_x_windows() {
    let Some(mut live) = live("the_taskbar_activates_and_closes_x_windows") else {
        return;
    };
    live.map_peer("wayland");
    assert!(matches!(live.fixture.run(Step::BindTaskbar), Ack::Done));
    let mut props = Props::new(RED);
    props.title = Some("xwin");
    let xid = live.x.map(&props);
    let id = live.managed(xid);
    assert_ne!(live.fixture.state.focus, Some(id));
    assert!(matches!(
        live.fixture.run(Step::Activate("xwin".to_owned())),
        Ack::Done
    ));
    eventually(&mut live.fixture, "the taskbar's activate", |fixture| {
        fixture.state.focus == Some(id)
    });
    live.x.drain();
    assert!(matches!(
        live.fixture.run(Step::Close("xwin".to_owned())),
        Ack::Done
    ));
    live.drain();
    let asked = live
        .x
        .drain()
        .into_iter()
        .any(|event| matches!(event, XEvent::ClientMessage(message) if message.window == xid));
    assert!(asked, "the taskbar's close never reached the X window");
    assert!(
        id_of_xid(&live.fixture.state, xid).is_some(),
        "closing is a request, not a kill"
    );
}
