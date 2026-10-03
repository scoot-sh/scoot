//! The bus client without a bus: address parsing, validators, and wire
//! round-trips through the writer and the parser. Anything needing a peer
//! (auth, `Hello`, calls) runs against the tray's fake bus
//! (`modules/tray/fake.rs`), which speaks the same bytes.

use std::ffi::OsStr;

use super::conn::{bus_path_for, system_bus_path_for};
use super::proto::{
    Kind, Message, Writer, check_body_signature, check_interface, check_member, check_name,
    check_path, check_signature, frame_at,
};

#[test]
fn the_bus_address_names_a_path_or_is_refused() {
    let path = |text| bus_path_for(Some(OsStr::new(text)));
    assert_eq!(
        path("unix:path=/run/user/1000/bus"),
        Ok("/run/user/1000/bus".into())
    );
    assert_eq!(
        path("unix:path=/sock,guid=abc;unix:path=/other"),
        Ok("/sock".into())
    );
    // The address's own escapes are decoded; a broken one is kept.
    assert_eq!(path("unix:path=/tmp/a%20b%2Fc"), Ok("/tmp/a b/c".into()));
    assert_eq!(path("unix:path=/tmp/100%"), Ok("/tmp/100%".into()));
    assert_eq!(path("unix:path=/tmp/%zz"), Ok("/tmp/%zz".into()));
    // A later address with a path serves when an earlier one has none.
    assert_eq!(
        path("unix:abstract=/tmp/dbus-X;unix:path=/second"),
        Ok("/second".into())
    );
    // Set but with no path (abstract, tcp, autolaunch): refused, not
    // replaced by whatever bus is at the default place.
    for address in [
        "unix:abstract=/tmp/dbus-XXXX,guid=abc",
        "autolaunch:",
        "tcp:host=localhost,port=1",
        "unix:path=",
        "garbage",
    ] {
        assert_eq!(path(address), Err(()), "{address}");
    }
    // Not set, or empty: the runtime default.
    for address in [None, Some(OsStr::new(""))] {
        let path = bus_path_for(address).unwrap();
        assert!(path.ends_with("bus"), "{}", path.display());
        assert!(path.is_absolute());
    }
}

#[test]
fn the_system_bus_address_names_a_path_or_falls_back() {
    use super::conn::SYSTEM_BUS_PATH;
    use std::path::PathBuf;
    let path = |text| system_bus_path_for(Some(OsStr::new(text)));
    assert_eq!(
        path("unix:path=/run/dbus/system_bus_socket"),
        PathBuf::from(SYSTEM_BUS_PATH)
    );
    assert_eq!(path("unix:path=/sock,guid=abc"), PathBuf::from("/sock"));
    // Escapes decoded, as on the session bus.
    assert_eq!(path("unix:path=/tmp/a%20b"), PathBuf::from("/tmp/a b"));
    // Set but with no path: the socket, not a refusal (the system bus has
    // a fixed place).
    for address in [
        "unix:abstract=/tmp/dbus-XXXX,guid=abc",
        "tcp:host=localhost,port=1",
        "unix:path=",
        "garbage",
    ] {
        assert_eq!(path(address), PathBuf::from(SYSTEM_BUS_PATH), "{address}");
    }
    // Not set, or empty: the socket.
    for address in [None, Some(OsStr::new(""))] {
        assert_eq!(system_bus_path_for(address), PathBuf::from(SYSTEM_BUS_PATH));
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
    // Dashes ride along in bus names, a leading one too (the spec allows
    // it and a daemon takes `org.example.-x`), never in member or
    // interface names, which `check_member` and `check_interface` hold.
    assert_eq!(check_name("a-b.c-d"), Ok("a-b.c-d"));
    assert_eq!(check_name("-a.b"), Ok("-a.b"));
    assert_eq!(check_name("org.example.-lead"), Ok("org.example.-lead"));
    // The spec's own 255 bytes, not fewer.
    let longest = format!("org.{}", "x".repeat(251));
    assert_eq!(longest.len(), 255);
    assert_eq!(check_name(&longest), Ok(longest.as_str()));
    assert_eq!(check_name(&format!("{longest}x")), Err(()));
    assert_eq!(
        check_interface("org.kde.StatusNotifierItem"),
        Ok("org.kde.StatusNotifierItem")
    );
    for invalid in ["", "nodots", "org.has-dash", ":1.7", "org.9lives", "org..x"] {
        assert_eq!(check_interface(invalid), Err(()), "{invalid}");
    }
    for invalid in [
        "",
        "no-dots",
        ".leading",
        "trailing.",
        "has space",
        "9lives.lead-digit",
        &"x".repeat(256),
    ] {
        assert_eq!(check_name(invalid), Err(()), "{invalid}");
    }
    for valid in ["/", "/StatusNotifierWatcher", "/org/freedesktop/DBus"] {
        assert_eq!(check_path(valid), Ok(valid));
    }
    // A path is bounded: a few dozen bytes are real, a megabyte is not.
    let long = format!("/{}", "a".repeat(super::proto::MAX_PATH - 1));
    assert_eq!(check_path(&long), Ok(long.as_str()));
    assert_eq!(check_path(&format!("{long}a")), Err(()));
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

// A valid message past the size this client reads, and floods: neither
// may cost the connection (which was an item's `GetAll` with a 512 by 512
// pixmap turning the tray off for the session).

use super::conn::Event;

/// A valid method return of `len` array bytes, which `Writer`'s own cap
/// would refuse.
fn oversized_reply(serial: u32, reply_to: u32, len: usize) -> Vec<u8> {
    let mut writer = Writer::with_cap(len + 4096);
    writer.begin_return_to(serial, ":1.7", reply_to, "ay");
    let cookie = writer.open_array(1).unwrap();
    writer.raw(&vec![7u8; len]);
    writer.close_array(cookie);
    writer.finish().unwrap()
}

fn small_reply(serial: u32, reply_to: u32) -> Vec<u8> {
    let mut body = Writer::new();
    body.str("ok");
    let body = body.take_body().unwrap();
    let mut reply = Writer::new();
    reply.begin_return_to(serial, ":1.7", reply_to, "s");
    reply.raw(&body);
    reply.finish().unwrap()
}

/// Writes `bytes` to the daemon's end in its own thread (the connection
/// is pumped meanwhile: the socket buffer is smaller than the message).
fn send_later(mut daemon: UnixStream, bytes: Vec<u8>) -> std::thread::JoinHandle<UnixStream> {
    std::thread::spawn(move || {
        daemon.write_all(&bytes).unwrap();
        daemon
    })
}

/// Pumps until `want` events are collected (or fails after 20 s),
/// returning them and the most ever staged.
fn pump_events(conn: &mut conn::Conn, want: usize) -> (Vec<Event>, usize) {
    let start = Instant::now();
    let mut got = Vec::new();
    let mut peak = 0;
    while got.len() < want {
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "{} events",
            got.len()
        );
        assert!(
            !conn.dead(),
            "the connection died after {} events",
            got.len()
        );
        let (events, _) = conn.pump();
        peak = peak.max(conn.staged_len());
        got.extend(events);
        std::thread::sleep(Duration::from_millis(1));
    }
    (got, peak)
}

#[test]
fn an_over_cap_reply_is_skipped_and_the_connection_lives() {
    let (mut conn, daemon) = connected();
    let first = conn.call("a.b", "/", "a.b", "Big", "", &[], 0, 7).unwrap();
    let second = conn
        .call("a.b", "/", "a.b", "Small", "", &[], 0, 8)
        .unwrap();
    let _ = conn.pump();
    let mut bytes = oversized_reply(100, first, super::proto::MAX_MESSAGE + 4096);
    assert!(bytes.len() > super::proto::MAX_MESSAGE);
    // Valid to the spec, so framed, and every byte of it discarded.
    bytes.extend(small_reply(101, second));
    let sender = send_later(daemon, bytes);
    let (events, peak) = pump_events(&mut conn, 2);
    sender.join().unwrap();
    assert!(
        matches!(events[0], Event::Dropped { token: 7 }),
        "{:?}",
        events[0]
    );
    assert!(
        matches!(&events[1], Event::Reply { token: 8, body, .. } if body.len() == 7),
        "{:?}",
        events[1]
    );
    // It never sat in staging whole.
    assert!(peak <= conn::Conn::WATERMARK + 8192, "{peak}");
    assert!(!conn.dead());
}

/// An over-cap reply whose header fields are past what is read is
/// skipped unread: no call can be named, so the event says the answer
/// was lost without saying whose, and the consumer re-reads or gives up.
#[test]
fn an_over_cap_reply_past_the_fields_read_is_an_unknown_drop() {
    let (mut conn, daemon) = connected();
    let first = conn.call("a.b", "/", "a.b", "Big", "", &[], 0, 7).unwrap();
    let _ = conn.pump();
    // A reply header by hand: fields past MAX_OVERSIZE_FIELDS, a body to
    // the spec's scale. Only the prefix and a piece of the body are sent:
    // enough to skip by, never the whole message.
    let (body_len, fields_len) = (2 * 1024 * 1024u32, 100_000u32);
    let mut m = vec![b'l', 2, 0, 1];
    m.extend_from_slice(&body_len.to_le_bytes());
    m.extend_from_slice(&5u32.to_le_bytes());
    m.extend_from_slice(&fields_len.to_le_bytes());
    let prefix = 16 + fields_len as usize + (8 - (16 + fields_len as usize) % 8) % 8;
    m.resize(prefix + 64 * 1024, 0);
    // The whole piece fits the socket buffers: joined first, the pumps
    // below are deterministic, not a race with the writer. A clone stays
    // open meanwhile, so the writer finishing is not an EOF (which would
    // kill the connection before it is worked).
    let _held = daemon.try_clone().unwrap();
    let sender = send_later(daemon, m);
    sender.join().unwrap();
    let mut events = Vec::new();
    for _ in 0..5 {
        events.extend(conn.pump().0);
    }
    assert!(
        matches!(events.as_slice(), [Event::Dropped { token }] if *token == conn::DROPPED_UNKNOWN),
        "{events:?}"
    );
    let _ = first;
    assert!(!conn.dead());
}

/// A big-endian over-cap header is read big-endian: the fields length is
/// not garbage and the skip is by the real prefix. The reply the fields
/// would name (which the little-endian-only parser cannot match to a
/// call) is skipped silently — like a normal-size big-endian message,
/// which is framed but never parsed — and the flight stays for the real
/// answer: not a stuck flight, and not a mass release either.
#[test]
fn a_big_endian_over_cap_reply_is_skipped_silently_and_the_flight_stays() {
    let (mut conn, daemon) = connected();
    let first = conn.call("a.b", "/", "a.b", "Big", "", &[], 0, 7).unwrap();
    let second = conn
        .call("a.b", "/", "a.b", "Small", "", &[], 0, 8)
        .unwrap();
    let _ = conn.pump();
    // A valid over-cap reply, then byte-swapped to big-endian: the flag
    // and every header word.
    let mut bytes = oversized_reply(100, first, super::proto::MAX_MESSAGE + 4096);
    assert!(bytes.len() > super::proto::MAX_MESSAGE);
    bytes[0] = b'B';
    for word in [[4, 8], [8, 12], [12, 16]] {
        bytes[word[0]..word[1]].reverse();
    }
    let fields = u32::from_be_bytes(bytes[12..16].try_into().unwrap()) as usize;
    // Small fields, read right: the skip is knowable, the sender is not.
    assert!(fields <= 64 * 1024, "{fields}");
    bytes.extend(small_reply(101, second));
    let sender = send_later(daemon, bytes);
    // Silent, and the flight stayed: the only event is the real answer.
    let (events, _) = pump_events(&mut conn, 1);
    // The sender blocks until everything is read: drain past what the
    // assertion needs before joining it (on old code the wait above ends
    // at the drop, with most of the flood still unread).
    let start = Instant::now();
    while conn.discard_pending() > 0 {
        assert!(start.elapsed() < Duration::from_secs(20), "never drained");
        let _ = conn.pump();
    }
    sender.join().unwrap();
    assert!(
        matches!(&events[0], Event::Reply { token: 8, body, .. } if body.len() == 7),
        "{:?}",
        events[0]
    );
    assert!(!conn.dead());
}

#[test]
fn an_over_cap_call_or_signal_is_skipped_with_no_event() {
    let (mut conn, daemon) = connected();
    let big = |kind: u8| {
        // A valid header by hand: a call or a signal with a 2 MiB body,
        // fields larger than the part read of an over-cap message.
        let (body_len, fields_len) = (2 * 1024 * 1024u32, 100_000u32);
        let mut m = vec![b'l', kind, 0, 1];
        m.extend_from_slice(&body_len.to_le_bytes());
        m.extend_from_slice(&5u32.to_le_bytes());
        m.extend_from_slice(&fields_len.to_le_bytes());
        let total = 16 + fields_len as usize + (8 - (16 + fields_len as usize) % 8) % 8;
        m.resize(total + body_len as usize, 0);
        m
    };
    let mut bytes = big(1);
    bytes.extend(big(4));
    let call = conn.call("a.b", "/", "a.b", "M", "", &[], 0, 3).unwrap();
    bytes.extend(small_reply(9, call));
    let sender = send_later(daemon, bytes);
    let (events, _) = pump_events(&mut conn, 1);
    sender.join().unwrap();
    assert!(
        matches!(events[0], Event::Reply { token: 3, .. }),
        "{:?}",
        events[0]
    );
    assert!(!conn.dead());
}

/// A sender that outruns the reader holds one turn, not the bar: while
/// a 60 MiB message is discarded, one pump reads a bounded amount (the
/// poll is woken for the rest), the connection lives, and the idle state
/// asks for nothing after.
#[test]
fn discarding_a_flood_is_bounded_a_pump_at_a_time() {
    use super::conn::Conn;
    let (client, mut daemon_end) = UnixStream::pair().unwrap();
    // Buffers big enough to hold megabytes before the first pump, so one
    // unbounded pump would eat past the budget deterministically.
    rustix::net::sockopt::set_socket_send_buffer_size(&daemon_end, 4 * 1024 * 1024).unwrap();
    rustix::net::sockopt::set_socket_recv_buffer_size(&client, 4 * 1024 * 1024).unwrap();
    let server = std::thread::spawn(move || {
        super::testdaemon::serve_setup(&mut daemon_end);
        daemon_end
    });
    let mut conn = conn::setup(client).unwrap();
    let mut daemon = server.join().unwrap();
    // A valid reply to no pending call (serial 1 is the set-up's): valid
    // to the spec, so framed, and every byte of it discarded.
    let total = 60 * 1024 * 1024;
    let mut writer = Writer::with_cap(total + 4096);
    writer.begin_return_to(99, ":1.7", 1, "ay");
    let cookie = writer.open_array(1).unwrap();
    writer.raw(&vec![7u8; total]);
    writer.close_array(cookie);
    let bytes = writer.finish().unwrap();
    // Fill the buffers deterministically (no race with the first pump):
    // the skip is already mid-flight when it is measured.
    daemon.set_nonblocking(true).unwrap();
    let mut at = 0;
    while at < bytes.len() {
        match daemon.write(&bytes[at..]) {
            Ok(n) => at += n,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(error) => panic!("{error}"),
        }
    }
    assert!(
        at > Conn::DISCARD_BUDGET + 8192,
        "the buffers hold {at} bytes: too little to tell a bounded pump from an unbounded one"
    );
    let sender = std::thread::spawn(move || {
        daemon.set_nonblocking(false).unwrap();
        daemon.write_all(&bytes[at..]).unwrap();
        daemon
    });
    // The priming pump reads the header and starts the skip; the measured
    // pump is mid-skip, so what it consumes is exactly what it read. The
    // measured pump must see data: a sender stalled by a loaded box (or
    // small kernel buffers) leaves the socket dry, and a dry pump
    // honestly reports uncapped — wait those out instead of measuring
    // them (seen 2026-10-03: CI failed the capped assert with the sender
    // starved, product paths untouched).
    let (events, _) = conn.pump();
    assert!(events.is_empty());
    assert!(conn.discard_pending() > 0, "the skip never started");
    let (capped, read) = {
        let start = Instant::now();
        loop {
            assert!(
                start.elapsed() < Duration::from_secs(20),
                "the sender never caught up"
            );
            let before = conn.discard_pending();
            let (events, capped) = conn.pump();
            let read = before.saturating_sub(conn.discard_pending());
            assert!(events.is_empty());
            if read > 0 && (capped || conn.discard_pending() == 0) {
                break (capped, read);
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    };
    // Bounded work, and the poll woken for the rest (unless the flood
    // drained whole in the measured pump: then nothing is left to wake
    // for, and the drain below is already done).
    assert!(
        read <= Conn::DISCARD_BUDGET + 8192,
        "{read} bytes in one pump"
    );
    if conn.discard_pending() > 0 {
        assert!(capped, "the poll is woken for the rest");
        assert!(conn.has_staged_work());
    }
    // It drains, bounded a pump, and the connection lives.
    let mut pumps = 1;
    let mut longest = Duration::ZERO;
    let start = Instant::now();
    while conn.discard_pending() > 0 {
        assert!(start.elapsed() < Duration::from_secs(60), "never drained");
        let turn = Instant::now();
        let _ = conn.pump();
        longest = longest.max(turn.elapsed());
        pumps += 1;
    }
    // One turn never holds the bar for the stream: milliseconds, not the
    // seconds an unbounded drain takes. The bound is generous (a loaded
    // box is slow, not wrong); the real number is in the report.
    assert!(
        longest < Duration::from_secs(1),
        "one pump held {longest:?}"
    );
    sender.join().unwrap();
    assert!(!conn.dead());
    // Idle again: no OUT, no staged work — wakeups must not change.
    assert!(!conn.want_write());
    assert!(!conn.has_staged_work());
    // Sanity: it took many bounded pumps, not one long one.
    assert!(pumps > 10, "{pumps}");
}

#[test]
fn a_flood_is_worked_a_turn_at_a_time_and_never_kills_the_connection() {
    let (mut conn, daemon) = connected();
    // 40,000 NewIcon signals: some 5 MiB, well past what staging holds.
    let mut bytes = Vec::new();
    for n in 0..40_000u32 {
        let mut writer = Writer::new();
        writer.begin_signal(
            n + 1,
            "/StatusNotifierItem",
            "org.kde.StatusNotifierItem",
            "NewIcon",
            "",
        );
        bytes.extend(writer.finish().unwrap());
    }
    assert!(bytes.len() > 3 * super::proto::MAX_MESSAGE);
    let sender = send_later(daemon, bytes);
    let (events, peak) = pump_events(&mut conn, 40_000);
    sender.join().unwrap();
    assert_eq!(events.len(), 40_000);
    assert!(peak <= conn::Conn::WATERMARK + 8192, "{peak}");
    assert!(!conn.dead());
}

#[test]
fn a_header_that_is_not_a_message_still_ends_the_connection() {
    let (mut conn, daemon) = connected();
    let sender = send_later(
        daemon,
        b"GET / HTTP/1.1\r\n\r\n garbage that frames as nothing".to_vec(),
    );
    let start = Instant::now();
    while !conn.dead() {
        assert!(start.elapsed() < Duration::from_secs(10));
        let _ = conn.pump();
    }
    sender.join().unwrap();
    // A length past the spec's own 128 MiB is not a message either.
    let (mut conn, daemon) = connected();
    let mut header = vec![b'l', 4, 0, 1];
    header.extend_from_slice(&(200u32 * 1024 * 1024).to_le_bytes());
    header.extend_from_slice(&1u32.to_le_bytes());
    header.extend_from_slice(&0u32.to_le_bytes());
    let sender = send_later(daemon, header);
    let start = Instant::now();
    while !conn.dead() {
        assert!(start.elapsed() < Duration::from_secs(10));
        let _ = conn.pump();
    }
    sender.join().unwrap();
}

#[test]
fn one_name_this_client_does_not_take_does_not_blind_the_list() {
    use super::proto::read_names;
    // A name list as a daemon sends it, with names a peer may own: one
    // past the spec's length, one that is no name. The rest is read.
    let mut body = Writer::new();
    let cookie = body.open_array(4).unwrap();
    for name in [
        "org.freedesktop.DBus".to_owned(),
        format!("org.example.{}", "x".repeat(300)),
        "not a name".to_owned(),
        "org.example.-lead".to_owned(),
        "org.kde.StatusNotifierItem-1-1".to_owned(),
    ] {
        body.str(&name);
    }
    body.close_array(cookie);
    let names = read_names("as", &body.take_body().unwrap()).unwrap();
    assert_eq!(
        names,
        [
            "org.freedesktop.DBus",
            "org.example.-lead",
            "org.kde.StatusNotifierItem-1-1"
        ]
    );
}

/// A header field code the spec does not give this client is walked past
/// by its signature, not a refusal of the message.
#[test]
fn an_unknown_header_field_is_ignored() {
    let mut writer = Writer::new();
    writer.begin_return(2, 1, "s");
    writer.str("hi");
    let message = writer.finish().unwrap();
    let fields_len = u32::from_le_bytes(message[12..16].try_into().unwrap()) as usize;
    let old_end = 16 + fields_len;
    let body_at = old_end + (8 - old_end % 8) % 8;
    let mut fields = message[16..old_end].to_vec();
    while !(16 + fields.len()).is_multiple_of(8) {
        fields.push(0);
    }
    // `{y, v}` with code 21 and a `u` of 7.
    fields.extend_from_slice(&[21, 1, b'u', 0, 7, 0, 0, 0]);
    let mut changed = message[..12].to_vec();
    changed.extend_from_slice(&(fields.len() as u32).to_le_bytes());
    changed.extend_from_slice(&fields);
    while !changed.len().is_multiple_of(8) {
        changed.push(0);
    }
    changed.extend_from_slice(&message[body_at..]);
    assert_eq!(frame_at(&changed), Ok(Some(changed.len())));
    let parsed = Message::parse(&changed).expect("an unknown field is not a refusal");
    assert_eq!(parsed.reply_serial, Some(1));
    assert_eq!(parsed.signature, "s");
    assert_eq!(
        super::proto::read_string(parsed.signature, parsed.body.rest()).as_deref(),
        Ok("hi")
    );
}

/// The header alone says how long a message is, however long: only a
/// header that is no message, or past the spec's 128 MiB, is refused.
#[test]
fn the_header_frames_a_long_message_and_refuses_a_false_one() {
    use super::proto::{MAX_MESSAGE, MAX_WIRE, frame_header};
    let header = |body: u32, fields: u32| {
        let mut h = vec![b'l', 2, 0, 1];
        h.extend_from_slice(&body.to_le_bytes());
        h.extend_from_slice(&1u32.to_le_bytes());
        h.extend_from_slice(&fields.to_le_bytes());
        h
    };
    assert_eq!(frame_header(&header(100, 0)[..10]), Ok(None));
    assert_eq!(frame_header(&header(100, 0)), Ok(Some(116)));
    let long = frame_header(&header(MAX_MESSAGE as u32 * 3, 0))
        .unwrap()
        .unwrap();
    assert!(long > MAX_MESSAGE);
    // The whole-message framing still refuses it: the set-up cannot skip.
    assert_eq!(frame_at(&header(MAX_MESSAGE as u32 * 3, 0)), Err(()));
    assert_eq!(frame_header(&header(MAX_WIRE as u32, 0)), Err(()));
    assert_eq!(frame_header(&header(5, u32::MAX)), Err(()));
    let mut bad = header(1, 0);
    bad[3] = 2;
    assert_eq!(frame_header(&bad), Err(()));
}
