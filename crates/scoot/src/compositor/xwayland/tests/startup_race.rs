//! The startup-id race: a launched X application sets its startup id on its
//! client leader at startup, well before its first window maps, and every X
//! client can read it. A background X client that copies the id onto a
//! window of its own and maps first must not redeem the launch's token --
//! the redemption is bound to the process the token was minted for (see
//! `focus.rs`, rule 2).
//!
//! The launched application here is real: this test binary re-run, through
//! `State::spawn` and an `sh -c` wrapper, as [`launched_app`] -- so its token
//! carries the wrapper's pid, reaches it as `DESKTOP_STARTUP_ID` with
//! `DISPLAY` beside it, and the process mapping the window is a *descendant*
//! of the one the token names, as any app behind a wrapper script is. The
//! racer is this test process: another process, watching, as an attacker
//! would be.

use std::time::{Duration, Instant};

use x11rb::connection::Connection as _;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _, PropMode};
use x11rb::wrapper::ConnectionExt as _;

use super::live::{BLUE, RED, live};
use super::x11::{Props, XClient, eventually};
use smithay::reexports::wayland_server::Resource as _;
use smithay::wayland::xdg_activation::{XdgActivationToken, XdgActivationTokenData};

/// Set (by the `sh -c` wrapper) only on the re-run that plays the launched
/// application, so [`launched_app`] is a no-op anywhere else -- including a
/// `cargo test -- --ignored` run.
const LAUNCHED_APP_ENV: &str = "SCOOT_TEST_STARTUP_RACE_APP";

/// The property the test sets on the launched app's leader to tell it to map
/// its window: the go signal, over X, so the helper needs no side channel.
const GO: &str = "SCOOT_TEST_STARTUP_RACE_GO";

/// This helper's libtest name, for the re-run.
const LAUNCHED_APP_TEST: &str = "compositor::xwayland::tests::startup_race::launched_app";

/// A racer copies the launched app's startup id and maps first: refused, the
/// token left live -- and `_NET_ACTIVE_WINDOW` from the racer refused too.
/// Then the launched app maps its own window, naming its leader, and takes
/// focus from the Wayland window by redeeming the token. Before the gate
/// was bound to the process this failed at the first assertion: the racer
/// took focus.
#[test]
fn a_copied_startup_id_cannot_race_the_launched_app_to_focus() {
    let Some(mut live) = live("a_copied_startup_id_cannot_race_the_launched_app_to_focus") else {
        return;
    };
    let wayland = live.map_peer("wayland");
    let exe = std::env::current_exe().expect("the test binary's path");
    // Not `exec`: the wrapper stays the token's process, and the app its
    // child, the shape of a launcher script.
    let script = format!(
        "{LAUNCHED_APP_ENV}=1 \"$0\" --exact {LAUNCHED_APP_TEST} --ignored --nocapture; exit 0"
    );
    assert!(live.fixture.state.spawn(&[
        "sh".to_owned(),
        "-c".to_owned(),
        script,
        exe.to_string_lossy().into_owned(),
    ]));
    let spawned: Vec<u32> = live
        .fixture
        .state
        .spawned_children
        .iter()
        .copied()
        .collect();

    // The app's leader appears carrying its startup id; the racer reads it
    // off the X server, as any watching client can.
    eventually(
        &mut live.fixture,
        "the launched app's leader carrying its startup id",
        |fixture| !fixture.state.x11_startup_carriers.is_empty(),
    );
    let leader = *live
        .fixture
        .state
        .x11_startup_carriers
        .keys()
        .next()
        .expect("just waited for it");
    let copied = read_startup_id(&live.x, leader);
    let token = XdgActivationToken::from(copied.clone());
    assert!(
        live.fixture
            .state
            .xdg_activation
            .data_for_token(&token)
            .is_some(),
        "the leader's startup id is not the spawn's live token"
    );

    let mut props = Props::new(RED);
    props.startup_id = Some(copied);
    let racer = live.x.map(&props);
    live.managed(racer);
    assert_eq!(
        live.fixture.state.focus,
        Some(wayland),
        "a copied startup id raced the launched app to focus"
    );
    assert!(
        live.fixture
            .state
            .xdg_activation
            .data_for_token(&token)
            .is_some(),
        "the racer's refused map spent the launched app's token"
    );
    live.x.request_activation(racer);
    live.drain();
    assert_eq!(
        live.fixture.state.focus,
        Some(wayland),
        "a copied startup id took focus with _NET_ACTIVE_WINDOW"
    );

    // Go: the app maps its window, naming its leader.
    let go = live.x.atom(GO);
    live.x
        .conn
        .change_property8(PropMode::REPLACE, leader, go, AtomEnum::STRING, b"1")
        .expect("a property request");
    live.x.conn.flush().expect("the go signal hit the wire");
    eventually(&mut live.fixture, "the launched app's window", |fixture| {
        fixture.state.windows.values().any(|window| {
            window
                .x11_surface()
                .is_some_and(|x11| x11.window_id() != racer)
        })
    });
    let app = live
        .fixture
        .state
        .windows
        .iter()
        .find(|(_, window)| {
            window
                .x11_surface()
                .is_some_and(|x11| x11.window_id() != racer)
        })
        .map(|(&id, _)| id)
        .expect("just waited for it");
    eventually(
        &mut live.fixture,
        "the launched app taking focus",
        |fixture| fixture.state.focus == Some(app),
    );
    assert!(
        live.fixture
            .state
            .xdg_activation
            .data_for_token(&token)
            .is_none(),
        "the launched app took focus but left its token live"
    );
    // The app exits when the X server goes with the fixture; the wrapper
    // with it. Killed here too, so a failure above leaves nothing behind
    // for longer than the fixture.
    for pid in spawned {
        let _ = std::process::Command::new("pkill")
            .args(["-P", &pid.to_string()])
            .status();
    }
}

/// `_NET_STARTUP_ID` on `window`, read over `x` -- what a watching client
/// sees.
fn read_startup_id(x: &XClient, window: u32) -> String {
    let atom = x.atom("_NET_STARTUP_ID");
    let reply = x
        .conn
        .get_property(false, window, atom, AtomEnum::ANY, 0, 1024)
        .expect("a property request")
        .reply()
        .expect("the property");
    String::from_utf8(reply.value).expect("a UTF-8 startup id")
}

/// Not a test: the launched application of
/// [`a_copied_startup_id_cannot_race_the_launched_app_to_focus`], run as its
/// own process by that test through `State::spawn`. Does what a GTK app
/// does -- a client-leader window carrying `$DESKTOP_STARTUP_ID` at startup
/// -- then waits for the go signal before mapping its window in that
/// leader's group, and stays connected until the X server goes away.
#[test]
#[ignore = "a helper process the startup-id race test runs; a no-op on its own"]
fn launched_app() {
    if std::env::var_os(LAUNCHED_APP_ENV).is_none() {
        return;
    }
    let display: u32 = std::env::var(super::DISPLAY_ENV)
        .ok()
        .and_then(|value| value.strip_prefix(':')?.parse().ok())
        .expect("State::spawn exported DISPLAY");
    let startup =
        std::env::var("DESKTOP_STARTUP_ID").expect("State::spawn exported the startup id");
    let x = XClient::connect(display);
    let leader = x.leader_with_startup_id(&startup);
    let go = x.atom(GO);
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let Ok(reply) = x
            .conn
            .get_property(false, leader, go, AtomEnum::ANY, 0, 1)
            .map_err(drop)
            .and_then(|cookie| cookie.reply().map_err(drop))
        else {
            // The server went away: the test is over.
            return;
        };
        if !reply.value.is_empty() {
            break;
        }
        if Instant::now() > deadline {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let mut props = Props::new(BLUE);
    props.group_leader = Some(leader);
    x.map(&props);
    while x.conn.wait_for_event().is_ok() {}
}

/// A spawn token whose spawn is no longer tracked -- reaped, so its pid may
/// already be someone else's -- redeems for no startup id, and is left
/// alone.
#[test]
fn a_reaped_spawns_startup_id_is_refused() {
    let Some(mut live) = live("a_reaped_spawns_startup_id_is_refused") else {
        return;
    };
    let wayland = live.map_peer("wayland");
    let token = live.launch_token();
    live.fixture
        .state
        .spawned_children
        .remove(&std::os::unix::process::parent_id());
    let mut props = Props::new(RED);
    props.startup_id = Some(token.as_str().to_owned());
    let xid = live.x.map(&props);
    live.managed(xid);
    assert_eq!(
        live.fixture.state.focus,
        Some(wayland),
        "a reaped spawn's startup id took focus"
    );
    assert!(
        live.fixture
            .state
            .xdg_activation
            .data_for_token(&token)
            .is_some()
    );
}

/// A token scoot minted with no spawn recorded -- as for a spawn while
/// XWayland was not live, never handed to an X toolkit -- redeems nothing
/// for an X window: one naming it copied it.
#[test]
fn an_unbound_compositor_token_is_refused_for_x() {
    let Some(mut live) = live("an_unbound_compositor_token_is_refused_for_x") else {
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
    live.managed(xid);
    assert_eq!(
        live.fixture.state.focus,
        Some(wayland),
        "an unbound compositor token took focus"
    );
    assert!(
        live.fixture
            .state
            .xdg_activation
            .data_for_token(&token)
            .is_some()
    );
}

/// A Wayland launcher's token -- minted by a client, so it passed the
/// serial gate -- keeps the unbound rule: scoot cannot know which process
/// the launcher started, so any X window naming it redeems it, once (the
/// documented cost in `focus.rs`: this is the race left open).
#[test]
fn a_wayland_launchers_token_keeps_the_unbound_rule() {
    let Some(mut live) = live("a_wayland_launchers_token_keeps_the_unbound_rule") else {
        return;
    };
    let wayland = live.map_peer("wayland");
    let launcher = live
        .fixture
        .state
        .windows
        .get(&wayland)
        .and_then(|window| window.toplevel())
        .and_then(|toplevel| toplevel.wl_surface().client())
        .expect("the peer's client")
        .id();
    let (token, _) =
        live.fixture
            .state
            .xdg_activation
            .create_external_token(XdgActivationTokenData {
                client_id: Some(launcher),
                ..XdgActivationTokenData::default()
            });
    let token = token.clone();
    let mut props = Props::new(RED);
    props.startup_id = Some(token.as_str().to_owned());
    let xid = live.x.map(&props);
    let id = live.managed(xid);
    assert_eq!(
        live.fixture.state.focus,
        Some(id),
        "a launcher's token was not redeemed"
    );
    assert!(
        live.fixture
            .state
            .xdg_activation
            .data_for_token(&token)
            .is_none()
    );
}
