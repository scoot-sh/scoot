//! The client against a real `dbus-daemon` ([`super::testdaemon`]): the
//! set-up, calls and replies, signals through match rules, errors, and a
//! peer going away.

use super::conn::{self, Conn, Event};
use super::proto::{self, Reader, Writer, request};
use super::testdaemon::{Daemon, until};

fn connect(daemon: &Daemon) -> Conn {
    conn::connect(&daemon.path()).expect("the daemon accepts EXTERNAL and says Hello")
}

/// A call to the bus itself with a string argument.
fn bus_call(conn: &mut Conn, member: &str, sig: &str, body: &[u8], token: u64) {
    conn.call(
        conn::BUS_NAME,
        conn::BUS_PATH,
        conn::BUS_INTERFACE,
        member,
        sig,
        body,
        0,
        token,
    )
    .unwrap();
}

fn string_body(text: &str) -> Vec<u8> {
    let mut body = Writer::new();
    body.str(text);
    body.take_body().unwrap()
}

#[test]
fn a_real_daemon_names_us_and_answers_the_set_up_calls() {
    let Some(daemon) = Daemon::spawn() else {
        return;
    };
    let mut conn = connect(&daemon);
    assert!(conn.unique().starts_with(":1."), "{}", conn.unique());

    // RequestName: primary owner.
    let mut body = Writer::new();
    body.str("sh.scoot.Test");
    body.u32(request::ALLOW_REPLACEMENT | request::DO_NOT_QUEUE);
    bus_call(
        &mut conn,
        "RequestName",
        "su",
        &body.take_body().unwrap(),
        1,
    );
    let Event::Reply {
        signature, body, ..
    } = until(&mut conn, |event| {
        matches!(event, Event::Reply { token: 1, .. })
    })
    else {
        unreachable!()
    };
    assert_eq!(
        proto::read_request_reply(&signature, &body),
        Ok(proto::request_reply::PRIMARY_OWNER)
    );

    // ListNames holds the name and ours; GetNameOwner maps one to the other.
    bus_call(&mut conn, "ListNames", "", &[], 2);
    let Event::Reply {
        signature, body, ..
    } = until(&mut conn, |event| {
        matches!(event, Event::Reply { token: 2, .. })
    })
    else {
        unreachable!()
    };
    let names = proto::read_names(&signature, &body).unwrap();
    assert!(names.iter().any(|name| name == "sh.scoot.Test"));
    assert!(names.iter().any(|name| name == conn.unique()));
    bus_call(
        &mut conn,
        "GetNameOwner",
        "s",
        &string_body("sh.scoot.Test"),
        3,
    );
    let Event::Reply {
        signature, body, ..
    } = until(&mut conn, |event| {
        matches!(event, Event::Reply { token: 3, .. })
    })
    else {
        unreachable!()
    };
    assert_eq!(proto::read_owner(&signature, &body).unwrap(), conn.unique());

    // A name nobody owns is an error reply, correlated, not a hang.
    bus_call(
        &mut conn,
        "GetNameOwner",
        "s",
        &string_body("sh.scoot.Nobody"),
        4,
    );
    let Event::CallError { name, .. } = until(&mut conn, |event| {
        matches!(event, Event::CallError { token: 4, .. })
    }) else {
        unreachable!()
    };
    assert!(name.ends_with("NameHasNoOwner"), "{name}");
    assert!(!conn.dead());
}

#[test]
fn calls_replies_signals_and_a_vanishing_peer_between_two_connections() {
    let Some(daemon) = Daemon::spawn() else {
        return;
    };
    let mut service = connect(&daemon);
    let mut client = connect(&daemon);
    let mut body = Writer::new();
    body.str("sh.scoot.Peer");
    body.u32(0);
    bus_call(
        &mut service,
        "RequestName",
        "su",
        &body.take_body().unwrap(),
        1,
    );
    until(&mut service, |event| {
        matches!(event, Event::Reply { token: 1, .. })
    });

    // The client listens for the service's signals and for owners
    // leaving, through match rules (no reply wanted).
    for rule in [
        "type='signal',interface='sh.scoot.Peer'",
        "type='signal',sender='org.freedesktop.DBus',interface='org.freedesktop.DBus',\
         member='NameOwnerChanged'",
    ] {
        client
            .call(
                conn::BUS_NAME,
                conn::BUS_PATH,
                conn::BUS_INTERFACE,
                "AddMatch",
                "s",
                &string_body(rule),
                proto::flag::NO_REPLY_EXPECTED,
                0,
            )
            .unwrap();
    }
    // A method call at the service, which answers it addressed to the
    // sender (a reply with no destination is dropped by the daemon).
    client
        .call(
            "sh.scoot.Peer",
            "/Peer",
            "sh.scoot.Peer",
            "Echo",
            "s",
            &string_body("hi"),
            0,
            7,
        )
        .unwrap();
    // Queued calls leave on a pump: flush before waiting on the peer.
    let _ = client.pump();
    let Event::MethodCall {
        sender,
        member,
        serial,
        body,
        ..
    } = until(&mut service, |event| {
        matches!(event, Event::MethodCall { .. })
    })
    else {
        unreachable!()
    };
    assert_eq!(member, "Echo");
    assert_eq!(sender, client.unique());
    assert_eq!(Reader::le(&body).str(), Ok("hi"));
    service.reply_return(&sender, serial, "s", &body);
    // Queued output leaves on a pump (the bar's loop polls for `OUT`).
    let _ = service.pump();
    let Event::Reply { body, .. } = until(&mut client, |event| {
        matches!(event, Event::Reply { token: 7, .. })
    }) else {
        unreachable!()
    };
    assert_eq!(Reader::le(&body).str(), Ok("hi"));

    // A broadcast signal reaches the matched client.
    service.signal("/Peer", "sh.scoot.Peer", "Ping", "s", &string_body("sig"));
    // Flush it: the service's own pump writes the outbox.
    let _ = service.pump();
    let Event::Signal { member, body, .. } = until(
        &mut client,
        |event| matches!(event, Event::Signal { member, .. } if member == "Ping"),
    ) else {
        unreachable!()
    };
    assert_eq!(member, "Ping");
    assert_eq!(Reader::le(&body).str(), Ok("sig"));

    // The service going away is a NameOwnerChanged naming it, with no
    // new owner: what drops a crashed item.
    drop(service);
    let Event::Signal { body, .. } = until(&mut client, |event| {
        matches!(event, Event::Signal { member, body, .. }
            if member == "NameOwnerChanged"
                && proto::read_name_owner_changed(body)
                    .is_ok_and(|(name, _, new)| name == "sh.scoot.Peer" && new.is_none()))
    }) else {
        unreachable!()
    };
    let (name, old, new) = proto::read_name_owner_changed(&body).unwrap();
    assert_eq!(name, "sh.scoot.Peer");
    assert!(old.is_some() && new.is_none());
}
