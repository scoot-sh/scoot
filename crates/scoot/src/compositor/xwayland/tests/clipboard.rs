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
use super::xsel::{DataManner, Owner, OwnerManner, Pace, Read, Reader, flood, patterned};
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
    // No echo: had the bridge announced the crossed selection back to X,
    // the window manager would have taken it from the X owner.
    assert!(
        !owner.lost(),
        "the bridge took the selection back from its X owner"
    );
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
    assert!(
        !owner.lost(),
        "the X owner lost its selection to the bridge"
    );
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

fn ended(live: &mut Live) -> Vec<(bool, usize)> {
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
        ends.iter().filter(|&&(ended, _)| !ended).count(),
        8,
        "pastes waiting on a silent X owner are not bounded at eight: {ends:?}"
    );
    assert!(ends[..8].iter().all(|&(ended, _)| !ended), "{ends:?}");
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
        ended(&mut live).iter().all(|&(ended, _)| ended),
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

/// The other direction's bound: a Wayland client that asks for a large X
/// selection and never reads it holds the X owner to a few chunks -- the
/// window manager asks for the next chunk only once it has written the last
/// one into the reader's pipe -- rather than pulling the whole selection
/// into the compositor. (Before the fork's pacing fix the second chunk was
/// deleted unread and the transfer ended at one chunk instead.)
#[test]
fn a_stuck_wayland_reader_holds_the_x_owner_to_a_few_chunks() {
    let Some((mut live, x, wayland)) =
        clipboard("a_stuck_wayland_reader_holds_the_x_owner_to_a_few_chunks")
    else {
        return;
    };
    let payload = patterned(16 * 1024 * 1024);
    let owner = Owner::start(
        live.display,
        "CLIPBOARD",
        X_UTF8,
        payload,
        OwnerManner::default(),
    );
    live.drain();
    focus(&mut live, wayland);
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
    for _ in 0..200 {
        live.fixture.settle();
    }
    let sent = owner.sent();
    assert!(
        sent > 0,
        "the transfer never started: nothing to measure the bound against"
    );
    assert!(
        sent < 1024 * 1024,
        "the X owner handed out {sent} bytes to a Wayland reader that read none"
    );
    // The loop still serves: a paste of a fresh, small X copy works.
    focus(&mut live, x);
    let fresh = Arc::new(b"after the stuck reader".to_vec());
    let _fresh = Owner::start(
        live.display,
        "CLIPBOARD",
        X_UTF8,
        fresh.clone(),
        OwnerManner::default(),
    );
    live.drain();
    focus(&mut live, wayland);
    assert_eq!(
        receive(&mut live, Which::Clipboard).as_deref(),
        Ok(fresh.as_slice())
    );
}

fn receive_later(live: &mut Live) {
    assert!(matches!(
        clip(
            live,
            ClipStep::ReceiveLater {
                which: Which::Clipboard,
                mime: UTF8
            }
        ),
        Ack::Done
    ));
}

/// Longer than the window manager's grace for a transfer stalled on its
/// owner when the selection changes hands (`OWNER_CHANGE_GRACE`, 1 s).
fn outlast_the_owner_change_grace(live: &mut Live) {
    live.fixture.tick(std::time::Duration::from_millis(1300));
}

/// The owner-borrowing hijack. `SetSelectionOwner` accepts any window id, and
/// conversions go to whoever made the request -- so a background X client
/// can take the clipboard *under the approved owner's own window id*, and
/// the owner the window manager tracks does not change. Answering no
/// `TARGETS`, it is never announced either. Its bytes must still not reach
/// a Wayland paste: any change of hands since the copy was let through
/// refuses the paste.
#[test]
fn a_background_client_borrowing_the_owners_window_cannot_serve_a_paste() {
    let Some((mut live, _, wayland)) =
        clipboard("a_background_client_borrowing_the_owners_window_cannot_serve_a_paste")
    else {
        return;
    };
    let copy = Owner::start(
        live.display,
        "CLIPBOARD",
        X_UTF8,
        Arc::new(b"copied in the X window".to_vec()),
        OwnerManner::default(),
    );
    live.drain();
    focus(&mut live, wayland);
    assert!(offered(&mut live, Which::Clipboard).is_some());
    let intruder = Arc::new(b"PWNED-BY-BACKGROUND-X".to_vec());
    let hidden = Owner::start(
        live.display,
        "CLIPBOARD",
        X_UTF8,
        intruder.clone(),
        OwnerManner {
            answers_targets: false,
            borrow_owner_window: true,
            ..OwnerManner::default()
        },
    );
    live.drain();
    assert!(copy.lost(), "the borrowing client never took the clipboard");
    let got = receive(&mut live, Which::Clipboard);
    assert!(
        got.as_deref() != Ok(intruder.as_slice()),
        "a paste delivered the bytes of a client that borrowed the approved owner's window"
    );
    assert_eq!(
        hidden.served(),
        0,
        "the borrowing client served a Wayland paste"
    );
}

/// An owner that announces `INCR` and then appends its data without waiting
/// for a single delete. Each append is a new value; none of them is a chunk
/// the window manager asked for. What reaches the reader is the owner's
/// bytes, each at most once -- not the property re-read on every append
/// (which handed the reader the same bytes again and again, and grew the
/// compositor's buffer with them). The transfer then waits on an owner that
/// sends nothing more, and a change of owner ends it.
#[test]
fn an_owner_appending_without_waiting_is_read_once() {
    let Some((mut live, x, wayland)) = clipboard("an_owner_appending_without_waiting_is_read_once")
    else {
        return;
    };
    let payload = patterned(4 * 1024 * 1024);
    let _owner = Owner::start(
        live.display,
        "CLIPBOARD",
        X_UTF8,
        payload.clone(),
        OwnerManner {
            data: DataManner::AppendWithoutWaiting,
            ..OwnerManner::default()
        },
    );
    live.drain();
    focus(&mut live, wayland);
    receive_later(&mut live);
    // Read it all as it comes, while the compositor runs; the transfer then
    // waits for a next chunk the owner never sends, and a new owner ends it.
    live.fixture
        .send_step(0, Step::Clip(ClipStep::DrainLater(0)));
    outlast_the_owner_change_grace(&mut live);
    focus(&mut live, x);
    let _next = Owner::start(
        live.display,
        "CLIPBOARD",
        X_UTF8,
        patterned(16),
        OwnerManner::default(),
    );
    let got = match live.fixture.wait_for_ack(0) {
        Ack::Bytes(Ok(got)) => got,
        other => panic!("the stalled read never ended: {other:?}"),
    };
    assert!(
        got.len() <= payload.len(),
        "the reader got {} bytes of a {}-byte payload: bytes were read more than once",
        got.len(),
        payload.len()
    );
    assert!(
        got[..] == payload[..got.len()],
        "the reader's bytes are not the owner's, in order"
    );
    assert!(!got.is_empty(), "nothing of the appends reached the reader");
}

/// An owner answering with a single property far larger than one read --
/// built by appends, past the X request size -- reaches the reader intact:
/// the window manager reads it a slice at a time, never all at once.
#[test]
fn a_single_huge_property_is_streamed_intact() {
    let Some((mut live, _, wayland)) = clipboard("a_single_huge_property_is_streamed_intact")
    else {
        return;
    };
    let payload = patterned(8 * 1024 * 1024 + 4321);
    let _owner = Owner::start(
        live.display,
        "CLIPBOARD",
        X_UTF8,
        payload.clone(),
        OwnerManner {
            data: DataManner::SingleProperty,
            ..OwnerManner::default()
        },
    );
    live.drain();
    focus(&mut live, wayland);
    let got = receive(&mut live, Which::Clipboard).expect("the paste completes");
    assert_eq!(
        got.len(),
        payload.len(),
        "the paste was truncated or padded"
    );
    assert!(
        got == *payload,
        "the paste's bytes differ from the X owner's"
    );
}

/// One X client asking for the Wayland clipboard from many windows at once,
/// never taking the data: only a few conversions are opened (each holds a
/// pipe and up to two chunks until taken), the rest refused at once.
#[test]
fn an_x_client_opens_only_a_few_transfers_at_once() {
    let Some((mut live, x, wayland)) = clipboard("an_x_client_opens_only_a_few_transfers_at_once")
    else {
        return;
    };
    focus(&mut live, wayland);
    set(&mut live, Which::Clipboard, patterned(4 * 1024 * 1024));
    live.drain();
    focus(&mut live, x);
    let (answered, refused) = flood(&mut live.fixture, live.display, "CLIPBOARD", X_UTF8, 12);
    assert_eq!(
        (answered, refused),
        (4, 8),
        "one X client opened {answered} transfers out of 12 requests"
    );
}

/// Pastes stalled by an owner that answered `INCR` and went silent do not
/// hold the bound for good. Past eight a paste is refused (it reads nothing,
/// it is not queued); a reader closing frees its slot; and a new owner taking
/// the selection ends the pastes still stalled on the old one, after which
/// pasting works.
#[test]
fn stalled_pastes_do_not_hold_the_bound_for_good() {
    let Some((mut live, x, wayland)) = clipboard("stalled_pastes_do_not_hold_the_bound_for_good")
    else {
        return;
    };
    let _silent = Owner::start(
        live.display,
        "CLIPBOARD",
        X_UTF8,
        patterned(1024 * 1024),
        OwnerManner {
            data: DataManner::StallAfterIncr,
            ..OwnerManner::default()
        },
    );
    live.drain();
    focus(&mut live, wayland);
    for _ in 0..8 {
        receive_later(&mut live);
    }
    live.drain();
    let ends = ended(&mut live);
    assert!(ends.iter().all(|&(ended, _)| !ended), "{ends:?}");
    receive_later(&mut live);
    live.drain();
    assert_eq!(
        ended(&mut live)[8],
        (true, 0),
        "a ninth paste past the bound should read nothing at once"
    );
    // The eight readers go away; their slots come back.
    assert!(matches!(clip(&mut live, ClipStep::DropLater), Ack::Done));
    receive_later(&mut live);
    live.drain();
    assert!(
        !ended(&mut live)[9].0,
        "the bound was still full of transfers whose readers had gone"
    );
    // A new owner ends the paste still stalled on the old one...
    outlast_the_owner_change_grace(&mut live);
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
    assert!(
        ended(&mut live)[9].0,
        "a paste stalled on the previous owner outlived the change of owner"
    );
    // ... and pasting works.
    focus(&mut live, wayland);
    assert_eq!(
        receive(&mut live, Which::Clipboard).as_deref(),
        Ok(payload.as_slice())
    );
}

/// The other half of that rule: a paste that is moving -- waiting on its
/// reader, not its owner -- is not ended by a change of owner, even when it
/// has been idle a while; ending it would hand the reader part of the
/// selection as if it were all of it.
#[test]
fn a_moving_paste_survives_a_change_of_owner() {
    let Some((mut live, x, wayland)) = clipboard("a_moving_paste_survives_a_change_of_owner")
    else {
        return;
    };
    let payload = patterned(2 * 1024 * 1024);
    let _first = Owner::start(
        live.display,
        "CLIPBOARD",
        X_UTF8,
        payload.clone(),
        OwnerManner::default(),
    );
    live.drain();
    focus(&mut live, wayland);
    receive_later(&mut live);
    live.drain();
    outlast_the_owner_change_grace(&mut live);
    focus(&mut live, x);
    let _second = Owner::start(
        live.display,
        "CLIPBOARD",
        X_UTF8,
        patterned(16),
        OwnerManner::default(),
    );
    live.drain();
    let got = match clip(&mut live, ClipStep::DrainLater(0)) {
        Ack::Bytes(Ok(got)) => got,
        other => panic!("the paste did not complete: {other:?}"),
    };
    assert_eq!(got.len(), payload.len(), "the paste was cut short");
    assert!(
        got == *payload,
        "the paste's bytes differ from the first owner's"
    );
}
/// A transfer that has not moved for the window manager's transfer timeout is
/// dropped the next time it looks -- here, when the next paste is answered.
#[test]
fn an_idle_transfer_is_dropped_after_the_timeout() {
    let Some((mut live, _, wayland)) = clipboard("an_idle_transfer_is_dropped_after_the_timeout")
    else {
        return;
    };
    live.fixture
        .state
        .xwm
        .as_mut()
        .expect("a window manager")
        .set_selection_transfer_timeout(std::time::Duration::from_millis(300));
    let _silent = Owner::start(
        live.display,
        "CLIPBOARD",
        X_UTF8,
        patterned(1024 * 1024),
        OwnerManner {
            data: DataManner::StallAfterIncr,
            ..OwnerManner::default()
        },
    );
    live.drain();
    focus(&mut live, wayland);
    receive_later(&mut live);
    live.drain();
    assert!(!ended(&mut live)[0].0);
    live.fixture.tick(std::time::Duration::from_millis(500));
    receive_later(&mut live);
    live.drain();
    let ends = ended(&mut live);
    assert!(ends[0].0, "an idle transfer outlived the timeout: {ends:?}");
    assert!(!ends[1].0, "the fresh transfer was dropped too: {ends:?}");
}
