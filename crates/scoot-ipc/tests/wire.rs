//! The JSON shapes clients depend on. A change that breaks one of these needs a
//! `PROTOCOL_VERSION` bump.

use scoot_ipc::{
    Action, Horizontal, OutputSnapshot, PointerButton, Rect, Request, Response, Screenshot,
    WindowSnapshot, decode, encode,
};
use serde_json::{Value, json};

fn json_of<T: serde::Serialize>(value: &T) -> Value {
    serde_json::to_value(value).unwrap()
}

#[test]
fn unit_requests_are_just_a_type() {
    assert_eq!(json_of(&Request::Windows), json!({ "type": "windows" }));
    assert_eq!(json_of(&Request::Reload), json!({ "type": "reload" }));
    assert_eq!(
        decode::<Request>(&encode(&Request::Reload).unwrap()).unwrap(),
        Request::Reload
    );
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
fn the_indexed_workspace_actions_travel_as_snake_case_with_an_index() {
    // Both directions: the exact JSON shape a client sends, and the decode
    // of that shape back into the same request. Pinning both keeps an
    // additive variant honest about what it actually puts on the wire.
    for (action, tag, index) in [
        (
            Action::FocusWorkspaceIndex { index: 2 },
            "focus_workspace_index",
            2,
        ),
        (
            Action::MoveWindowToWorkspaceIndex { index: 3 },
            "move_window_to_workspace_index",
            3,
        ),
    ] {
        let request = Request::Action(action);
        assert_eq!(
            json_of(&request),
            json!({ "type": "action", "action": tag, "index": index })
        );
        assert_eq!(
            decode::<Request>(&encode(&request).unwrap()).unwrap(),
            request
        );
    }
}

#[test]
fn the_output_actions_travel_as_snake_case_with_an_output() {
    // Same pin as the indexed workspace actions above: the exact JSON shape
    // a client sends, and its decode back into the same request.
    for (action, tag) in [
        (
            Action::MoveFocusedWindowToOutput { output: 2 },
            "move_focused_window_to_output",
        ),
        (Action::FocusOutput { output: 2 }, "focus_output"),
    ] {
        let request = Request::Action(action);
        assert_eq!(
            json_of(&request),
            json!({ "type": "action", "action": tag, "output": 2 })
        );
        assert_eq!(
            decode::<Request>(&encode(&request).unwrap()).unwrap(),
            request
        );
    }
}

#[test]
fn unknown_action_tags_are_rejected_like_unknown_request_types() {
    // The no-`PROTOCOL_VERSION`-bump half of the argument: a new action tag
    // is nested inside `Request::Action`, so an older server's decode of the
    // whole request fails exactly the way an unknown `Request` tag does --
    // answered with an ordinary `Error` while the server keeps serving (see
    // the crate's `PROTOCOL_VERSION` doc), never a kill and never a silent
    // misroute. An older client never sends a tag it doesn't know, so the
    // other direction needs nothing.
    assert!(
        decode::<Request>(r#"{"type":"action","action":"hypothetical_future_action"}"#).is_err()
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
fn a_reload_report_round_trips_with_both_lists() {
    let response = Response::Reloaded {
        applied: vec!["layout.gap".into(), "binds".into()],
        refused: vec!["output.scale".into()],
    };
    assert_eq!(
        json_of(&response),
        json!({
            "type": "reloaded",
            "applied": ["layout.gap", "binds"],
            "refused": ["output.scale"],
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
        Request::Reload,
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

/// `scale` is additive and defaulted: an older server that predates the field
/// must still decode, as scale 1.0 -- the only scale such a server ran at.
/// This is what lets the field ship without a `PROTOCOL_VERSION` bump (see
/// the crate's `PROTOCOL_VERSION` doc for the "breaks existing clients" bar).
#[test]
fn an_output_snapshot_defaults_a_missing_scale_to_one() {
    let decoded: OutputSnapshot =
        decode(r#"{"id":1,"name":"headless","rect":{"x":0,"y":0,"width":800,"height":600}}"#)
            .expect("an older server's output snapshot still decodes");
    assert_eq!(decoded.scale, 1.0);
    assert_eq!(
        decoded.rect,
        Rect {
            x: 0,
            y: 0,
            width: 800,
            height: 600
        }
    );
}

#[test]
fn an_output_snapshot_carries_its_scale_on_the_wire() {
    let snapshot = OutputSnapshot {
        id: 1,
        name: "headless".into(),
        rect: Rect {
            x: 0,
            y: 0,
            width: 1920,
            height: 1200,
        },
        scale: 1.5,
        usable: Rect {
            x: 0,
            y: 0,
            width: 1920,
            height: 1200,
        },
    };
    assert_eq!(
        json_of(&snapshot),
        json!({
            "id": 1,
            "name": "headless",
            "rect": { "x": 0, "y": 0, "width": 1920, "height": 1200 },
            "scale": 1.5,
            "usable": { "x": 0, "y": 0, "width": 1920, "height": 1200 },
        })
    );
    assert_eq!(
        decode::<OutputSnapshot>(&encode(&snapshot).unwrap()).unwrap(),
        snapshot
    );
}

#[test]
fn an_output_snapshot_carries_its_usable_rect_on_the_wire() {
    let snapshot = OutputSnapshot {
        id: 1,
        name: "headless".into(),
        rect: Rect {
            x: 0,
            y: 0,
            width: 1920,
            height: 1200,
        },
        scale: 1.0,
        usable: Rect {
            x: 0,
            y: 30,
            width: 1920,
            height: 1170,
        },
    };
    assert_eq!(
        json_of(&snapshot),
        json!({
            "id": 1,
            "name": "headless",
            "rect": { "x": 0, "y": 0, "width": 1920, "height": 1200 },
            "scale": 1.0,
            "usable": { "x": 0, "y": 30, "width": 1920, "height": 1170 },
        })
    );
}

/// An older server's reply -- no `usable`, no `scale` -- still decodes:
/// `usable` falls back to the all-zero sentinel (never a real area, so a
/// client can tell "predates the field" apart from a value), exactly the
/// contract that keeps this off the `PROTOCOL_VERSION`-bump list.
#[test]
fn an_output_snapshot_from_before_usable_still_decodes() {
    let snapshot: OutputSnapshot =
        decode(r#"{"id":1,"name":"headless","rect":{"x":0,"y":0,"width":1920,"height":1200}}"#)
            .unwrap();
    assert_eq!(snapshot.scale, 1.0);
    assert_eq!(
        snapshot.usable,
        Rect {
            x: 0,
            y: 0,
            width: 0,
            height: 0
        }
    );
}

/// Same contract for the lock flag: `{"type":"ok"}` from an older server
/// decodes with `locked: false`, and a new server's reply still decodes
/// for an older client (serde ignores the unknown field -- asserted here
/// by decoding with a struct that lacks it, the way an old client would).
#[test]
fn an_ok_from_before_locked_still_decodes() {
    let response: Response = decode(r#"{"type":"ok"}"#).unwrap();
    assert_eq!(response, Response::Ok { locked: false });

    #[derive(serde::Deserialize, PartialEq, Debug)]
    #[serde(tag = "type", rename_all = "snake_case")]
    enum OldResponse {
        Ok,
    }
    let old: OldResponse = decode(r#"{"type":"ok","locked":true}"#).unwrap();
    assert_eq!(old, OldResponse::Ok);
}

/// `popup_grab` is additive and defaulted like `icon` before it: an older
/// server's window snapshot still decodes (as "no grab"), and a new server's
/// reply still decodes for an older client -- which is what keeps the field
/// off the `PROTOCOL_VERSION`-bump list.
#[test]
fn a_window_snapshot_from_before_popup_grab_still_decodes() {
    let snapshot: WindowSnapshot = decode(
        r#"{"id":1,"app_id":"foot","title":"zsh","output":1,"rect":{"x":0,"y":0,"width":800,"height":600},"visible":true,"focused":true}"#,
    )
    .expect("an older server's window snapshot still decodes");
    assert!(!snapshot.popup_grab);
}

#[test]
fn a_window_snapshot_carries_its_popup_grab_on_the_wire() {
    let snapshot = WindowSnapshot {
        id: 1,
        app_id: "foot".into(),
        title: "zsh".into(),
        icon: None,
        output: 1,
        rect: Rect {
            x: 0,
            y: 0,
            width: 800,
            height: 600,
        },
        visible: true,
        focused: false,
        popup_grab: true,
    };
    assert_eq!(
        json_of(&snapshot),
        json!({
            "id": 1,
            "app_id": "foot",
            "title": "zsh",
            "icon": null,
            "output": 1,
            "rect": { "x": 0, "y": 0, "width": 800, "height": 600 },
            "visible": true,
            "focused": false,
            "popup_grab": true,
        })
    );
    assert_eq!(
        decode::<WindowSnapshot>(&encode(&snapshot).unwrap()).unwrap(),
        snapshot
    );

    // ...and an older client, decoding with a struct that lacks the field,
    // ignores it rather than failing.
    #[derive(serde::Deserialize, PartialEq, Debug)]
    struct OldWindowSnapshot {
        id: u64,
        focused: bool,
    }
    let old: OldWindowSnapshot = decode(&encode(&snapshot).unwrap()).unwrap();
    assert_eq!(
        old,
        OldWindowSnapshot {
            id: 1,
            focused: false,
        }
    );
}
