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

// ESTABLISHED 2026-10-03 (this test's first run as a probe): dbus-daemon
// 1.16.2 delivers an unsolicited METHOD_RETURN (and ERROR) from a peer
// that was not the callee to the named destination, and dbus-broker 37
// does not (the forgery never arrived; the real answer did). So the fix
// below is real, not defence in depth: a reply is refused when its sender
// is not the callee, whenever the callee is known.
fn read_string(body: &[u8]) -> String {
    Reader::le(body).str().unwrap().to_owned()
}

/// A peer that guesses a pending serial cannot forge its answer: the
/// forged reply is refused, the flight stays, and the real answer still
/// lands.
#[test]
fn a_forged_peer_reply_is_refused_and_the_real_answer_lands() {
    let Some(daemon) = Daemon::spawn() else {
        return;
    };
    let mut victim = connect(&daemon);
    let mut callee = connect(&daemon);
    let mut forger = connect(&daemon);
    // The victim calls the callee; the callee holds the call unanswered.
    let serial = victim
        .call(callee.unique(), "/", "a.b", "Slow", "", &[], 0, 42)
        .unwrap();
    let _ = victim.pump();
    let Event::MethodCall { sender, .. } = until(&mut callee, |event| {
        matches!(event, Event::MethodCall { .. })
    }) else {
        unreachable!()
    };
    assert_eq!(sender, victim.unique());
    // The forger answers it first, return and error alike, quoting the
    // victim's serial.
    forger.reply_return(victim.unique(), serial, "s", &string_body("forged"));
    forger.reply_error(victim.unique(), serial, "org.example.Forged");
    // Flushes the forgeries out; what this pump reads is discarded, which
    // is safe only because nothing of the forger's is waited for yet (at
    // most its own `NameAcquired`, never a reply: both forgeries want
    // none, and the barrier call below is queued after this pump).
    let _ = forger.pump();
    // The forger's ListNames round trip proves the daemon read its
    // messages in order. That is not delivery: across connections it says
    // nothing about the victim's socket, so on its own the "nothing
    // arrives" below can pass vacuously. A ping the forger sends the
    // victim afterwards closes it: the daemon dispatches one connection's
    // messages in order, so the ping lands after both forgeries, and the
    // victim seeing the ping proves both forgeries are already in its
    // socket. (No pre-pump on the forger here: `until` flushes on its own
    // first pump, and discarding a pump on a connection then waited on
    // loses a reply that arrived between the flush and the read.)
    bus_call(&mut forger, "ListNames", "", &[], 99);
    until(&mut forger, |event| {
        matches!(event, Event::Reply { token: 99, .. })
    });
    forger
        .call(
            victim.unique(),
            "/p",
            "a.b",
            "Ping",
            "",
            &[],
            proto::flag::NO_REPLY_EXPECTED,
            0,
        )
        .unwrap();
    // A flush of another connection than the one waited on: the ping has
    // to leave before the victim can see it, and nothing of the forger's
    // is waited for.
    let _ = forger.pump();
    let Event::MethodCall { member, .. } = until(&mut victim, |event| {
        matches!(event, Event::MethodCall { .. })
    }) else {
        unreachable!()
    };
    assert_eq!(member, "Ping");
    // While only the forgeries are in flight, nothing arrives: both are
    // refused, and the flight stays for the real answer.
    for _ in 0..5 {
        let (events, _) = victim.pump();
        assert!(events.is_empty(), "{events:?}");
    }
    // The callee answers for real: the real answer lands, whole.
    callee.reply_return(&sender, serial, "s", &string_body("real"));
    let _ = callee.pump();
    let Event::Reply {
        signature, body, ..
    } = until(&mut victim, |event| {
        matches!(event, Event::Reply { token: 42, .. })
    })
    else {
        unreachable!()
    };
    assert_eq!(signature, "s");
    assert_eq!(read_string(&body), "real");
}

/// A unique-name callee that disconnects mid-call is answered by the bus
/// itself: the daemon synthesizes the error with its own name as the
/// sender (measured 2026-10-03 on dbus-daemon 1.16.2:
/// `org.freedesktop.DBus.Error.NoReply` from `org.freedesktop.DBus`).
/// That is the reply, not a forgery, so the flight resolves through the
/// error path instead of leaking to the reap.
#[test]
fn a_callee_that_disconnects_mid_call_is_answered_by_the_bus() {
    let Some(daemon) = Daemon::spawn() else {
        return;
    };
    let mut victim = connect(&daemon);
    let callee = connect(&daemon);
    victim
        .call(callee.unique(), "/", "a.b", "Slow", "", &[], 0, 57)
        .unwrap();
    // Flushes the call out; what this pump reads is discarded, which is
    // safe only because nothing of the victim's is waited for yet (at
    // most its own `NameAcquired`: the callee has not answered, and the
    // daemon's error comes only after the drop below).
    let _ = victim.pump();
    drop(callee);
    let Event::CallError { name, .. } = until(&mut victim, |event| {
        matches!(event, Event::CallError { token: 57, .. })
    }) else {
        unreachable!()
    };
    assert!(!name.is_empty(), "the bus named its error");
}

/// The bus's own answer cannot be forged either: a peer's reply to a bus
/// call is refused, and the bus's real answer still lands. Deterministic
/// without waiting on the bus: the forgery is sent before the call, so it
/// is first in the victim's socket, and the bus's answer follows it.
#[test]
fn a_forged_bus_reply_is_refused_and_the_real_answer_lands() {
    let Some(daemon) = Daemon::spawn() else {
        return;
    };
    let mut victim = connect(&daemon);
    let mut forger = connect(&daemon);
    // `Hello` is serial 1, so the next call is serial 2: forged first.
    forger.reply_return(victim.unique(), 2, "s", &string_body("forged"));
    let _ = forger.pump();
    let serial = victim
        .call(
            conn::BUS_NAME,
            conn::BUS_PATH,
            conn::BUS_INTERFACE,
            "ListNames",
            "",
            &[],
            0,
            43,
        )
        .unwrap();
    assert_eq!(serial, 2);
    // The only answer is the bus's own: a list of names, never the
    // forgery (which is refused, sender against callee).
    let Event::Reply {
        signature, body, ..
    } = until(&mut victim, |event| {
        matches!(event, Event::Reply { token: 43, .. })
    })
    else {
        unreachable!()
    };
    let names = proto::read_names(&signature, &body).unwrap();
    assert!(names.iter().any(|name| name == victim.unique()));
    let (events, _) = victim.pump();
    assert!(events.is_empty(), "{events:?}");
}

/// A valid method return of `len` array bytes to `dest`, which `Writer`'s
/// own cap would refuse: the tests' way to have a peer send what the
/// over-cap skip paths must sort.
fn over_cap_reply(dest: &str, reply_to: u32, len: usize) -> Vec<u8> {
    let mut writer = Writer::with_cap(len + 4096);
    writer.begin_return_to(400, dest, reply_to, "ay");
    let cookie = writer.open_array(1).unwrap();
    writer.raw(&vec![7u8; len]);
    writer.close_array(cookie);
    writer.finish().unwrap()
}

/// Flushes everything `forger` queued (a megabyte takes several turns),
/// then a ping proving its delivery: the daemon dispatches one
/// connection's messages in order, so the victim seeing the ping proves
/// everything before it reached the victim's socket. Discarding the
/// forger's own pumps is safe: nothing of the forger's is ever waited for.
fn flood_then_ping(forger: &mut Conn, victim_unique: &str, bytes: &[u8]) {
    forger.queue_raw(bytes);
    drain_with_deadline(forger);
    forger
        .call(
            victim_unique,
            "/p",
            "a.b",
            "Ping",
            "",
            &[],
            proto::flag::NO_REPLY_EXPECTED,
            0,
        )
        .unwrap();
    drain_with_deadline(forger);
}

/// Flushes everything `forger` queued (a megabyte takes several turns),
/// bounded: a dead daemon must fail the test, not spin the drain until
/// the CI job times out.
fn drain_with_deadline(forger: &mut Conn) {
    let start = std::time::Instant::now();
    while forger.want_write() {
        assert!(
            !forger.dead(),
            "the bus died while flushing what the forger queued"
        );
        assert!(
            start.elapsed() < std::time::Duration::from_secs(20),
            "the forger never flushed what it queued"
        );
        let _ = forger.pump();
    }
}

/// Pumps `victim` until the forger's ping lands — proving everything sent
/// before it arrived — and panics on any `Dropped` or forged `Reply`: an
/// over-cap stranger must be silent, never a release of everything
/// waiting.
fn until_ping_without_a_drop(victim: &mut Conn) {
    let start = std::time::Instant::now();
    loop {
        if victim.dead() {
            panic!("the bus died while waiting for what the test waits for");
        }
        assert!(
            start.elapsed() < std::time::Duration::from_secs(10),
            "the ping never landed"
        );
        let (events, _) = victim.pump();
        let mut saw_ping = false;
        for event in events {
            match event {
                Event::MethodCall { member, .. } if member == "Ping" => saw_ping = true,
                Event::Dropped { token } => panic!("an over-cap stranger released {token}"),
                Event::Reply { token, .. } => panic!("a forgery was accepted for {token}"),
                _ => {}
            }
        }
        if saw_ping {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

/// A forged reply past what is read is refused like a normal-size one:
/// silently (no mass release of everything waiting), the flight staying
/// for the real answer.
#[test]
fn an_over_cap_forged_reply_is_refused_silently_and_the_real_answer_lands() {
    let Some(daemon) = Daemon::spawn() else {
        return;
    };
    let mut victim = connect(&daemon);
    let mut callee = connect(&daemon);
    let mut forger = connect(&daemon);
    // The victim calls the callee; the callee holds the call unanswered.
    let serial = victim
        .call(callee.unique(), "/", "a.b", "Slow", "", &[], 0, 42)
        .unwrap();
    // Flushes the call out (the wait is on the callee, not this
    // connection); the callee holding it is the scene the forgery needs.
    let _ = victim.pump();
    until(&mut callee, |event| {
        matches!(event, Event::MethodCall { .. })
    });
    // The forger answers the victim's serial with a valid over-cap reply.
    // The daemon stamps it with the forger's name on delivery.
    let big = over_cap_reply(victim.unique(), serial, proto::MAX_MESSAGE + 4096);
    assert!(big.len() > proto::MAX_MESSAGE);
    flood_then_ping(&mut forger, victim.unique(), &big);
    until_ping_without_a_drop(&mut victim);
    // The flight stayed: the real answer still lands, whole.
    callee.reply_return(victim.unique(), serial, "s", &string_body("real"));
    let _ = callee.pump();
    let Event::Reply { body, .. } = until(&mut victim, |event| {
        matches!(event, Event::Reply { token: 42, .. })
    }) else {
        unreachable!()
    };
    assert_eq!(read_string(&body), "real");
}

/// An over-cap reply answering nothing waiting is silent too: with a call
/// still pending, no unknown drop releases everything.
#[test]
fn an_over_cap_reply_to_nothing_waiting_is_silent() {
    let Some(daemon) = Daemon::spawn() else {
        return;
    };
    let mut victim = connect(&daemon);
    let mut callee = connect(&daemon);
    let mut forger = connect(&daemon);
    let serial = victim
        .call(callee.unique(), "/", "a.b", "Slow", "", &[], 0, 42)
        .unwrap();
    let _ = victim.pump();
    until(&mut callee, |event| {
        matches!(event, Event::MethodCall { .. })
    });
    // A valid over-cap reply to a serial nothing waits for.
    let big = over_cap_reply(victim.unique(), 0x1234_5678, proto::MAX_MESSAGE + 4096);
    assert!(big.len() > proto::MAX_MESSAGE);
    flood_then_ping(&mut forger, victim.unique(), &big);
    until_ping_without_a_drop(&mut victim);
    // Nothing was released: the real answer still lands, whole.
    callee.reply_return(victim.unique(), serial, "s", &string_body("real"));
    let _ = callee.pump();
    let Event::Reply { body, .. } = until(&mut victim, |event| {
        matches!(event, Event::Reply { token: 42, .. })
    }) else {
        unreachable!()
    };
    assert_eq!(read_string(&body), "real");
}

/// What one signal's dispatch costs, measured so a regression shows:
/// the six owned pieces (sender, path, interface, member, signature,
/// body) plus the events vec's first growth. All six are inherent — the
/// events own their data over a reused buffer — so there is nothing
/// cheap to take; an idle pump is zero.
#[test]
fn a_signal_costs_six_small_allocations() {
    let Some(daemon) = Daemon::spawn() else {
        return;
    };
    let mut watcher = connect(&daemon);
    let mut peer = connect(&daemon);
    // The match, then a barrier proving it is installed (the bus works
    // one connection's calls in order).
    let mut rule = Writer::new();
    rule.str("type='signal',interface='sh.scoot.Alloc'");
    watcher
        .call(
            conn::BUS_NAME,
            conn::BUS_PATH,
            conn::BUS_INTERFACE,
            "AddMatch",
            "s",
            &rule.take_body().unwrap(),
            proto::flag::NO_REPLY_EXPECTED,
            0,
        )
        .unwrap();
    bus_call(
        &mut watcher,
        "GetNameOwner",
        "s",
        &string_body("org.freedesktop.DBus"),
        1,
    );
    // No pre-pump: `until` flushes the queued calls on its own first
    // pump, and a discarded pump on the waited-on connection loses a
    // reply that arrived between the flush and the read.
    until(&mut watcher, |event| {
        matches!(event, Event::Reply { token: 1, .. })
    });
    let mut ping = || {
        peer.signal("/p", "sh.scoot.Alloc", "Ping", "s", &string_body("x"));
        peer.pump();
    };
    // Warm: staging keeps its capacity, and one signal is worked.
    ping();
    until(&mut watcher, |event| matches!(event, Event::Signal { .. }));
    // Measured: one more identical signal, already waiting in the
    // socket before it is counted (polled for, never pumped for).
    ping();
    {
        use rustix::event::{PollFd, PollFlags, Timespec, poll};
        let fd = watcher.as_fd();
        let start = std::time::Instant::now();
        loop {
            let mut fds = [PollFd::new(&fd, PollFlags::IN)];
            let timeout = Timespec {
                tv_sec: 0,
                tv_nsec: 50_000_000,
            };
            let _ = poll(&mut fds, Some(&timeout));
            if !fds[0].revents().is_empty() {
                break;
            }
            assert!(
                start.elapsed() < std::time::Duration::from_secs(10),
                "no signal"
            );
        }
    }
    let ((events, _), allocations) = scootbg_mem::count_allocations(|| watcher.pump());
    assert_eq!(events.len(), 1);
    assert_eq!(allocations, 7, "six owned pieces and the events vec");
    // And an idle pump is zero.
    let (_, idle) = scootbg_mem::count_allocations(|| watcher.pump());
    assert_eq!(idle, 0, "an idle pump allocates nothing");
}
