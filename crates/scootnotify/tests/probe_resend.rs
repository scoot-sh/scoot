//! Pin for the serving probe's silence resend: a scripted bus drops
//! the first `GetServerInformation` probe and answers the second.
//! `support`'s `wait_serving` must still succeed — and must have sent
//! at least two probes to do it. Without the silence resend (resend
//! only on the name-unowned error, the old shape) this test times out
//! at its budget instead: the pin fails before the fix and passes
//! after. The pin also asserts no `Notify` was ever sent: the probe
//! makes no card by design.
//!
//! The scripted bus speaks just enough D-Bus: the SASL handshake, a
//! `Hello` assigning `:9.9`, then `GetServerInformation` handling. It
//! needs no `dbus-daemon` and no server binary.

#[path = "support/mod.rs"]
mod support;

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use scoot_dbus::conn::{self, Conn};
use scoot_dbus::proto::{self, Kind, Writer};

/// Runs the scripted bus on `stream`: handshake, `Hello`, then drops
/// the first `GetServerInformation` and answers the rest with the
/// daemon's shape. Counts every `GetServerInformation` in `infos` and
/// every `Notify` in `notifies` (which must stay zero: the probe never
/// notifies).
fn serve_fake_bus(mut stream: UnixStream, infos: Arc<AtomicU64>, notifies: Arc<AtomicU64>) {
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("a read timeout");
    stream
        .set_write_timeout(Some(Duration::from_secs(10)))
        .expect("a write timeout");
    read_until(&mut stream, b"AUTH EXTERNAL\r\n");
    stream.write_all(b"DATA\r\n").expect("the challenge");
    read_until(&mut stream, b"DATA\r\n");
    stream
        .write_all(b"OK 0123456789abcdef\r\n")
        .expect("the ok");
    read_until(&mut stream, b"BEGIN\r\n");
    let hello = read_frame(&mut stream);
    let call = proto::Message::parse(&hello).expect("a Hello frame");
    assert_eq!(call.member, Some("Hello"));
    let mut body = Writer::new();
    body.str(":9.9");
    let body = body.take_body().expect("a unique name");
    let mut reply = Writer::new();
    reply.begin_return(1, call.serial, "s");
    reply.raw(&body);
    stream
        .write_all(&reply.finish().expect("a Hello reply"))
        .expect("the Hello answer");
    let mut serial = 1u32;
    loop {
        let Some(frame) = read_frame_opt(&mut stream) else {
            // The client went away: the pin is over.
            return;
        };
        let message = proto::Message::parse(&frame).expect("a call frame");
        if message.kind != Kind::MethodCall {
            continue;
        }
        match message.member {
            Some("GetServerInformation") => {
                let seen = infos.fetch_add(1, Ordering::SeqCst) + 1;
                if seen == 1 {
                    // The dropped probe: no answer at all, as if the
                    // bus consumed it without forwarding it.
                    continue;
                }
                let mut info = Writer::new();
                info.str("scootnotify");
                info.str("scoot");
                info.str(env!("CARGO_PKG_VERSION"));
                info.str("1.3");
                let info = info.take_body().expect("server information");
                serial += 1;
                let mut answer = Writer::new();
                answer.begin_return_to(serial, ":9.9", message.serial, "ssss");
                answer.raw(&info);
                if stream
                    .write_all(&answer.finish().expect("an info reply"))
                    .is_err()
                {
                    return;
                }
            }
            Some("Notify") => {
                notifies.fetch_add(1, Ordering::SeqCst);
            }
            _ => {}
        }
    }
}

/// Reads from `stream` until `want` has been read, byte by byte.
fn read_until(stream: &mut UnixStream, want: &[u8]) {
    let mut got = Vec::new();
    let mut byte = [0u8; 1];
    while !got.ends_with(want) {
        let n = stream.read(&mut byte).expect("the client talks");
        assert_eq!(n, 1, "the client talks before closing");
        got.push(byte[0]);
    }
}

/// Reads one whole frame, or `None` when the client went away.
fn read_frame_opt(stream: &mut UnixStream) -> Option<Vec<u8>> {
    let mut staged = Vec::new();
    let mut chunk = [0u8; 65536];
    loop {
        if let Ok(Some(len)) = proto::frame_at(&staged) {
            return Some(staged.drain(..len).collect());
        }
        match stream.read(&mut chunk) {
            Ok(0) => return None,
            Ok(n) => staged.extend_from_slice(&chunk[..n]),
            Err(_) => return None,
        }
    }
}

/// Reads one whole frame: the handshake's answer fails loud instead of
/// hanging the pin.
fn read_frame(stream: &mut UnixStream) -> Vec<u8> {
    read_frame_opt(stream).expect("the client talks before closing")
}

/// A live-albeit-sleeping stand-in for the daemon `Child`: `support`
/// only reaps it (never alive here) and reads its `/proc` state on
/// timeout (never reached here).
fn sleeping_server() -> Child {
    Command::new("sleep")
        .arg("30")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("sleep starts")
}

#[test]
fn an_unanswered_probe_is_resent_until_answered() {
    let (client_end, server_end) = UnixStream::pair().expect("a socket pair");
    let infos = Arc::new(AtomicU64::new(0));
    let notifies = Arc::new(AtomicU64::new(0));
    let info_counter = infos.clone();
    let notify_counter = notifies.clone();
    std::thread::spawn(move || serve_fake_bus(server_end, info_counter, notify_counter));
    let mut client: Conn = conn::setup(client_end).expect("the handshake");
    let mut server = sleeping_server();
    support::wait_serving_budget(
        &mut client,
        &mut server,
        support::LogSource::Pipe,
        Duration::from_secs(30),
        Duration::from_millis(200),
    );
    let _ = server.kill();
    let _ = server.wait();
    assert!(
        infos.load(Ordering::SeqCst) >= 2,
        "the first probe was dropped, so success proves a resend"
    );
    assert_eq!(
        notifies.load(Ordering::SeqCst),
        0,
        "the probe never notifies"
    );
}
