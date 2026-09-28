//! Who a spawn's exit re-asks for (`State::x11_focus_for_exited_spawns`):
//! only a window that asked for focus with the spawn's token while it ran
//! and was refused for it, moments before the exit, with focus unmoved
//! since. A window that only carries the startup id, an unrelated child's
//! exit, a stale refusal and a user's focus change in between all grant
//! nothing; of two askers the first to ask wins.
//!
//! The spawns are real (`State::spawn`, reaped the way the `SIGCHLD` drain
//! reaps them); the windows are this test process's, a process none of the
//! spawns started, as a forwarded-to running instance is.

use std::time::Duration;

use scoot_core::{Action, WindowId};
use smithay::wayland::xdg_activation::XdgActivationToken;
use x11rb::protocol::xproto::Window as XWindow;

use super::live::{Live, RED, live};
use super::startup_exited::{end, named, spawn, wait_exited};
use super::x11::{Props, XClient, eventually};
use crate::compositor::xwayland::focus::REFUSAL_GRACE;

/// Sets `token` as `xid`'s startup id after it mapped, and waits until the
/// XWM has read it.
fn name_later(live: &mut Live, xid: XWindow, id: WindowId, token: &XdgActivationToken) {
    live.x.set_startup_id(xid, token.as_str());
    eventually(
        &mut live.fixture,
        "the XWM reading the new startup id",
        |fixture| {
            fixture
                .state
                .window(id)
                .and_then(|window| window.x11_surface())
                .and_then(|x11| x11.startup_id())
                .as_deref()
                == Some(token.as_str())
        },
    );
}

fn live_token(live: &Live, token: &XdgActivationToken) -> bool {
    live.fixture
        .state
        .xdg_activation
        .data_for_token(token)
        .is_some()
}

/// An X client that set the launch's startup id on a window it mapped
/// earlier -- never asking for focus with it -- is owed nothing when the
/// spawn exits; the window that asked and was refused is, though it mapped
/// later (a higher id).
#[test]
fn a_window_that_only_carries_the_startup_id_is_not_granted() {
    let Some(mut live) = live("a_window_that_only_carries_the_startup_id_is_not_granted") else {
        return;
    };
    let wayland = live.map_peer("wayland");
    let (pid, token) = spawn(&mut live, &["sleep", "30"]);
    let passive_xid = live.x.map(&Props::new(RED));
    let passive = live.managed(passive_xid);
    name_later(&mut live, passive_xid, passive, &token);
    let asker_xid = live.x.map(&named(&token));
    let asker = live.managed(asker_xid);
    assert!(passive < asker);
    assert_eq!(live.fixture.state.focus, Some(wayland));

    end(pid);
    live.fixture.state.reap_children();
    assert_eq!(
        live.fixture.state.focus,
        Some(asker),
        "the spawn's exit did not grant the window that asked"
    );
    assert!(
        !live_token(&live, &token),
        "the redeemed token was left live"
    );
}

/// The same passive window, alone: nothing is granted, and the token is
/// left for the unbound rule.
#[test]
fn a_spawn_exit_with_no_asker_grants_nothing() {
    let Some(mut live) = live("a_spawn_exit_with_no_asker_grants_nothing") else {
        return;
    };
    let wayland = live.map_peer("wayland");
    let (pid, token) = spawn(&mut live, &["sleep", "30"]);
    let xid = live.x.map(&Props::new(RED));
    let passive = live.managed(xid);
    name_later(&mut live, xid, passive, &token);

    end(pid);
    live.fixture.state.reap_children();
    assert_eq!(
        live.fixture.state.focus,
        Some(wayland),
        "a window that never asked took focus on the spawn's exit"
    );
    assert!(live_token(&live, &token), "an unowed token was spent");
}

/// A spawn token already orphaned (its spawn exited with no window about),
/// named by a window after the fact: another child's exit -- a volume key's
/// spawn, say -- re-asks for nothing.
#[test]
fn an_unrelated_childs_exit_grants_nothing() {
    let Some(mut live) = live("an_unrelated_childs_exit_grants_nothing") else {
        return;
    };
    let wayland = live.map_peer("wayland");
    let (orphaned, token) = spawn(&mut live, &["sh", "-c", "exit 0"]);
    wait_exited(orphaned);
    live.fixture.state.reap_children();
    let xid = live.x.map(&Props::new(RED));
    let passive = live.managed(xid);
    name_later(&mut live, xid, passive, &token);

    let (unrelated, _) = spawn(&mut live, &["sh", "-c", "exit 0"]);
    wait_exited(unrelated);
    live.fixture.state.reap_children();
    assert!(!live.fixture.state.spawned_children.contains(&unrelated));
    assert_eq!(
        live.fixture.state.focus,
        Some(wayland),
        "an unrelated child's exit focused a window naming another spawn's token"
    );
    assert!(live_token(&live, &token));
}

/// A refusal older than `REFUSAL_GRACE` when the spawn exits is not cashed
/// in: the window asked long before, and the token is left.
#[test]
fn a_stale_refusal_is_not_granted() {
    let Some(mut live) = live("a_stale_refusal_is_not_granted") else {
        return;
    };
    let wayland = live.map_peer("wayland");
    let (pid, token) = spawn(&mut live, &["sleep", "30"]);
    let xid = live.x.map(&named(&token));
    let id = live.managed(xid);
    assert_eq!(live.fixture.state.focus, Some(wayland));

    std::thread::sleep(REFUSAL_GRACE + Duration::from_millis(200));
    end(pid);
    live.fixture.state.reap_children();
    assert_ne!(
        live.fixture.state.focus,
        Some(id),
        "a refusal older than the grace was granted"
    );
    // The first asker's lapsed claim settles the token: spent, so a window
    // that asks with a copied id after the app's window did cannot take it
    // (as under the unbound rule, where the app's map spent it).
    assert!(
        !live_token(&live, &token),
        "the token outlived the first asker's lapsed claim"
    );
    let copier = XClient::connect(live.display);
    let copied = copier.map(&named(&token));
    let copied = live.managed(copied);
    assert_ne!(
        live.fixture.state.focus,
        Some(copied),
        "a window asking after the app's window took the spawn's focus"
    );
}

/// Focus moved after the refusal -- the user chose a window -- so the
/// spawn's exit does not take it back.
#[test]
fn a_focus_change_since_the_refusal_is_kept() {
    let Some(mut live) = live("a_focus_change_since_the_refusal_is_kept") else {
        return;
    };
    let first = live.map_peer("first");
    let second = live.map_peer("second");
    let (pid, token) = spawn(&mut live, &["sleep", "30"]);
    let xid = live.x.map(&named(&token));
    let id = live.managed(xid);
    let refused_under = live.fixture.state.focus;
    assert!(refused_under == Some(first) || refused_under == Some(second));
    let other = if refused_under == Some(first) {
        second
    } else {
        first
    };
    live.fixture.state.act(Action::FocusWindowId(other));
    assert_eq!(live.fixture.state.focus, Some(other));

    end(pid);
    live.fixture.state.reap_children();
    assert_eq!(
        live.fixture.state.focus,
        Some(other),
        "the spawn's exit overrode a focus change made after the refusal"
    );
    assert_ne!(live.fixture.state.focus, Some(id));
    // The first asker settled the token: spent with no grant.
    assert!(!live_token(&live, &token));
}

/// Two windows asked with the token and were refused: the first to ask
/// wins, not the first mapped. The lower-id window maps first without the
/// id, then names it and asks second by `_NET_ACTIVE_WINDOW`.
#[test]
fn of_two_refused_askers_the_first_to_ask_wins() {
    let Some(mut live) = live("of_two_refused_askers_the_first_to_ask_wins") else {
        return;
    };
    let wayland = live.map_peer("wayland");
    let (pid, token) = spawn(&mut live, &["sleep", "30"]);
    let late_xid = live.x.map(&Props::new(RED));
    let late = live.managed(late_xid);
    let early_xid = live.x.map(&named(&token));
    let early = live.managed(early_xid);
    assert!(late < early);
    name_later(&mut live, late_xid, late, &token);
    live.x.request_activation(late_xid);
    live.drain();
    assert_eq!(live.fixture.state.focus, Some(wayland));

    end(pid);
    live.fixture.state.reap_children();
    assert_eq!(
        live.fixture.state.focus,
        Some(early),
        "the later asker (the lower id) took the spawn's token"
    );
    assert!(!live_token(&live, &token));
}

/// A window refused the token at map but focused anyway -- by rule 1,
/// nothing else being focused -- has the token spent when the spawn exits:
/// left live, it would outlive the spawn under the unbound rule, a copyable
/// id any X client could redeem to take focus back from it.
#[test]
fn a_refused_window_focused_anyway_spends_the_token_on_exit() {
    let Some(mut live) = live("a_refused_window_focused_anyway_spends_the_token_on_exit") else {
        return;
    };
    let (pid, token) = spawn(&mut live, &["sleep", "30"]);
    let xid = live.x.map(&named(&token));
    let id = live.managed(xid);
    assert_eq!(
        live.fixture.state.focus,
        Some(id),
        "rule 1 did not focus it"
    );
    assert!(
        live_token(&live, &token),
        "the binding's refusal spent the token"
    );

    end(pid);
    live.fixture.state.reap_children();
    assert_eq!(live.fixture.state.focus, Some(id));
    assert!(
        !live_token(&live, &token),
        "the focused window's token outlived its spawn"
    );
}
