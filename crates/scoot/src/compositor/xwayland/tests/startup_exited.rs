//! A spawn token whose launched process has exited: the startup id falls
//! back to the unbound rule (see `focus.rs`, rule 2's binding).
//!
//! Two launch shapes hand the app's window to a process the spawn is not an
//! ancestor of, and let the spawn exit: a single-instance app whose second
//! launch forwards to the running instance and exits (GApplication,
//! `KDBusService`), and an app that forks into the background and lets the
//! spawn exit (`gvim` without `-f`). The window names the spawn's token,
//! but its process does not descend from the spawn. While the spawn runs,
//! that is refused -- the race stays closed; once it has exited, a window
//! that *then* asks naming the token may redeem it once, within its
//! lifetime. A window refused while the spawn ran is re-asked for when it
//! exits -- if it asked moments before and focus has not moved since --
//! because the forwarder's exit was measured to trail the window's map by a
//! few milliseconds. Who else that re-ask may grant: `startup_regrant.rs`.
//!
//! The spawns are real (`State::spawn`, a real exit reaped the way the
//! `SIGCHLD` drain reaps it); the window that redeems is this test process's
//! -- another process, as the running instance or the forked app is.

use std::time::{Duration, Instant};

use smithay::wayland::xdg_activation::XdgActivationToken;

use super::live::{Live, RED, id_of_xid, live};
use super::peer::{Ack, Step};
use super::x11::{Props, eventually};
use crate::compositor::xwayland::SpawnedPid;

/// Spawns `command` through `State::spawn` and returns the new child's pid
/// and the token minted for it.
pub(super) fn spawn(live: &mut Live, command: &[&str]) -> (u32, XdgActivationToken) {
    let state = &mut live.fixture.state;
    let before = state.spawned_children.clone();
    let command: Vec<String> = command.iter().map(|&arg| arg.to_owned()).collect();
    assert!(state.spawn(&command), "the spawn failed to start");
    let pid = *state
        .spawned_children
        .difference(&before)
        .next()
        .expect("the spawn is tracked");
    let token = state
        .xdg_activation
        .tokens()
        .find(|(_, data)| data.user_data.get::<SpawnedPid>() == Some(&SpawnedPid(pid)))
        .map(|(token, _)| token.clone())
        .expect("the spawn's token carries its pid");
    (pid, token)
}

/// Waits until `pid` -- a child of this process -- is a zombie: exited, not
/// yet reaped. Reads `/proc`, so nothing is reaped here.
pub(super) fn wait_exited(pid: u32) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .expect("an unreaped child has a /proc entry");
        let state = stat
            .rsplit_once(')')
            .and_then(|(_, rest)| rest.trim_start().chars().next());
        if state == Some('Z') {
            return;
        }
        assert!(Instant::now() < deadline, "the spawn never exited");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Ends a `sleep` spawn and waits for it to be a zombie.
pub(super) fn end(pid: u32) {
    // SAFETY: `kill` with a pid this test spawned and has not reaped (so it
    // is still ours) and a valid signal; no memory is involved.
    let sent = unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
    assert_eq!(sent, 0, "could not signal the spawn");
    wait_exited(pid);
}

/// The startup-id with `token` on a fresh X window of this test process.
pub(super) fn named(token: &XdgActivationToken) -> Props {
    let mut props = Props::new(RED);
    props.startup_id = Some(token.as_str().to_owned());
    props
}

/// The forward-and-exit order: the spawn has exited and been reaped before
/// a window naming its token maps from another process. Redeemed, and
/// focused. Before the fallback this was refused: the spawn was untracked.
#[test]
fn an_exited_spawns_startup_id_is_redeemed_by_another_process() {
    let Some(mut live) = live("an_exited_spawns_startup_id_is_redeemed_by_another_process") else {
        return;
    };
    let wayland = live.map_peer("wayland");
    let (pid, token) = spawn(&mut live, &["sh", "-c", "exit 0"]);
    wait_exited(pid);
    live.fixture.state.reap_children();
    assert!(!live.fixture.state.spawned_children.contains(&pid));

    let xid = live.x.map(&named(&token));
    let id = live.managed(xid);
    assert_ne!(id, wayland);
    assert_eq!(
        live.fixture.state.focus,
        Some(id),
        "an exited spawn's startup id was refused"
    );
    assert!(
        live.fixture
            .state
            .xdg_activation
            .data_for_token(&token)
            .is_none(),
        "the redeemed token was left live"
    );
}

/// The order measured with a real forwarder: the window maps while the
/// spawn is still running (refused -- the process is not the spawn's), and
/// the spawn exits a moment later -- well within `REFUSAL_GRACE` of the
/// refusal, with focus unmoved (the forwarder was measured 8 ms behind).
/// Its exit re-asks for that window, which then takes focus and spends the
/// token.
#[test]
fn a_window_refused_while_its_spawn_ran_takes_focus_when_it_exits() {
    let Some(mut live) = live("a_window_refused_while_its_spawn_ran_takes_focus_when_it_exits")
    else {
        return;
    };
    let wayland = live.map_peer("wayland");
    let (pid, token) = spawn(&mut live, &["sleep", "30"]);
    let xid = live.x.map(&named(&token));
    let id = live.managed(xid);
    assert_eq!(
        live.fixture.state.focus,
        Some(wayland),
        "a running spawn's token redeemed for a process it did not start"
    );
    assert!(
        live.fixture
            .state
            .xdg_activation
            .data_for_token(&token)
            .is_some()
    );

    end(pid);
    live.fixture.state.reap_children();
    assert_eq!(
        live.fixture.state.focus,
        Some(id),
        "the window was not re-asked for when its spawn exited"
    );
    assert!(
        live.fixture
            .state
            .xdg_activation
            .data_for_token(&token)
            .is_none(),
        "the redeemed token was left live"
    );
}

/// The refusal cache (`RefusedSpawn`) records a "no" against the spawn's
/// pid while it runs; once the spawn is gone that "no" must not outlive it.
/// Forgotten here without the drain's re-ask (the pid dropped and reaped by
/// hand), so only the window's own `_NET_ACTIVE_WINDOW` can redeem -- and
/// it does.
#[test]
fn a_refusal_cached_while_the_spawn_ran_does_not_outlive_it() {
    let Some(mut live) = live("a_refusal_cached_while_the_spawn_ran_does_not_outlive_it") else {
        return;
    };
    let wayland = live.map_peer("wayland");
    let (pid, token) = spawn(&mut live, &["sleep", "30"]);
    let xid = live.x.map(&named(&token));
    let id = live.managed(xid);
    // Refused on map, then again from the cache.
    live.x.request_activation(xid);
    live.drain();
    assert_eq!(live.fixture.state.focus, Some(wayland));

    end(pid);
    assert!(live.fixture.state.spawned_children.remove(&pid));
    let mut status = 0;
    // SAFETY: reaps our own zombie child; `status` is a valid out-pointer.
    let reaped = unsafe { libc::waitpid(pid as libc::pid_t, &mut status, 0) };
    assert_eq!(reaped, pid as libc::pid_t);

    live.x.request_activation(xid);
    eventually(
        &mut live.fixture,
        "the window redeeming the exited spawn's token",
        |fixture| fixture.state.focus == Some(id),
    );
    assert!(
        live.fixture
            .state
            .xdg_activation
            .data_for_token(&token)
            .is_none()
    );
}

/// The re-ask on exit is an activation like any other: behind the lock it
/// grants nothing, and leaves the token alone. The window maps and is
/// refused *after* the session locked, so focus has not moved since the
/// refusal and only the lock guard stands between it and the grant.
#[test]
fn a_spawn_exiting_under_the_lock_focuses_nothing() {
    let Some(mut live) = live("a_spawn_exiting_under_the_lock_focuses_nothing") else {
        return;
    };
    live.map_peer("wayland");
    let (pid, token) = spawn(&mut live, &["sleep", "30"]);
    assert!(matches!(live.fixture.run(Step::Lock), Ack::Done));
    eventually(&mut live.fixture, "the session locking", |fixture| {
        fixture.state.session_lock.is_locked()
    });
    let xid = live.x.map(&named(&token));
    // Only managed: nothing draws behind the lock, so `Live::managed`'s wait
    // for the window's first frame would never end.
    eventually(
        &mut live.fixture,
        "the X window entering the layout",
        |fixture| id_of_xid(&fixture.state, xid).is_some(),
    );
    let id = id_of_xid(&live.fixture.state, xid).expect("just waited for it");

    end(pid);
    live.fixture.state.reap_children();
    assert_ne!(
        live.fixture.state.focus,
        Some(id),
        "a spawn's exit focused an X window behind the lock"
    );
    assert!(
        live.fixture
            .state
            .xdg_activation
            .data_for_token(&token)
            .is_some(),
        "the lock's refusal spent the token"
    );
}
