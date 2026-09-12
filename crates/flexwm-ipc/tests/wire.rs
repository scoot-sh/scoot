//! The JSON shapes clients depend on. A change that breaks one of these needs a
//! `PROTOCOL_VERSION` bump.

use flexwm_ipc::{
    Action, Horizontal, PointerButton, Request, Response, Screenshot, decode, encode,
};
use serde_json::{Value, json};

fn json_of<T: serde::Serialize>(value: &T) -> Value {
    serde_json::to_value(value).unwrap()
}

#[test]
fn unit_requests_are_just_a_type() {
    assert_eq!(json_of(&Request::Windows), json!({ "type": "windows" }));
}

#[test]
fn actions_flatten_into_the_request() {
    let request = Request::Action(Action::FocusColumn {
        direction: Horizontal::Left,
    });
    assert_eq!(
        json_of(&request),
        json!({ "type": "action", "action": "focus_column", "direction": "left" })
    );
}

#[test]
fn key_combos_travel_as_strings() {
    let request = Request::Key {
        keys: "ctrl+shift+t".parse().unwrap(),
    };
    assert_eq!(
        json_of(&request),
        json!({ "type": "key", "keys": "ctrl+shift+t" })
    );
}

#[test]
fn click_defaults_to_the_left_button() {
    let request: Request = decode(r#"{"type":"click","x":10,"y":20}"#).unwrap();
    assert_eq!(
        request,
        Request::Click {
            x: 10.0,
            y: 20.0,
            button: PointerButton::Left
        }
    );
}

#[test]
fn screenshots_carry_png_bytes_as_base64() {
    let response = Response::Screenshot(Screenshot {
        width: 2,
        height: 1,
        png: vec![0x89, b'P', b'N', b'G'],
    });
    assert_eq!(
        json_of(&response),
        json!({ "type": "screenshot", "width": 2, "height": 1, "png": "iVBORw==" })
    );
    assert_eq!(
        decode::<Response>(&encode(&response).unwrap()).unwrap(),
        response
    );
}

#[test]
fn warnings_carry_a_message_and_round_trip() {
    let response = Response::Warning {
        message: "switched away from this session over IPC".into(),
    };
    assert_eq!(
        json_of(&response),
        json!({
            "type": "warning",
            "message": "switched away from this session over IPC",
        })
    );
    assert_eq!(
        decode::<Response>(&encode(&response).unwrap()).unwrap(),
        response
    );
}

#[test]
fn every_request_round_trips_on_one_line() {
    let requests = [
        Request::Version,
        Request::Outputs,
        Request::Windows,
        Request::Action(Action::Spawn {
            command: vec!["foot".into()],
        }),
        Request::Screenshot { output: Some(1) },
        Request::Screenshot { output: None },
        Request::PointerMove { x: 1.5, y: 2.0 },
        Request::PointerButton {
            button: PointerButton::Right,
            pressed: true,
        },
        Request::Click {
            x: 3.0,
            y: 4.0,
            button: PointerButton::Middle,
        },
        Request::Scroll { dx: 0.0, dy: -15.0 },
        Request::Key {
            keys: "Return".parse().unwrap(),
        },
        Request::Type {
            text: "hello\nworld".into(),
        },
        Request::WaitIdle {
            quiet_ms: 200,
            timeout_ms: 5000,
        },
    ];
    for request in requests {
        let line = encode(&request).unwrap();
        assert!(
            line.ends_with('\n') && !line.trim_end().contains('\n'),
            "{line}"
        );
        assert_eq!(decode::<Request>(&line).unwrap(), request);
    }
}

#[test]
fn unknown_request_types_are_rejected() {
    assert!(decode::<Request>(r#"{"type":"format_disk"}"#).is_err());
}
