//! Unit tests for the session table and the D-Bus body shapes.

use super::*;
use scoot_dbus::proto::Reader;

#[test]
fn create_and_remove_round_trips() {
    let mut sessions = Sessions::new();
    assert!(sessions.get("/org/xdp/s1").is_none());
    sessions
        .create(
            String::from("/org/xdp/s1"),
            String::from("org.example"),
            String::from(":1.7"),
        )
        .unwrap();
    let session = sessions.get("/org/xdp/s1").unwrap();
    assert_eq!(session.app_id, "org.example");
    let removed = sessions.remove("/org/xdp/s1").unwrap();
    assert_eq!(removed.sender, ":1.7");
    assert!(sessions.get("/org/xdp/s1").is_none());
}

#[test]
fn duplicate_handle_for_another_app_is_refused() {
    let mut sessions = Sessions::new();
    sessions
        .create(
            String::from("/org/xdp/s1"),
            String::from("org.a"),
            String::from(":1.7"),
        )
        .unwrap();
    assert!(
        sessions
            .create(
                String::from("/org/xdp/s1"),
                String::from("org.b"),
                String::from(":1.8"),
            )
            .is_err()
    );
}

#[test]
fn sender_death_drops_only_its_sessions() {
    let mut sessions = Sessions::new();
    sessions
        .create(
            String::from("/s/1"),
            String::from("org.a"),
            String::from(":1.7"),
        )
        .unwrap();
    sessions
        .create(
            String::from("/s/2"),
            String::from("org.b"),
            String::from(":1.8"),
        )
        .unwrap();
    let dropped = sessions.remove_sender(":1.7");
    assert_eq!(dropped.len(), 1);
    assert_eq!(dropped[0].0, "/s/1");
    assert!(sessions.get("/s/1").is_none());
    assert!(sessions.get("/s/2").is_some());
}

#[test]
fn table_caps_sessions() {
    let mut sessions = Sessions::new();
    for index in 0..MAX_SESSIONS {
        sessions
            .create(
                format!("/s/{index}"),
                String::from("org.example"),
                String::from(":1.9"),
            )
            .unwrap();
    }
    assert!(
        sessions
            .create(
                String::from("/s/overflow"),
                String::from("org.example"),
                String::from(":1.9"),
            )
            .is_err()
    );
}

#[test]
fn bind_body_round_trips() {
    // The exact shape a portal client sends: one shortcut with a
    // description, then the parent window and empty options.
    let mut body = Writer::new();
    body.str("/req/2");
    body.str("/sess/1");
    let cookie = body.open_array(8).unwrap();
    body.open_struct();
    body.str("toggle");
    let inner = body.open_array(8).unwrap();
    body.open_struct();
    body.str("description");
    body.variant("s");
    body.str("Toggle");
    body.close_struct();
    body.open_struct();
    body.str("preferred_trigger");
    body.variant("s");
    body.str("super+ctrl+r");
    body.close_struct();
    body.close_array(inner);
    body.close_struct();
    body.close_array(cookie);
    body.str("");
    let dict = body.open_array(8).unwrap();
    body.close_array(dict);
    let arg = body.take_body().unwrap();
    let (handle, session, shortcuts, _parent) = parse_bind_shortcuts(&arg).unwrap();
    assert_eq!(handle, "/req/2");
    assert_eq!(session, "/sess/1");
    assert_eq!(shortcuts.len(), 1);
    assert_eq!(shortcuts[0].shortcut_id, "toggle");
    assert_eq!(shortcuts[0].description.as_deref(), Some("Toggle"));
    assert_eq!(
        shortcuts[0].preferred_trigger.as_deref(),
        Some("super+ctrl+r")
    );
}

#[test]
fn response_body_decodes_as_ua_sv() {
    let results = vec![(
        String::from("shortcuts"),
        WriterValue::Shortcuts(vec![ShortcutResult {
            shortcut_id: String::from("toggle"),
            description: Some(String::from("Toggle")),
            trigger_description: Some(String::from("super+ctrl+r")),
        }]),
    )];
    let body = reply_response(&results).unwrap();
    let mut reader = Reader::le(&body);
    assert_eq!(reader.u32().unwrap(), 0);
    let mut dict = reader.elements(8).unwrap();
    assert!(!dict.exhausted());
    dict.enter_struct().unwrap();
    assert_eq!(dict.str().unwrap(), "shortcuts");
    let variant = dict.variant_raw().unwrap();
    assert_eq!(variant.signature(), "a(sa{sv})");
    dict.leave_struct();
    assert!(reader.exhausted());
}

#[test]
fn activation_body_carries_session_id_and_time() {
    let body = signal_activation_body("/s/1", "toggle", 1234).unwrap();
    let mut reader = Reader::le(&body);
    assert_eq!(reader.str().unwrap(), "/s/1");
    assert_eq!(reader.str().unwrap(), "toggle");
    assert_eq!(reader.u64().unwrap(), 1234);
    reader.skip("a{sv}").unwrap();
    assert!(reader.exhausted());
}
