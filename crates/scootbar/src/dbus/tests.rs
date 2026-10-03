//! The bus client without a bus: address parsing, validators, and wire
//! round-trips through the writer and the parser. Anything needing a peer
//! (auth, `Hello`, calls) runs against the tray's fake bus
//! (`modules/tray/fake.rs`), which speaks the same bytes.

use std::ffi::OsStr;

use super::conn::bus_path_for;
use super::proto::{
    Kind, Message, Writer, check_body_signature, check_member, check_name, check_path,
    check_signature, frame_at,
};

#[test]
fn the_bus_address_names_a_path_or_falls_through() {
    assert_eq!(
        bus_path_for(Some(OsStr::new("unix:path=/run/user/1000/bus"))),
        std::path::Path::new("/run/user/1000/bus")
    );
    assert_eq!(
        bus_path_for(Some(OsStr::new(
            "unix:path=/sock,guid=abc;unix:path=/other"
        ))),
        std::path::Path::new("/sock")
    );
    // Abstract sockets are not dialled: falls through to the default.
    let abstracted = bus_path_for(Some(OsStr::new("unix:abstract=/tmp/dbus-XXXX,guid=abc")));
    assert!(abstracted.ends_with("bus"));
    // No address, or one without a path: the runtime default.
    for address in [None, Some(OsStr::new("")), Some(OsStr::new("autolaunch:"))] {
        let path = bus_path_for(address);
        assert!(path.ends_with("bus"), "{}", path.display());
        assert!(path.is_absolute());
    }
}

#[test]
fn a_call_round_trips_through_framing_and_the_parser() {
    let mut writer = Writer::new();
    writer.begin_call(
        42,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "Hello",
        "",
        0,
    );
    let message = writer.finish().unwrap();
    assert_eq!(frame_at(&message), Ok(Some(message.len())));
    assert_eq!(frame_at(&message[..message.len() / 2]), Ok(None));
    let parsed = Message::parse(&message).unwrap();
    assert_eq!(parsed.kind, Kind::MethodCall);
    assert_eq!(parsed.serial, 42);
    assert_eq!(parsed.destination, Some("org.freedesktop.DBus"));
    assert_eq!(parsed.path, Some("/org/freedesktop/DBus"));
    assert_eq!(parsed.interface, Some("org.freedesktop.DBus"));
    assert_eq!(parsed.member, Some("Hello"));
    assert_eq!(parsed.signature, "");
    assert!(parsed.body.exhausted());
}

#[test]
fn a_body_round_trips_typed() {
    let mut writer = Writer::new();
    writer.begin_call(
        1,
        ":1.7",
        "/StatusNotifierItem",
        "org.kde.StatusNotifierItem",
        "Activate",
        "ii",
        0,
    );
    writer.i32(100);
    writer.i32(-50);
    let message = writer.finish().unwrap();
    let parsed = Message::parse(&message).unwrap();
    assert_eq!(parsed.signature, "ii");
    let mut body = parsed.body;
    assert_eq!(body.i32().unwrap(), 100);
    assert_eq!(body.i32().unwrap(), -50);
    assert!(body.exhausted());
}

#[test]
fn every_reply_shape_round_trips() {
    // What `reply_return` and `reply_error` emit (round trip and error,
    // empty and Bodied): the parser takes them back, against the
    // signatures a peer validates.
    for (serial, to, sig, body) in [
        (9u32, 2u32, "", vec![]),
        (10, 3, "u", {
            let mut body = Writer::new();
            body.u32(1);
            body.take_body().unwrap()
        }),
        (11, 4, "a{sv}", {
            let mut body = Writer::new();
            let Some(cookie) = body.open_array(8) else {
                panic!("fits");
            };
            body.close_array(cookie);
            body.take_body().unwrap()
        }),
    ] {
        let mut writer = Writer::new();
        writer.begin_return(serial, to, sig);
        writer.raw(&body);
        let message = writer.finish().expect("builds");
        assert_eq!(frame_at(&message), Ok(Some(message.len())));
        let parsed = Message::parse(&message).unwrap();
        assert_eq!(parsed.kind, Kind::MethodReturn);
        assert_eq!(parsed.reply_serial, Some(to));
        assert_eq!(parsed.signature, sig);
        assert_eq!(parsed.body.rest(), body.as_slice());
    }
    // And an error reply.
    let mut writer = Writer::new();
    writer.begin_error(12, 5, "org.freedesktop.DBus.Error.UnknownMethod", "");
    let message = writer.finish().expect("builds");
    let parsed = Message::parse(&message).unwrap();
    assert_eq!(parsed.kind, Kind::Error);
    assert_eq!(
        parsed.error,
        Some("org.freedesktop.DBus.Error.UnknownMethod")
    );
}

#[test]
fn framing_refuses_without_a_panic() {
    // Bad magic, bad version, bad kind.
    for header in [
        [0u8, 1, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0],
        [b'l', 1, 0, 2, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0],
        [b'l', 9, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0],
    ] {
        assert_eq!(frame_at(&header), Err(()));
    }
    // A length past the cap.
    let mut huge = [b'l', 1, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0];
    huge[4..8].copy_from_slice(&0x200_0000u32.to_le_bytes());
    assert_eq!(frame_at(&huge), Err(()));
    // Short reads wait.
    assert_eq!(frame_at(b""), Ok(None));
    assert_eq!(frame_at(b"l"), Ok(None));
}

#[test]
fn names_paths_members_and_signatures_validate() {
    for valid in [
        ":1.7",
        "org.kde.StatusNotifierWatcher",
        "org.freedesktop.StatusNotifierItem-4077-1",
    ] {
        assert_eq!(check_name(valid), Ok(valid));
    }
    // Dashes ride along in bus names (never leading, never in member or
    // interface names, which `check_member` holds).
    assert_eq!(check_name("a-b.c-d"), Ok("a-b.c-d"));
    assert_eq!(check_name("-a.b"), Err(()));
    for invalid in [
        "",
        "no-dots",
        ".leading",
        "trailing.",
        "has space",
        "9lives.lead-digit",
        &"x".repeat(129),
    ] {
        assert_eq!(check_name(invalid), Err(()), "{invalid}");
    }
    for valid in ["/", "/StatusNotifierWatcher", "/org/freedesktop/DBus"] {
        assert_eq!(check_path(valid), Ok(valid));
    }
    for invalid in [
        "",
        "relative",
        "//double",
        "/trailing/",
        "/has space",
        "/has-dash",
    ] {
        assert_eq!(check_path(invalid), Err(()), "{invalid}");
    }
    assert_eq!(check_member("Hello"), Ok("Hello"));
    assert_eq!(check_member("NewStatus"), Ok("NewStatus"));
    for invalid in ["", "has.dot", "has-dash", "9lives"] {
        assert_eq!(check_member(invalid), Err(()), "{invalid}");
    }
    for valid in ["s", "u", "as", "a{sv}", "a(iiay)", "(sa(iiay)ss)"] {
        assert_eq!(check_signature(valid), Ok(valid));
    }
    for invalid in [
        "",
        "{}",
        "()",
        "a",
        "{ss",
        "(s",
        "a{ssv}",
        "xs",
        "su",
        &"(".repeat(40),
    ] {
        assert_eq!(check_signature(invalid), Err(()), "{invalid}");
    }
    // Body signatures concatenate: empty and sequences pass here, never
    // as a single type above.
    for valid in ["", "s", "su", "sss", "a{sv}", "sa{sv}as"] {
        assert_eq!(check_body_signature(valid), Ok(valid));
    }
    for invalid in ["{}", "a", &"a".repeat(300)] {
        assert_eq!(check_body_signature(invalid), Err(()), "{invalid}");
    }
}

#[test]
fn the_writer_overflows_instead_of_growing_past_the_cap() {
    let mut writer = Writer::new();
    writer.begin_call(1, "x", "/", "x", "y", "ay", 0);
    if let Some(cookie) = writer.open_array(1) {
        for _ in 0..super::proto::MAX_MESSAGE {
            writer.u8(0xFF);
            if writer.overflowed() {
                break;
            }
        }
        writer.close_array(cookie);
    }
    assert!(writer.overflowed());
    assert_eq!(writer.finish(), None);
}

#[test]
fn the_writer_refuses_a_signature_past_its_bound() {
    let mut writer = Writer::new();
    writer.signature(&"a".repeat(300));
    assert!(writer.overflowed());
}

/// A whole message marshalled by sd-bus (see the tray test of the same
/// fixture): framed, parsed, and its body skipped end to end by the
/// signature walk, which meets variants at offsets that are not
/// multiples of 8.
#[test]
fn a_message_marshalled_by_sd_bus_frames_parses_and_skips() {
    let frame = include_bytes!("fixtures/sdbus-getall-call.bin");
    assert_eq!(frame_at(frame), Ok(Some(frame.len())));
    let mut message = Message::parse(frame).unwrap();
    assert_eq!(message.kind, Kind::MethodCall);
    assert_eq!(message.member, Some("GetAll"));
    assert_eq!(message.signature, "a{sv}");
    message.body.skip("a{sv}").unwrap();
    assert!(message.body.exhausted());
}

/// Every byte-order flag but little-endian is a dropped message: the
/// body readers take bytes with no byte order beside them.
#[test]
fn a_big_endian_message_is_framed_but_not_parsed() {
    let mut frame = include_bytes!("fixtures/sdbus-getall-call.bin").to_vec();
    frame[0] = b'B';
    // The lengths are little-endian bytes now read big-endian: framing
    // refuses the absurd length or waits for more, and `parse` refuses.
    assert!(Message::parse(&frame).is_err());
}

// The connection against a scripted peer: a socketpair, with a thread
// that plays the daemon's side of the set-up (or fails to).

use std::io::Read;
use std::io::Write;
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use super::conn::{self, CallError, MAX_PENDING, SetupError};

/// Plays the daemon through auth and `Hello`: the client's end is
/// returned set up, with the daemon's end still ours.
fn connected() -> (conn::Conn, UnixStream) {
    let (client, mut daemon) = UnixStream::pair().unwrap();
    let server = std::thread::spawn(move || {
        super::testdaemon::serve_setup(&mut daemon);
        daemon
    });
    let conn = conn::setup(client).unwrap();
    (conn, server.join().unwrap())
}

#[test]
fn setup_on_a_scripted_daemon_names_us() {
    let (conn, _daemon) = connected();
    assert_eq!(conn.unique(), ":1.7");
    assert!(!conn.dead());
}

/// A bus that accepts and never answers (a stopped daemon) costs the
/// set-up its total deadline once, not a read timeout per step.
#[test]
fn setup_gives_up_on_a_silent_bus() {
    let (client, _daemon) = UnixStream::pair().unwrap();
    let start = Instant::now();
    let error = conn::setup(client).unwrap_err();
    assert!(matches!(error, SetupError::Refused(_)), "{error}");
    assert!(
        start.elapsed() < Duration::from_secs(4),
        "{:?}",
        start.elapsed()
    );
}

/// A bus that answers a byte at a time cannot stretch the deadline: the
/// per-read timeout would never fire here, the total one must.
#[test]
fn setup_gives_up_on_a_bus_that_trickles() {
    let (client, mut daemon) = UnixStream::pair().unwrap();
    let trickle = std::thread::spawn(move || {
        let mut byte = [0u8; 1];
        // Take the NUL, so the client's writes do not block.
        let _ = daemon.read(&mut byte);
        for _ in 0..40 {
            if daemon.write_all(b"D").is_err() {
                return;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    });
    let start = Instant::now();
    let error = conn::setup(client).unwrap_err();
    assert!(matches!(error, SetupError::Refused(_)), "{error}");
    assert!(
        start.elapsed() < Duration::from_secs(4),
        "{:?}",
        start.elapsed()
    );
    trickle.join().unwrap();
}

/// The pending table is bounded, and a call nobody answers is forgotten
/// by age, returning its token, so the table is not held for good.
#[test]
fn unanswered_calls_expire_and_free_their_slots() {
    let (mut conn, _daemon) = connected();
    for token in 0..MAX_PENDING as u64 {
        conn.call("a.b", "/", "a.b", "M", "", &[], 0, token)
            .unwrap();
    }
    assert_eq!(
        conn.call("a.b", "/", "a.b", "M", "", &[], 0, 99),
        Err(CallError::Full)
    );
    // Nothing is old enough yet.
    assert!(conn.expire(Duration::from_secs(60)).is_empty());
    let mut tokens = conn.expire(Duration::ZERO);
    tokens.sort_unstable();
    assert_eq!(tokens, (0..MAX_PENDING as u64).collect::<Vec<_>>());
    conn.call("a.b", "/", "a.b", "M", "", &[], 0, 99).unwrap();
}

/// A bus that stops reading while we owe it output is a dead
/// connection at the cap, not an outbox growing for the life of the bar.
#[test]
fn a_bus_that_stops_reading_kills_the_connection() {
    let (mut conn, _daemon) = connected();
    let mut body = Writer::new();
    body.str(&"x".repeat(900_000));
    let body = body.take_body().unwrap();
    for _ in 0..6 {
        conn.signal("/s", "a.b", "S", "s", &body);
    }
    let _ = conn.pump();
    assert!(conn.dead());
}

/// An `a{sv}` body of `count` properties, each `key` as a variant of
/// signature `sig` whose value `write` writes.
fn dict(count: usize, key: &str, sig: &str, write: impl Fn(&mut Writer)) -> Vec<u8> {
    let mut body = Writer::new();
    let cookie = body.open_array(8).unwrap();
    for _ in 0..count {
        assert!(body.open_struct());
        body.str(key);
        body.variant(sig);
        write(&mut body);
        body.close_struct();
    }
    body.close_array(cookie);
    body.take_body().unwrap()
}

#[test]
fn an_item_answer_is_read_typed_and_refused_whole_when_hostile() {
    use super::proto::{MAX_PROPERTIES, read_item_props};
    // One property of a known key and type.
    let good = dict(1, "Title", "s", |w| w.str("hello"));
    assert_eq!(read_item_props(&good).unwrap().title, Some("hello"));
    // The same key with another type is an unknown property: skipped.
    let wrong = dict(1, "Title", "u", |w| w.u32(7));
    assert_eq!(read_item_props(&wrong).unwrap().title, None);
    // As many properties as the cap are read, one more is refused.
    let at_cap = dict(MAX_PROPERTIES, "Extra", "u", |w| w.u32(1));
    assert!(read_item_props(&at_cap).is_ok());
    let past = dict(MAX_PROPERTIES + 1, "Extra", "u", |w| w.u32(1));
    assert!(read_item_props(&past).is_err());
    // Bytes after the dictionary are refused, a cut one too.
    let mut trailing = good.clone();
    trailing.extend_from_slice(&[0; 8]);
    assert!(read_item_props(&trailing).is_err());
    assert!(read_item_props(&good[..good.len() - 3]).is_err());
    assert!(read_item_props(&[]).is_err());
    // A pixmap whose side is past the bound is dropped from the list, the
    // answer still read.
    let big = dict(1, "IconPixmap", "a(iiay)", |w| {
        let list = w.open_array(8).unwrap();
        w.open_struct();
        w.u32(300);
        w.u32(300);
        let bytes = w.open_array(1).unwrap();
        w.raw(&vec![0u8; 300 * 300 * 4]);
        w.close_array(bytes);
        w.close_struct();
        w.close_array(list);
    });
    // 360 KB fits under the message cap; the entry is skipped.
    assert_eq!(read_item_props(&big).unwrap().pixmaps.unwrap().len(), 0);
}
