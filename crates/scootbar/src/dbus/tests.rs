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
    assert_eq!(parsed.error, Some("org.freedesktop.DBus.Error.UnknownMethod"));
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
