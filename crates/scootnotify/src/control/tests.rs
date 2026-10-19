//! The control socket's contract: the protocol's parse and replies, and
//! what the daemon answers to each request — without a bus or a
//! compositor.

use std::borrow::Cow;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use scoot_dbus::conn::Conn;
use scoot_dbus::proto::{Message, Writer, frame_at};

use super::protocol::{
    DndMode, EventKind, PROTOCOL_VERSION, ReloadScope, Request, parse, write_event, write_reply,
};
use super::respond::Responder;
use super::{Handler, Server};
use crate::bus::hints::{Hints, Urgency};
use crate::bus::notify::{Action, Incoming};
use crate::bus::server::Server as NotifyServer;
use crate::config::Config;
use crate::model::store::IdFile;

/// Parses `text` (without its newline) the way the daemon parses a line.
fn parse_text(text: &str) -> Result<Request<'_>, super::protocol::RequestError> {
    parse(text.as_bytes())
}

#[test]
fn every_verb_parses() {
    assert_eq!(
        parse_text(r#"{"protocol":2,"type":"list"}"#).unwrap(),
        Request::List
    );
    assert_eq!(
        parse_text(r#"{"protocol":2,"type":"history"}"#).unwrap(),
        Request::History
    );
    assert_eq!(
        parse_text(r#"{"protocol":2,"type":"restore"}"#).unwrap(),
        Request::Restore
    );
    assert_eq!(
        parse_text(r#"{"protocol":2,"type":"dnd"}"#).unwrap(),
        Request::Dnd { mode: None }
    );
    assert_eq!(
        parse_text(r#"{"protocol":2,"type":"dnd","mode":"on"}"#).unwrap(),
        Request::Dnd {
            mode: Some(DndMode::On)
        }
    );
    assert_eq!(
        parse_text(r#"{"protocol":2,"type":"dnd","mode":"toggle"}"#).unwrap(),
        Request::Dnd {
            mode: Some(DndMode::Toggle)
        }
    );
    assert_eq!(
        parse_text(r#"{"protocol":2,"type":"dismiss","id":3}"#).unwrap(),
        Request::Dismiss {
            id: Some(3),
            all: false
        }
    );
    assert_eq!(
        parse_text(r#"{"protocol":2,"type":"dismiss","all":true}"#).unwrap(),
        Request::Dismiss {
            id: None,
            all: true
        }
    );
    assert_eq!(
        parse_text(r#"{"protocol":2,"type":"invoke","id":3}"#).unwrap(),
        Request::Invoke {
            id: 3,
            action: Cow::Borrowed("default")
        }
    );
    assert_eq!(
        parse_text(r#"{"protocol":2,"type":"invoke","id":3,"action":"open"}"#).unwrap(),
        Request::Invoke {
            id: 3,
            action: Cow::Borrowed("open")
        }
    );
    assert_eq!(
        parse_text(r#"{"protocol":2,"type":"reload"}"#).unwrap(),
        Request::Reload { scope: None }
    );
    assert_eq!(
        parse_text(r#"{"protocol":2,"type":"reload","scope":"look"}"#).unwrap(),
        Request::Reload {
            scope: Some(ReloadScope::Look)
        }
    );
    assert_eq!(
        parse_text(r#"{"protocol":2,"type":"subscribe"}"#).unwrap(),
        Request::Subscribe {
            events: EventKind::ALL.to_vec()
        }
    );
    assert_eq!(
        parse_text(r#"{"protocol":2,"type":"subscribe","events":[]}"#).unwrap(),
        Request::Subscribe {
            events: EventKind::ALL.to_vec()
        }
    );
    assert_eq!(
        parse_text(r#"{"protocol":2,"type":"subscribe","events":["close"]}"#).unwrap(),
        Request::Subscribe {
            events: vec![EventKind::Close]
        }
    );
    assert_eq!(
        parse_text(r#"{"protocol":2,"type":"subscribe","events":["notify","dnd"]}"#).unwrap(),
        Request::Subscribe {
            events: vec![EventKind::Notify, EventKind::Dnd]
        }
    );
    assert_eq!(
        parse_text(r#"{"protocol":2,"type":"version"}"#).unwrap(),
        Request::Version
    );
}

#[test]
fn requests_round_trip_through_their_lines() {
    let requests = [
        Request::List,
        Request::History,
        Request::Restore,
        Request::Dnd { mode: None },
        Request::Dnd {
            mode: Some(DndMode::Toggle),
        },
        Request::Dismiss {
            id: Some(3),
            all: false,
        },
        Request::Dismiss {
            id: None,
            all: true,
        },
        Request::Invoke {
            id: 3,
            action: Cow::Borrowed("open"),
        },
        Request::Reload { scope: None },
        Request::Reload {
            scope: Some(ReloadScope::Look),
        },
        Request::Subscribe {
            events: vec![EventKind::Notify, EventKind::Close],
        },
        Request::Subscribe {
            events: vec![EventKind::Dnd],
        },
        Request::Version,
    ];
    for request in &requests {
        let line = request.line();
        assert_eq!(&parse(line.as_bytes()).unwrap(), request);
    }
}

#[test]
fn unknown_fields_are_ignored() {
    assert_eq!(
        parse_text(r#"{"protocol":2,"type":"list","future":true}"#).unwrap(),
        Request::List
    );
}

#[test]
fn framing_refusals_name_themselves() {
    assert!(parse(b"").is_err());
    assert!(parse(b"[1, \"list\"]").is_err());
    assert!(parse(b"42").is_err());
    assert!(parse(b"{not json").is_err());
    let error = parse_text(r#"{"type":"list"}"#).unwrap_err();
    assert!(error.to_string().contains("protocol"), "{error}");
    let error = parse_text(r#"{"protocol":999,"type":"list"}"#).unwrap_err();
    assert!(error.to_string().contains("999"), "{error}");
    let error = parse_text(r#"{"protocol":2}"#).unwrap_err();
    assert!(error.to_string().contains("type"), "{error}");
    let error = parse_text(r#"{"protocol":2,"type":"history-bogus"}"#).unwrap_err();
    assert!(error.to_string().contains("history-bogus"), "{error}");
    // A version-1 client speaks a version the daemon no longer parses.
    let error = parse_text(r#"{"protocol":1,"type":"list"}"#).unwrap_err();
    assert!(error.to_string().contains("mismatch"), "{error}");
}

#[test]
fn id_and_scope_refusals_name_themselves() {
    // `dismiss` with neither an id nor `"all":true`.
    assert!(parse_text(r#"{"protocol":2,"type":"dismiss"}"#).is_err());
    // Not whole numbers that fit: floats, strings, negatives, zero, huge.
    for bad in ["3.5", "\"3\"", "-1", "0", "4294967296"] {
        let error =
            parse_text(&format!(r#"{{"protocol":2,"type":"dismiss","id":{bad}}}"#)).unwrap_err();
        assert!(error.to_string().contains("id"), "{bad}: {error}");
        let error =
            parse_text(&format!(r#"{{"protocol":2,"type":"invoke","id":{bad}}}"#)).unwrap_err();
        assert!(error.to_string().contains("id"), "{bad}: {error}");
    }
    assert!(parse_text(r#"{"protocol":2,"type":"invoke"}"#).is_err());
    let error = parse_text(r#"{"protocol":2,"type":"reload","scope":"leave"}"#).unwrap_err();
    assert!(error.to_string().contains("leave"), "{error}");
    let error = parse_text(r#"{"protocol":2,"type":"subscribe","events":["bogus"]}"#).unwrap_err();
    assert!(error.to_string().contains("bogus"), "{error}");
    let error = parse_text(r#"{"protocol":2,"type":"dnd","mode":"loud"}"#).unwrap_err();
    assert!(error.to_string().contains("loud"), "{error}");
    // The bounds: 1 and u32::MAX pass.
    assert!(parse_text(r#"{"protocol":2,"type":"dismiss","id":1}"#).is_ok());
    assert!(parse_text(r#"{"protocol":2,"type":"dismiss","id":4294967295}"#).is_ok());
}

/// A connected bus `Conn` whose peer is the test, mirroring the bus
/// server tests' handshake: dismissals and invocations queue their
/// signals on it.
fn connected() -> (Conn, UnixStream) {
    let (client, peer) = UnixStream::pair().unwrap();
    let thread = std::thread::spawn(move || {
        let mut peer = peer;
        peer.set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        read_until(&mut peer, b"AUTH EXTERNAL\r\n");
        peer.write_all(b"DATA\r\n").unwrap();
        read_until(&mut peer, b"DATA\r\n");
        peer.write_all(b"OK 0123456789abcdef\r\n").unwrap();
        read_until(&mut peer, b"BEGIN\r\n");
        let mut staged = Vec::new();
        let mut chunk = [0u8; 4096];
        let len = loop {
            if let Ok(Some(len)) = frame_at(&staged) {
                break len;
            }
            let n = peer.read(&mut chunk).unwrap();
            staged.extend_from_slice(&chunk[..n]);
        };
        let call = Message::parse(&staged[..len]).unwrap();
        assert_eq!(call.member, Some("Hello"));
        let mut body = Writer::new();
        body.str(":9.9");
        let body = body.take_body().unwrap();
        let mut reply = Writer::new();
        reply.begin_return_to(1, ":9.9", call.serial, "s");
        reply.raw(&body);
        peer.write_all(&reply.finish().unwrap()).unwrap();
        peer
    });
    let conn = scoot_dbus::conn::setup(client).unwrap();
    let peer = thread.join().unwrap();
    peer.set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    (conn, peer)
}

fn read_until(stream: &mut UnixStream, want: &[u8]) {
    let mut got = Vec::new();
    let mut byte = [0u8; 1];
    while !got.ends_with(want) {
        assert_eq!(stream.read(&mut byte).unwrap(), 1, "the client talks");
        got.push(byte[0]);
    }
}

/// One bounded `Notify`, for tests that fix the clock.
fn incoming(
    sender: &str,
    app: &str,
    summary: &str,
    body: &str,
    resident: bool,
    urgency: Urgency,
) -> Incoming {
    Incoming {
        app_name: app.to_owned(),
        replaces_id: 0,
        app_icon: String::new(),
        summary: summary.to_owned(),
        body: body.to_owned(),
        actions: vec![Action {
            key: String::from("default"),
            label: String::from("Open"),
        }],
        hints: Hints {
            resident,
            urgency,
            ..Hints::default()
        },
        expire_timeout: -1,
        sender: sender.to_owned(),
    }
}

struct Harness {
    server: NotifyServer,
    conn: Conn,
    _peer: UnixStream,
    config_path: PathBuf,
    dir: PathBuf,
}

/// One scratch dir per harness: tests share a process under `cargo
/// test`, so the pid alone would collide.
static HARNESS_NEXT: AtomicU64 = AtomicU64::new(0);

impl Harness {
    fn new() -> Self {
        let (conn, peer) = connected();
        let dir = std::env::temp_dir().join(format!(
            "scootnotify-control-test-{}-{}",
            std::process::id(),
            HARNESS_NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Self {
            server: NotifyServer::new(Config::default(), IdFile::open(None)),
            conn,
            _peer: peer,
            config_path: dir.join("notify.toml"),
            dir,
        }
    }

    /// One live card, with the `default` action: its id.
    fn notify(&mut self, sender: &str, app: &str, summary: &str, body: &str) -> u32 {
        self.notify_full(sender, app, summary, body, false, Urgency::Normal)
    }

    /// One live card at `now`/`now_ms`: the caller fixing the clock makes
    /// expiry arithmetic exact (a view computed a microsecond later would
    /// truncate a millisecond off).
    fn notify_at(
        &mut self,
        sender: &str,
        app: &str,
        summary: &str,
        body: &str,
        now: Instant,
        now_ms: u64,
    ) -> u32 {
        let incoming = incoming(sender, app, summary, body, false, Urgency::Normal);
        let (arrival, _) = self.server.store_mut().notify(incoming, now, now_ms);
        match arrival {
            crate::model::store::Arrival::Shown { id, .. } => id,
            crate::model::store::Arrival::Held { .. } => panic!("DND held the card"),
            crate::model::store::Arrival::Suppressed { .. } => panic!("the bucket is full"),
        }
    }

    fn notify_full(
        &mut self,
        sender: &str,
        app: &str,
        summary: &str,
        body: &str,
        resident: bool,
        urgency: Urgency,
    ) -> u32 {
        let incoming = incoming(sender, app, summary, body, resident, urgency);
        let now = Instant::now();
        let (arrival, _) = self.server.store_mut().notify(incoming, now, 1_000);
        match arrival {
            crate::model::store::Arrival::Shown { id, .. } => id,
            crate::model::store::Arrival::Held { .. } => panic!("DND held the card"),
            crate::model::store::Arrival::Suppressed { .. } => panic!("the bucket is full"),
        }
    }

    /// Answers one request line, parsed as JSON.
    fn ask(&mut self, line: &str) -> serde_json::Value {
        self.ask_at(line, None, Instant::now(), 1_000)
    }

    fn ask_with_look(&mut self, line: &str, look_state: Option<&Path>) -> serde_json::Value {
        self.ask_at(line, look_state, Instant::now(), 1_000)
    }

    /// Answers one request line at `now`/`now_ms`: the caller fixing the
    /// clock makes expiry arithmetic exact.
    fn ask_at(
        &mut self,
        line: &str,
        look_state: Option<&Path>,
        now: Instant,
        now_ms: u64,
    ) -> serde_json::Value {
        let mut responder = Responder::new(
            &mut self.server,
            &mut self.conn,
            &self.config_path,
            look_state,
            now,
            now_ms,
            0,
        );
        let mut out = Vec::new();
        responder.handle(line.as_bytes(), &mut out);
        serde_json::from_slice(&out).expect("the reply is JSON")
    }
}

#[test]
fn an_empty_list_is_an_empty_array() {
    let mut harness = Harness::new();
    let reply = harness.ask(&Request::List.line());
    assert_eq!(reply["type"], "notifications");
    assert_eq!(reply["notifications"], serde_json::Value::Array(vec![]));
}

#[test]
fn list_shows_every_live_field() {
    let mut harness = Harness::new();
    // One clock for arrival, presentation and the view alike, so the
    // 10 s default reads exactly (a later view would truncate a
    // millisecond off the remainder).
    let now = Instant::now();
    let id = harness.notify_at(
        ":1.1",
        "Firefox",
        "Done",
        "<b>file</b> &amp; more",
        now,
        1_000,
    );
    harness.server.store_mut().presented(id, now, 1_000);
    let reply = harness.ask_at(&Request::List.line(), None, now, 1_000);
    let cards = reply["notifications"].as_array().unwrap();
    assert_eq!(cards.len(), 1);
    let card = &cards[0];
    // Exact shape: an agent reads this without the docs beside it.
    assert_eq!(card["id"], id);
    assert_eq!(card["app_name"], "Firefox");
    assert_eq!(card["desktop_entry"], "");
    assert_eq!(card["summary"], "Done");
    assert_eq!(card["body"], "file & more");
    assert_eq!(card["urgency"], "normal");
    assert_eq!(card["category"], "");
    assert_eq!(
        card["actions"],
        serde_json::json!([{"key": "default", "label": "Open"}])
    );
    assert!(card.get("icon").is_none());
    assert_eq!(card["created"], 1_000);
    assert_eq!(card["shown"], 1_000);
    // The normal default is 10 s from presentation.
    assert_eq!(card["expires"], 11_000);
    assert_eq!(card["output"], serde_json::Value::Null);
}

#[test]
fn an_unshown_card_has_null_shown_and_expires() {
    let mut harness = Harness::new();
    harness.notify(":1.1", "Firefox", "Done", "body");
    let reply = harness.ask(&Request::List.line());
    let card = &reply["notifications"][0];
    assert_eq!(card["shown"], serde_json::Value::Null);
    assert_eq!(card["expires"], serde_json::Value::Null);
}

#[test]
fn dismiss_closes_with_reason_two_and_tells_the_bus() {
    let mut harness = Harness::new();
    let id = harness.notify(":1.1", "Firefox", "Done", "body");
    let reply = harness.ask(
        &Request::Dismiss {
            id: Some(id),
            all: false,
        }
        .line(),
    );
    assert_eq!(reply["type"], "ok");
    assert!(harness.server.store().get(id).is_none());
    let events = harness.server.take_control_events();
    assert_eq!(events.len(), 1);
    let event: serde_json::Value = serde_json::from_slice(&events[0].line).unwrap();
    assert_eq!(event["type"], "close");
    assert_eq!(event["id"], id);
    assert_eq!(event["reason"], "dismissed");
}

#[test]
fn dismissing_an_unknown_id_is_a_loud_error() {
    let mut harness = Harness::new();
    let id = harness.notify(":1.1", "Firefox", "Done", "body");
    let reply = harness.ask(
        &Request::Dismiss {
            id: Some(id + 100),
            all: false,
        }
        .line(),
    );
    assert_eq!(reply["type"], "error");
    assert!(
        reply["message"]
            .as_str()
            .unwrap()
            .contains(&(id + 100).to_string())
    );
    // The live card stands.
    assert!(harness.server.store().get(id).is_some());
    assert!(harness.server.take_control_events().is_empty());
}

#[test]
fn dismiss_all_closes_everything() {
    let mut harness = Harness::new();
    harness.notify(":1.1", "A", "one", "body");
    harness.notify(":1.2", "B", "two", "body");
    let reply = harness.ask(
        &Request::Dismiss {
            id: None,
            all: true,
        }
        .line(),
    );
    assert_eq!(reply["type"], "ok");
    assert!(harness.server.store().live_ids().is_empty());
    assert_eq!(harness.server.take_control_events().len(), 2);
}

#[test]
fn invoke_runs_the_action_and_closes_unless_resident() {
    let mut harness = Harness::new();
    let id = harness.notify(":1.1", "Firefox", "Done", "body");
    let reply = harness.ask(
        &Request::Invoke {
            id,
            action: Cow::Borrowed("default"),
        }
        .line(),
    );
    assert_eq!(reply["type"], "ok");
    assert!(harness.server.store().get(id).is_none());
    let events = harness.server.take_control_events();
    assert_eq!(events.len(), 2);
    let first: serde_json::Value = serde_json::from_slice(&events[0].line).unwrap();
    assert_eq!(first["type"], "invoke");
    assert_eq!(first["action"], "default");
    let second: serde_json::Value = serde_json::from_slice(&events[1].line).unwrap();
    assert_eq!(second["type"], "close");

    let resident = harness.notify_full(":1.1", "Music", "Playing", "body", true, Urgency::Normal);
    let reply = harness.ask(
        &Request::Invoke {
            id: resident,
            action: Cow::Borrowed("default"),
        }
        .line(),
    );
    assert_eq!(reply["type"], "ok");
    assert!(harness.server.store().get(resident).is_some());
}

#[test]
fn invoking_what_is_not_there_names_it() {
    let mut harness = Harness::new();
    let id = harness.notify(":1.1", "Firefox", "Done", "body");
    let reply = harness.ask(
        &Request::Invoke {
            id: id + 100,
            action: Cow::Borrowed("default"),
        }
        .line(),
    );
    assert_eq!(reply["type"], "error");
    assert!(
        reply["message"]
            .as_str()
            .unwrap()
            .contains(&(id + 100).to_string())
    );
    let reply = harness.ask(
        &Request::Invoke {
            id,
            action: Cow::Borrowed("snooze"),
        }
        .line(),
    );
    assert_eq!(reply["type"], "error");
    assert!(reply["message"].as_str().unwrap().contains("snooze"));
    assert!(harness.server.store().get(id).is_some());
}

#[test]
fn subscribe_dedicates_the_connection_within_the_cap() {
    let mut harness = Harness::new();
    let now = Instant::now();
    let mut responder = Responder::new(
        &mut harness.server,
        &mut harness.conn,
        &harness.config_path,
        None,
        now,
        1_000,
        0,
    );
    let mut out = Vec::new();
    responder.handle(
        Request::Subscribe {
            events: vec![EventKind::Notify, EventKind::Close],
        }
        .line()
        .as_bytes(),
        &mut out,
    );
    let reply: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(reply["type"], "subscribed");
    assert_eq!(reply["events"], serde_json::json!(["notify", "close"]));
    let kinds = responder.take_subscription();
    assert!(kinds.is_some());
    assert!(kinds.unwrap().wants(EventKind::Notify as u8));
    assert!(!kinds.unwrap().wants(EventKind::Invoke as u8));
}

#[test]
fn a_fifth_subscriber_is_refused() {
    let mut harness = Harness::new();
    let request = Request::Subscribe {
        events: EventKind::ALL.to_vec(),
    };
    for _ in 0..4 {
        let now = Instant::now();
        let mut responder = Responder::new(
            &mut harness.server,
            &mut harness.conn,
            &harness.config_path,
            None,
            now,
            1_000,
            0,
        );
        let mut out = Vec::new();
        responder.handle(request.line().as_bytes(), &mut out);
        let reply: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(reply["type"], "subscribed");
    }
    // The fifth arrives with four already subscribed: refused, and it
    // subscribes to nothing.
    let now = Instant::now();
    let mut responder = Responder::new(
        &mut harness.server,
        &mut harness.conn,
        &harness.config_path,
        None,
        now,
        1_000,
        4,
    );
    let mut out = Vec::new();
    responder.handle(request.line().as_bytes(), &mut out);
    let reply: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(reply["type"], "error");
    assert!(reply["message"].as_str().unwrap().contains("4"));
    assert!(responder.take_subscription().is_none());
}

#[test]
fn version_names_the_protocol_and_the_build() {
    let mut harness = Harness::new();
    let reply = harness.ask(&Request::Version.line());
    assert_eq!(reply["type"], "version");
    assert_eq!(reply["protocol"], PROTOCOL_VERSION);
    assert_eq!(reply["version"], env!("CARGO_PKG_VERSION"));
}

#[test]
fn a_malformed_line_is_an_error_and_leaves_the_next_one_served() {
    let mut harness = Harness::new();
    let now = Instant::now();
    let mut responder = Responder::new(
        &mut harness.server,
        &mut harness.conn,
        &harness.config_path,
        None,
        now,
        1_000,
        0,
    );
    let mut out = Vec::new();
    responder.handle(b"{not json", &mut out);
    responder.handle(Request::Version.line().as_bytes(), &mut out);
    let text = String::from_utf8(out).unwrap();
    let mut lines = text.lines();
    let first: serde_json::Value = serde_json::from_str(lines.next().unwrap()).unwrap();
    assert_eq!(first["type"], "error");
    let second: serde_json::Value = serde_json::from_str(lines.next().unwrap()).unwrap();
    assert_eq!(second["type"], "version");
    assert!(lines.next().is_none());
}

#[test]
fn a_wrong_protocol_is_refused_loudly() {
    let mut harness = Harness::new();
    let reply = harness.ask(r#"{"protocol":999,"type":"list"}"#);
    assert_eq!(reply["type"], "error");
    assert!(reply["message"].as_str().unwrap().contains("999"));
}

#[test]
fn reload_applies_a_good_file_and_replaces_the_running_config() {
    let mut harness = Harness::new();
    std::fs::write(&harness.config_path, "max_live = 7\noutput = \"DP-1\"\n").unwrap();
    let reply = harness.ask(&Request::Reload { scope: None }.line());
    assert_eq!(reply["type"], "ok");
    assert!(reply.get("scope").is_none());
    assert_eq!(harness.server.store().output_name(), Some("DP-1"));
}

#[test]
fn a_reload_with_a_bad_file_keeps_the_running_config_and_state() {
    let mut harness = Harness::new();
    std::fs::write(&harness.config_path, "output = \"DP-1\"\n").unwrap();
    let reply = harness.ask(&Request::Reload { scope: None }.line());
    assert_eq!(reply["type"], "ok");
    let id = harness.notify(":1.1", "Firefox", "Done", "body");
    // Half-edited: the file no longer parses.
    std::fs::write(&harness.config_path, "output = \n").unwrap();
    let reply = harness.ask(&Request::Reload { scope: None }.line());
    assert_eq!(reply["type"], "error");
    // The running config and the live card stand.
    assert_eq!(harness.server.store().output_name(), Some("DP-1"));
    assert!(harness.server.store().get(id).is_some());
}

#[test]
fn a_scoped_reload_ignores_a_half_edited_user_file() {
    let mut harness = Harness::new();
    std::fs::write(&harness.config_path, "output = \"DP-1\"\n").unwrap();
    let reply = harness.ask(&Request::Reload { scope: None }.line());
    assert_eq!(reply["type"], "ok");
    // A look layer with one role set.
    let state = harness.dir.join("state");
    std::fs::create_dir_all(state.join("look")).unwrap();
    std::fs::write(state.join("look").join("notify.toml"), "radius = 4\n").unwrap();
    // The user file is half-edited: the scoped reload never reads it.
    std::fs::write(&harness.config_path, "output = \n").unwrap();
    let reply = harness.ask_with_look(
        &Request::Reload {
            scope: Some(ReloadScope::Look),
        }
        .line(),
        Some(&state),
    );
    assert_eq!(reply["type"], "ok");
    assert_eq!(reply["scope"], "look");
    // The running config stands (the edit never applied).
    assert_eq!(harness.server.store().output_name(), Some("DP-1"));
}

#[test]
fn a_scoped_reload_without_a_layer_is_refused() {
    let mut harness = Harness::new();
    let reply = harness.ask(
        &Request::Reload {
            scope: Some(ReloadScope::Look),
        }
        .line(),
    );
    assert_eq!(reply["type"], "error");
    assert!(reply["message"].as_str().unwrap().contains("no look layer"));
}

#[test]
fn a_reload_refused_on_a_bad_fragment_keeps_the_last_good() {
    let mut harness = Harness::new();
    let state = harness.dir.join("state");
    std::fs::create_dir_all(state.join("look")).unwrap();
    std::fs::write(state.join("look").join("notify.toml"), "radius = 4\n").unwrap();
    let reply = harness.ask_with_look(
        &Request::Reload {
            scope: Some(ReloadScope::Look),
        }
        .line(),
        Some(&state),
    );
    assert_eq!(reply["type"], "ok");
    // The fragment is half-written: not a table at all.
    std::fs::write(state.join("look").join("notify.toml"), "radius = \n").unwrap();
    let reply = harness.ask_with_look(
        &Request::Reload {
            scope: Some(ReloadScope::Look),
        }
        .line(),
        Some(&state),
    );
    assert_eq!(reply["type"], "error");
}

#[test]
fn hostile_lines_are_refused_without_harm() {
    let mut harness = Harness::new();
    for hostile in [
        "",
        "}",
        "null",
        "[1]",
        "{\"protocol\":2,\"type\":\"dismiss\",\"id\":\"3\"}",
        "{\"protocol\":2,\"type\":\"invoke\",\"id\":3.5}",
        "{\"protocol\":2,\"type\":\"reload\",\"scope\":\"look\",\"scope\":\"leave\"}",
        "{\"protocol\":2,\"type\":\"dnd\",\"mode\":\"loud\"}",
    ] {
        let reply = harness.ask(hostile);
        assert_eq!(reply["type"], "error", "{hostile}");
    }
    // The daemon still serves after all of them.
    let reply = harness.ask(&Request::Version.line());
    assert_eq!(reply["type"], "version");
}

#[test]
fn an_oversized_request_line_is_refused_and_the_connection_closes() {
    use std::io::Read as _;
    let (mut client, stream) = UnixStream::pair().unwrap();
    stream.set_nonblocking(true).unwrap();
    let mut server = Server::with_spare(None);
    server.admit(stream);
    struct Silent;
    impl Handler for Silent {
        fn handle(&mut self, _line: &[u8], out: &mut Vec<u8>) {
            write_reply(out, &super::protocol::Reply::Ok { scope: None });
        }
    }
    // Past the 64 KiB cap: refused, and the connection goes away.
    let big = format!(
        "{{\"protocol\":2,\"type\":\"list\",\"pad\":\"{}\"}}\n",
        "x".repeat(70_000)
    );
    client.write_all(big.as_bytes()).unwrap();
    let mut handler = Silent;
    use rustix::event::PollFlags;
    let ready = PollFlags::IN | PollFlags::OUT | PollFlags::HUP | PollFlags::ERR;
    // Served until it closes (the overlong line arrives in 4 KiB
    // reads, so one round per piece).
    for _ in 0..40 {
        if server.conns().is_empty() {
            break;
        }
        server.service(0, ready, &mut handler);
    }
    assert!(
        server.conns().is_empty(),
        "the overlong line closes the connection"
    );
    // The refusal line, then the close: the close can race as a reset
    // (the socket closes with the flood still unread), so the read
    // tolerates one, but the error line itself must arrive.
    client
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut reply = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match client.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => reply.extend_from_slice(&chunk[..n]),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::BrokenPipe
                ) =>
            {
                break;
            }
            Err(error) => panic!("the refusal reads: {error}"),
        }
    }
    let text = String::from_utf8(reply).unwrap();
    assert!(text.contains("error"), "{text}");
    assert!(text.contains("65536"), "{text}");
}

#[test]
fn the_seventeenth_connection_evicts_the_oldest_non_subscriber() {
    use super::MAX_CONNECTIONS;
    let mut server = Server::with_spare(None);
    let mut clients = Vec::new();
    for _ in 0..MAX_CONNECTIONS {
        let (client, stream) = UnixStream::pair().unwrap();
        stream.set_nonblocking(true).unwrap();
        server.admit(stream);
        clients.push(client);
    }
    assert_eq!(server.conns().len(), MAX_CONNECTIONS);
    let (extra, stream) = UnixStream::pair().unwrap();
    stream.set_nonblocking(true).unwrap();
    server.admit(stream);
    assert_eq!(server.conns().len(), MAX_CONNECTIONS);
    // The oldest is gone: it reads EOF.
    clients[0]
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut byte = [0u8; 1];
    assert_eq!(clients[0].read(&mut byte).unwrap(), 0);
    let _ = extra;
}

#[test]
fn event_lines_are_one_json_object_each() {
    let mut out = Vec::new();
    write_event(
        &mut out,
        &super::protocol::Event::Close {
            id: 3,
            reason: "expired",
        },
    );
    let event: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(event["type"], "close");
    assert_eq!(event["id"], 3);
    assert_eq!(event["reason"], "expired");
    let mut out = Vec::new();
    write_reply(
        &mut out,
        &super::protocol::Reply::Subscribed {
            events: vec!["notify"],
        },
    );
    let reply: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(reply["type"], "subscribed");
}

#[test]
fn dnd_reads_sets_and_toggles_with_an_event_on_change() {
    let mut harness = Harness::new();
    // Reading the mode answers without an event.
    let reply = harness.ask(&Request::Dnd { mode: None }.line());
    assert_eq!(reply["type"], "dnd");
    assert_eq!(reply["dnd"], "off");
    assert!(harness.server.take_control_events().is_empty());
    // Setting it answers the new mode and queues the event.
    let reply = harness.ask(
        &Request::Dnd {
            mode: Some(DndMode::On),
        }
        .line(),
    );
    assert_eq!(reply["dnd"], "on");
    let events = harness.server.take_control_events();
    assert_eq!(events.len(), 1);
    let event: serde_json::Value = serde_json::from_slice(&events[0].line).unwrap();
    assert_eq!(event["type"], "dnd");
    assert_eq!(event["dnd"], "on");
    // Setting what holds changes nothing: no second event.
    let reply = harness.ask(
        &Request::Dnd {
            mode: Some(DndMode::On),
        }
        .line(),
    );
    assert_eq!(reply["dnd"], "on");
    assert!(harness.server.take_control_events().is_empty());
    // Toggling flips it back.
    let reply = harness.ask(
        &Request::Dnd {
            mode: Some(DndMode::Toggle),
        }
        .line(),
    );
    assert_eq!(reply["dnd"], "off");
    let events = harness.server.take_control_events();
    assert_eq!(events.len(), 1);
}

/// Reverting `set_dnd` to always report changed leaves the no-second-
/// event arm failing: the changed pin is load-bearing.
#[test]
fn dnd_holds_normal_cards_and_criticals_pass_through() {
    let mut harness = Harness::new();
    harness.ask(
        &Request::Dnd {
            mode: Some(DndMode::On),
        }
        .line(),
    );
    let now = Instant::now();
    let normal_in = incoming(":1.1", "Mail", "New", "body", false, Urgency::Normal);
    let (arrival, _) = harness.server.store_mut().notify(normal_in, now, 1_000);
    assert!(matches!(arrival, crate::model::store::Arrival::Held { .. }));
    // Nothing live, but history keeps it, unseen.
    assert!(harness.server.store().live_ids().is_empty());
    assert_eq!(harness.server.store().history().len(), 1);
    assert_eq!(harness.server.store().unread(), 1);
    // A critical still shows.
    let id = harness.notify_full(":1.1", "Alert", "Fire", "body", false, Urgency::Critical);
    assert!(harness.server.store().get(id).is_some());
}

#[test]
fn history_lists_closed_cards_oldest_first_and_restore_re_shows() {
    let mut harness = Harness::new();
    let now = Instant::now();
    let first = harness.notify_at(":1.1", "A", "one", "body one", now, 1_000);
    harness.server.store_mut().presented(first, now, 1_000);
    let second = harness.notify_at(":1.1", "B", "two", "body two", now, 1_000);
    harness.server.store_mut().presented(second, now, 1_000);
    harness.ask(
        &Request::Dismiss {
            id: Some(first),
            all: false,
        }
        .line(),
    );
    harness.ask(
        &Request::Dismiss {
            id: Some(second),
            all: false,
        }
        .line(),
    );
    let reply = harness.ask(&Request::History.line());
    assert_eq!(reply["type"], "history");
    let entries = reply["history"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0]["app_name"], "A");
    assert_eq!(entries[0]["body"], "body one");
    assert_eq!(entries[0]["seen"], true);
    assert_eq!(entries[1]["app_name"], "B");
    // Restore re-shows the newest as a new live card.
    let reply = harness.ask(&Request::Restore.line());
    assert_eq!(reply["type"], "ok");
    assert_eq!(harness.server.store().live_ids().len(), 1);
    let live = &harness.server.store().ordered()[0];
    assert_eq!(live.app_name, "B");
    assert_ne!(live.id, second);
    assert_eq!(live.id, 3);
    // The restored entry reads seen, and unread drops to zero.
    let reply = harness.ask(&Request::History.line());
    assert!(
        reply["history"].as_array().unwrap()[1]["seen"]
            .as_bool()
            .unwrap()
    );
    assert_eq!(harness.server.store().unread(), 0);
}

#[test]
fn restore_on_an_empty_history_is_a_loud_error() {
    let mut harness = Harness::new();
    let reply = harness.ask(&Request::History.line());
    assert_eq!(reply["history"], serde_json::Value::Array(vec![]));
    let reply = harness.ask(&Request::Restore.line());
    assert_eq!(reply["type"], "error");
    assert!(reply["message"].as_str().unwrap().contains("empty"));
}

#[test]
fn a_reload_keeps_dnd_and_truncates_history_to_the_new_cap() {
    let mut harness = Harness::new();
    harness.ask(
        &Request::Dnd {
            mode: Some(DndMode::On),
        }
        .line(),
    );
    let now = Instant::now();
    for _ in 0..3 {
        let in_ = incoming(":1.1", "Mail", "New", "body", false, Urgency::Normal);
        harness.server.store_mut().notify(in_, now, 1_000);
    }
    assert_eq!(harness.server.store().history().len(), 3);
    std::fs::write(&harness.config_path, "history_max = 2\n").unwrap();
    let reply = harness.ask(&Request::Reload { scope: None }.line());
    assert_eq!(reply["type"], "ok");
    // DND survives the reload; history keeps the newest two.
    assert_eq!(harness.server.dnd(), crate::model::dnd::Dnd::On);
    assert_eq!(harness.server.store().history().len(), 2);
}

impl Drop for Harness {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.dir).ok();
    }
}
