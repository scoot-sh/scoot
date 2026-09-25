//! Phase 4: the clipboard and the primary selection across the X/Wayland
//! line (see `xwayland/selection.rs`). Round trips both ways, small and
//! large; the gate (a background X client neither sets nor reads, the lock
//! refuses both); and the X owner a Wayland paste reaches being the one the
//! gate let through. Each refusal was first run against a bridge without the
//! branch that makes it (see the PR's fail-first record).

use std::sync::Arc;

use scoot_core::{Action, WindowId};

use super::live::{Live, RED, live};
use super::peer::{Ack, ClipStep, Step, Which};
use super::x11::{Props, eventually};
use super::xsel::{Owner, OwnerManner, Pace, Read, Reader, patterned};
use crate::compositor::keyboard_focus::KeyboardFocus;

/// The X target and the Wayland mime type Smithay maps it to.
const X_UTF8: &str = "UTF8_STRING";
const UTF8: &str = "text/plain;charset=utf-8";

/// More than a megabyte, so it crosses in `INCR` chunks both ways.
const LARGE: usize = 2 * 1024 * 1024 + 12_345;

fn clip(live: &mut Live, step: ClipStep) -> Ack {
    live.fixture.run(Step::Clip(step))
}

fn offered(live: &mut Live, which: Which) -> Option<Vec<String>> {
    match clip(live, ClipStep::Offered(which)) {
        Ack::Mimes(mimes) => mimes,
        other => panic!("expected mime types, got {other:?}"),
    }
}

fn control_offered(live: &mut Live, which: Which) -> Option<Vec<String>> {
    match clip(live, ClipStep::ControlOffered(which)) {
        Ack::Mimes(mimes) => mimes,
        other => panic!("expected mime types, got {other:?}"),
    }
}

fn receive(live: &mut Live, which: Which) -> Result<Vec<u8>, String> {
    match clip(live, ClipStep::Receive { which, mime: UTF8 }) {
        Ack::Bytes(bytes) => bytes,
        other => panic!("expected bytes, got {other:?}"),
    }
}

fn set(live: &mut Live, which: Which, payload: Arc<Vec<u8>>) {
    assert!(matches!(
        clip(
            live,
            ClipStep::Set {
                which,
                mime: UTF8,
                payload
            }
        ),
        Ack::Done
    ));
}

fn selection_name(which: Which) -> &'static str {
    match which {
        Which::Clipboard => "CLIPBOARD",
        Which::Primary => "PRIMARY",
    }
}

fn focus(live: &mut Live, id: WindowId) {
    live.fixture.state.act(Action::FocusWindowId(id));
    live.drain();
    assert_eq!(live.fixture.state.focus, Some(id));
}

/// A live fixture with the peer's selection devices bound, an X window
/// mapped and focused, and the peer's Wayland window mapped (unfocused).
/// Returns the fixture and the two windows' ids, `(x, wayland)`.
fn clipboard(test: &str) -> Option<(Live, WindowId, WindowId)> {
    let mut live = live(test)?;
    let xid = live.x.map(&Props::new(RED));
    let x = live.managed(xid);
    let wayland = live.map_peer("wayland");
    assert!(matches!(clip(&mut live, ClipStep::Bind), Ack::Done));
    focus(&mut live, x);
    assert!(matches!(live.keyboard(), Some(KeyboardFocus::X11 { .. })));
    Some((live, x, wayland))
}

/// X to Wayland, for `which` and a payload of `len` bytes: an X client sets
/// it while its window is focused, and the Wayland window, once focused,
/// pastes exactly those bytes. A clipboard manager (data-control) is told
/// about it too, whatever is focused.
fn x_to_wayland(test: &str, which: Which, len: usize) {
    let Some((mut live, _, wayland)) = clipboard(test) else {
        return;
    };
    let payload = patterned(len);
    let owner = Owner::start(
        live.display,
        selection_name(which),
        X_UTF8,
        payload.clone(),
        OwnerManner::default(),
    );
    live.drain();
    let control = control_offered(&mut live, which);
    assert!(
        control
            .as_deref()
            .is_some_and(|m| m.iter().any(|m| m == UTF8)),
        "a clipboard manager was not offered the X selection: {control:?}"
    );
    focus(&mut live, wayland);
    let mimes = offered(&mut live, which);
    assert!(
        mimes
            .as_deref()
            .is_some_and(|m| m.iter().any(|m| m == UTF8)),
        "the focused Wayland window was not offered the X selection: {mimes:?}"
    );
    let got = receive(&mut live, which).expect("the paste completes");
    assert_eq!(got.len(), payload.len(), "the paste was truncated");
    assert!(
        got == *payload,
        "the paste's bytes differ from the X owner's"
    );
    assert!(owner.served() >= 1);
}

/// Wayland to X, the other way round: the Wayland window sets it while
/// focused, and an X client, once an X window is focused, converts exactly
/// those bytes.
fn wayland_to_x(test: &str, which: Which, len: usize) {
    let Some((mut live, x, wayland)) = clipboard(test) else {
        return;
    };
    let payload = patterned(len);
    focus(&mut live, wayland);
    set(&mut live, which, payload.clone());
    live.drain();
    focus(&mut live, x);
    let reader = Reader::start(live.display, selection_name(which), X_UTF8, Pace::Normal);
    let got = reader
        .finish(&mut live.fixture)
        .expect("the X read completes");
    let Read::Data(got) = got else {
        panic!("the X read was refused");
    };
    assert_eq!(got.len(), payload.len(), "the X read was truncated");
    assert!(
        got == *payload,
        "the X read's bytes differ from the Wayland source's"
    );
}

#[test]
fn the_clipboard_crosses_from_x_to_wayland() {
    x_to_wayland(
        "the_clipboard_crosses_from_x_to_wayland",
        Which::Clipboard,
        64,
    );
}

#[test]
fn a_large_clipboard_crosses_from_x_to_wayland_in_chunks() {
    x_to_wayland(
        "a_large_clipboard_crosses_from_x_to_wayland_in_chunks",
        Which::Clipboard,
        LARGE,
    );
}

#[test]
fn the_clipboard_crosses_from_wayland_to_x() {
    wayland_to_x(
        "the_clipboard_crosses_from_wayland_to_x",
        Which::Clipboard,
        64,
    );
}

#[test]
fn a_large_clipboard_crosses_from_wayland_to_x_in_chunks() {
    wayland_to_x(
        "a_large_clipboard_crosses_from_wayland_to_x_in_chunks",
        Which::Clipboard,
        LARGE,
    );
}

#[test]
fn the_primary_selection_crosses_from_x_to_wayland() {
    x_to_wayland(
        "the_primary_selection_crosses_from_x_to_wayland",
        Which::Primary,
        64,
    );
}

#[test]
fn the_primary_selection_crosses_from_wayland_to_x() {
    wayland_to_x(
        "the_primary_selection_crosses_from_wayland_to_x",
        Which::Primary,
        64,
    );
}

/// While a Wayland window holds the keyboard, an X client setting the
/// clipboard or the primary selection does not reach Wayland: neither the
/// focused window nor a clipboard manager is offered it.
#[test]
fn a_background_x_client_cannot_set_the_wayland_selection() {
    let Some((mut live, _, wayland)) =
        clipboard("a_background_x_client_cannot_set_the_wayland_selection")
    else {
        return;
    };
    focus(&mut live, wayland);
    let _clipboard = Owner::start(
        live.display,
        "CLIPBOARD",
        X_UTF8,
        patterned(64),
        OwnerManner::default(),
    );
    let _primary = Owner::start(
        live.display,
        "PRIMARY",
        X_UTF8,
        patterned(64),
        OwnerManner::default(),
    );
    live.drain();
    for which in [Which::Clipboard, Which::Primary] {
        assert_eq!(
            offered(&mut live, which),
            None,
            "a background X client set the Wayland {which:?} for the focused window"
        );
        assert_eq!(
            control_offered(&mut live, which),
            None,
            "a background X client set the Wayland {which:?} for a clipboard manager"
        );
    }
}

/// While a Wayland window holds the keyboard, an X client asking for the
/// Wayland clipboard or primary selection is refused.
#[test]
fn a_background_x_client_cannot_read_the_wayland_selection() {
    let Some((mut live, _, wayland)) =
        clipboard("a_background_x_client_cannot_read_the_wayland_selection")
    else {
        return;
    };
    focus(&mut live, wayland);
    for which in [Which::Clipboard, Which::Primary] {
        set(&mut live, which, patterned(64));
        live.drain();
        let reader = Reader::start(live.display, selection_name(which), X_UTF8, Pace::Normal);
        assert_eq!(
            reader.finish(&mut live.fixture),
            Ok(Read::Refused),
            "a background X client read the Wayland {which:?}"
        );
    }
}

/// While the session is locked, an X client can neither set a selection
/// that reaches Wayland nor read one -- even with an X window focused just
/// before the lock.
#[test]
fn the_lock_refuses_x_selection_traffic() {
    let Some((mut live, _, wayland)) = clipboard("the_lock_refuses_x_selection_traffic") else {
        return;
    };
    // A Wayland selection to try to read, set before the lock.
    focus(&mut live, wayland);
    set(&mut live, Which::Clipboard, patterned(64));
    live.drain();
    let x = live
        .fixture
        .state
        .windows
        .iter()
        .find(|(_, window)| window.x11_surface().is_some())
        .map(|(&id, _)| id)
        .expect("the X window");
    focus(&mut live, x);
    assert!(matches!(live.fixture.run(Step::Lock), Ack::Done));
    eventually(&mut live.fixture, "the session locking", |fixture| {
        fixture.state.session_lock.is_locked()
    });
    live.drain();
    let reader = Reader::start(live.display, "CLIPBOARD", X_UTF8, Pace::Normal);
    assert_eq!(
        reader.finish(&mut live.fixture),
        Ok(Read::Refused),
        "an X client read the Wayland clipboard while the session was locked"
    );
    let _owner = Owner::start(
        live.display,
        "PRIMARY",
        X_UTF8,
        patterned(64),
        OwnerManner::default(),
    );
    live.drain();
    assert_eq!(
        control_offered(&mut live, Which::Primary),
        None,
        "an X client set the Wayland primary selection while the session was locked"
    );
}

/// The X selection a Wayland paste is converted from is the one the gate
/// let through -- not whichever X client owns the selection by the time of
/// the paste. Here a background X client takes the clipboard after the
/// user copied in X and moved to a Wayland window, and hides from the
/// bridge by never answering `TARGETS` (so the bridge is never told the
/// owner changed); it still answers a conversion to the type the
/// legitimate copy offered. The paste must not deliver its bytes.
#[test]
fn a_paste_is_never_served_by_an_owner_the_gate_did_not_let_through() {
    let Some((mut live, _, wayland)) =
        clipboard("a_paste_is_never_served_by_an_owner_the_gate_did_not_let_through")
    else {
        return;
    };
    let legitimate = Arc::new(b"copied in the X window".to_vec());
    let copy = Owner::start(
        live.display,
        "CLIPBOARD",
        X_UTF8,
        legitimate.clone(),
        OwnerManner::default(),
    );
    live.drain();
    focus(&mut live, wayland);
    assert!(offered(&mut live, Which::Clipboard).is_some());
    let intruder = Arc::new(b"rm -rf ~ # injected".to_vec());
    let hidden = Owner::start(
        live.display,
        "CLIPBOARD",
        X_UTF8,
        intruder.clone(),
        OwnerManner {
            answers_targets: false,
            ..OwnerManner::default()
        },
    );
    live.drain();
    assert!(copy.lost(), "the hidden owner never took the clipboard");
    let got = receive(&mut live, Which::Clipboard);
    assert!(
        got.as_deref() != Ok(intruder.as_slice()),
        "a paste delivered the bytes of an X owner the gate never let through"
    );
    assert_eq!(
        hidden.served(),
        0,
        "the hidden owner served a Wayland paste"
    );
}

/// Hostile selection types never reach a Wayland client: an X owner
/// offering an atom name longer than the Wayland wire allows (which would
/// disconnect every client it is sent to) and an X-only target name, next
/// to a real type, plus an atom name with a NUL in it. The X server itself
/// cuts an atom name at its first NUL (measured: `text/evil\0type` comes
/// back as `text/evil`), so that one arrives harmless and legitimately
/// crosses; the filter's own NUL refusal is pinned by
/// `hostile_types_are_filtered` for a server that did not. The paste works.
#[test]
fn hostile_x_selection_types_never_reach_wayland() {
    let Some((mut live, _, wayland)) = clipboard("hostile_x_selection_types_never_reach_wayland")
    else {
        return;
    };
    let payload = patterned(64);
    let _owner = Owner::start(
        live.display,
        "CLIPBOARD",
        X_UTF8,
        payload.clone(),
        OwnerManner {
            extra_targets: vec![
                b"text/evil\0type".to_vec(),
                format!("text/{}", "x".repeat(5000)).into_bytes(),
                b"SAVE_TARGETS".to_vec(),
            ],
            ..OwnerManner::default()
        },
    );
    live.drain();
    focus(&mut live, wayland);
    let expected = Some(vec![UTF8.to_owned(), "text/evil".to_owned()]);
    assert_eq!(control_offered(&mut live, Which::Clipboard), expected);
    assert_eq!(offered(&mut live, Which::Clipboard), expected);
    assert_eq!(
        receive(&mut live, Which::Clipboard).as_deref(),
        Ok(payload.as_slice())
    );
}

/// An X owner that quits takes its selection off the Wayland clipboard
/// too: nothing is left advertising a paste nobody can serve.
#[test]
fn an_x_owner_quitting_clears_the_wayland_selection_it_set() {
    let Some((mut live, _, _)) =
        clipboard("an_x_owner_quitting_clears_the_wayland_selection_it_set")
    else {
        return;
    };
    let owner = Owner::start(
        live.display,
        "CLIPBOARD",
        X_UTF8,
        patterned(64),
        OwnerManner::default(),
    );
    live.drain();
    assert!(control_offered(&mut live, Which::Clipboard).is_some());
    drop(owner);
    live.drain();
    assert_eq!(
        control_offered(&mut live, Which::Clipboard),
        None,
        "the X owner is gone but its selection is still on the Wayland clipboard"
    );
}

/// A Wayland copy made while a stuck X client is reading the old one still
/// crosses: the stuck read ties up nothing the next selection needs.
/// Also the bound: a stuck X reader holds the compositor to a bounded
/// buffer, not the whole Wayland source -- see `a_stuck_x_reader_*`.
#[test]
fn a_stuck_x_reader_does_not_make_the_compositor_buffer_the_whole_source() {
    let Some((mut live, x, wayland)) =
        clipboard("a_stuck_x_reader_does_not_make_the_compositor_buffer_the_whole_source")
    else {
        return;
    };
    // Far more than any bound the compositor may hold for one transfer.
    let payload = patterned(64 * 1024 * 1024);
    focus(&mut live, wayland);
    set(&mut live, Which::Clipboard, payload);
    live.drain();
    focus(&mut live, x);
    let reader = Reader::start(live.display, "CLIPBOARD", X_UTF8, Pace::Never);
    assert!(
        reader.finish(&mut live.fixture).is_err(),
        "the stuck reader is stuck"
    );
    // Give the compositor every chance to keep draining the source.
    for _ in 0..200 {
        live.fixture.settle();
    }
    let Ack::Count(written) = clip(&mut live, ClipStep::Written) else {
        panic!("expected a count");
    };
    assert!(
        written < 4 * 1024 * 1024,
        "the compositor drained {written} bytes of the Wayland source for an X reader that took none"
    );
    // And the loop still serves: a new Wayland copy crosses to X.
    focus(&mut live, wayland);
    set(&mut live, Which::Clipboard, patterned(64));
    live.drain();
    focus(&mut live, x);
    let fresh = Reader::start(live.display, "CLIPBOARD", X_UTF8, Pace::Normal);
    assert_eq!(
        fresh.finish(&mut live.fixture),
        Ok(Read::Data(patterned(64).to_vec()))
    );
    drop(reader);
}

fn ended(live: &mut Live) -> Vec<bool> {
    match clip(live, ClipStep::Ended) {
        Ack::Ends(ends) => ends,
        other => panic!("expected read ends, got {other:?}"),
    }
}

/// An X owner that never answers a conversion leaves every paste of it
/// waiting -- and each waiting paste holds a file descriptor and an X window
/// in the compositor. They are bounded: past eight in flight a paste is
/// refused at once (it reads nothing), and the owner changing hands ends
/// the ones still waiting on it, after which a paste works again.
#[test]
fn pastes_waiting_on_a_silent_x_owner_are_bounded() {
    let Some((mut live, x, wayland)) = clipboard("pastes_waiting_on_a_silent_x_owner_are_bounded")
    else {
        return;
    };
    let silent = Owner::start(
        live.display,
        "CLIPBOARD",
        X_UTF8,
        patterned(64),
        OwnerManner {
            answers_data: false,
            ..OwnerManner::default()
        },
    );
    live.drain();
    focus(&mut live, wayland);
    for _ in 0..12 {
        assert!(matches!(
            clip(
                &mut live,
                ClipStep::ReceiveLater {
                    which: Which::Clipboard,
                    mime: UTF8
                }
            ),
            Ack::Done
        ));
    }
    live.drain();
    let ends = ended(&mut live);
    assert_eq!(
        ends.iter().filter(|&&ended| !ended).count(),
        8,
        "pastes waiting on a silent X owner are not bounded at eight: {ends:?}"
    );
    assert!(ends[..8].iter().all(|&ended| !ended), "{ends:?}");
    // A new owner (taking the selection while the X window is focused, so it
    // crosses) ends the pastes still waiting on the old one...
    focus(&mut live, x);
    let payload = patterned(100);
    let _answering = Owner::start(
        live.display,
        "CLIPBOARD",
        X_UTF8,
        payload.clone(),
        OwnerManner::default(),
    );
    live.drain();
    assert!(silent.lost());
    assert!(
        ended(&mut live).iter().all(|&ended| ended),
        "pastes waiting on an owner that lost the selection were never ended"
    );
    // ... and pasting works again.
    focus(&mut live, wayland);
    assert_eq!(
        receive(&mut live, Which::Clipboard).as_deref(),
        Ok(payload.as_slice())
    );
}

/// A clipboard manager (or `wl-copy`) setting the clipboard through
/// data-control while an X window is focused -- no focus change anywhere to
/// flush anything along the way -- reaches the very next X read: the X
/// client reads the new selection, not the X owner the Wayland side just
/// replaced. (Found live: `xclip -o` straight after `wl-copy` read the
/// previous `xclip`'s selection, because the window manager queued its
/// ownership change without sending it.)
#[test]
fn a_data_control_copy_reaches_the_next_x_read() {
    let Some((mut live, _, _)) = clipboard("a_data_control_copy_reaches_the_next_x_read") else {
        return;
    };
    let old = Owner::start(
        live.display,
        "CLIPBOARD",
        X_UTF8,
        Arc::new(b"the previous X copy".to_vec()),
        OwnerManner::default(),
    );
    // Let every bit of window manager traffic the setup caused die down, so
    // nothing incidental flushes its connection later.
    for _ in 0..5 {
        live.drain();
    }
    let payload = Arc::new(b"copied by a clipboard manager".to_vec());
    // Acknowledged without the settle `run` adds: the X read goes out the
    // moment the Wayland copy has been handled, before any unrelated window
    // manager traffic could happen to flush the ownership change for it.
    live.fixture.send_step(
        0,
        Step::Clip(ClipStep::ControlSet {
            mime: UTF8,
            payload: payload.clone(),
        }),
    );
    assert!(matches!(live.fixture.wait_for_ack(0), Ack::Done));
    let reader = Reader::start(live.display, "CLIPBOARD", X_UTF8, Pace::Normal);
    assert_eq!(
        reader.finish(&mut live.fixture),
        Ok(Read::Data(payload.to_vec()))
    );
    assert!(old.lost(), "the previous X owner never lost the clipboard");
}
