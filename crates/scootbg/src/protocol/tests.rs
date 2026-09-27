use super::{OutputEntry, PROTOCOL_VERSION, Reply, Request, RequestError, parse, write_reply};

fn reply_string(reply: &Reply<'_>) -> String {
    let mut out = Vec::new();
    write_reply(&mut out, reply);
    String::from_utf8(out).unwrap()
}

#[test]
fn each_request_parses() {
    for request in [Request::Query, Request::Kill, Request::Version] {
        let line = request.line();
        assert!(line.ends_with('\n'));
        assert_eq!(parse(line.trim_end().as_bytes()).unwrap(), request);
    }
}

#[test]
fn field_order_whitespace_and_extra_fields_do_not_matter() {
    let line = br#" { "future": [1, 2], "type" : "query", "protocol" : 1 } "#;
    assert_eq!(parse(line).unwrap(), Request::Query);
    // A trailing carriage return is whitespace to JSON.
    assert_eq!(
        parse(b"{\"protocol\":1,\"type\":\"kill\"}\r").unwrap(),
        Request::Kill
    );
}

#[test]
fn an_escaped_type_still_parses() {
    let line = br#"{"protocol":1,"type":"query"}"#;
    assert_eq!(parse(line).unwrap(), Request::Query);
}

#[test]
fn malformed_json_is_refused() {
    for line in [
        &b"not json"[..],
        b"",
        b"{",
        b"[1,",
        b"{\"protocol\":1,\"type\":\"query\"} trailing",
        b"\xff\xfe",
        b"{\"protocol\":\"1\",\"type\":\"query\"}",
        b"{\"protocol\":-1,\"type\":\"query\"}",
        b"{\"protocol\":1,\"type\":7}",
    ] {
        assert!(
            matches!(parse(line), Err(RequestError::Malformed(_))),
            "{:?}",
            String::from_utf8_lossy(line)
        );
    }
}

#[test]
fn a_missing_or_wrong_protocol_is_refused() {
    assert!(matches!(
        parse(br#"{"type":"query"}"#),
        Err(RequestError::NoProtocol)
    ));
    assert!(matches!(
        parse(br#"{"protocol":2,"type":"query"}"#),
        Err(RequestError::WrongProtocol(2))
    ));
    assert!(matches!(
        parse(br#"{"protocol":0,"type":"query"}"#),
        Err(RequestError::WrongProtocol(0))
    ));
    let message = RequestError::WrongProtocol(9).to_string();
    assert!(message.contains('9') && message.contains(&PROTOCOL_VERSION.to_string()));
}

#[test]
fn a_missing_or_unknown_type_is_refused() {
    assert!(matches!(
        parse(br#"{"protocol":1}"#),
        Err(RequestError::NoType)
    ));
    match parse(br#"{"protocol":1,"type":"set"}"#) {
        Err(RequestError::Unknown(name)) => assert_eq!(name, "set"),
        other => panic!("expected Unknown, got {other:?}"),
    }
}

#[test]
fn replies_are_one_tagged_line() {
    assert_eq!(reply_string(&Reply::Ok), "{\"type\":\"ok\"}\n");
    assert_eq!(
        reply_string(&Reply::Outputs {
            outputs: &[] as &[OutputEntry<'_>; 0]
        }),
        "{\"type\":\"outputs\",\"outputs\":[]}\n"
    );
    assert_eq!(
        reply_string(&Reply::Version {
            protocol: 1,
            version: "0.1.0"
        }),
        "{\"type\":\"version\",\"protocol\":1,\"version\":\"0.1.0\"}\n"
    );
}

/// The `query` entry's wire shape, pinned: every key present, `null`
/// when unknown, sizes as objects. Scripts read this.
#[test]
fn output_entries_have_a_fixed_shape() {
    use super::{Size, SurfaceEntry};
    let known = OutputEntry {
        name: Some("DP-1"),
        description: Some("A \"quoted\" monitor"),
        mode: Some(Size {
            width: 3840,
            height: 2160,
        }),
        scale: 2,
        transform: "90",
        logical: Some(Size {
            width: 1080,
            height: 1920,
        }),
        surface: SurfaceEntry {
            state: "configured",
            size: Some(Size {
                width: 1080,
                height: 1920,
            }),
        },
        shows: None,
    };
    let unknown = OutputEntry {
        name: None,
        description: None,
        mode: None,
        scale: 1,
        transform: "normal",
        logical: None,
        surface: SurfaceEntry {
            state: "waiting",
            size: None,
        },
        shows: None,
    };
    let line = reply_string(&Reply::Outputs {
        outputs: &[known, unknown],
    });
    assert_eq!(line.matches('\n').count(), 1);
    let value: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(
        value,
        serde_json::json!({"type": "outputs", "outputs": [
            {
                "name": "DP-1",
                "description": "A \"quoted\" monitor",
                "mode": {"width": 3840, "height": 2160},
                "scale": 2,
                "transform": "90",
                "logical": {"width": 1080, "height": 1920},
                "surface": {"state": "configured", "size": {"width": 1080, "height": 1920}},
                "shows": null,
            },
            {
                "name": null,
                "description": null,
                "mode": null,
                "scale": 1,
                "transform": "normal",
                "logical": null,
                "surface": {"state": "waiting", "size": null},
                "shows": null,
            },
        ]})
    );
}

#[test]
fn an_error_message_is_escaped_into_one_line() {
    let error = RequestError::Unknown("a\"b\nc".into());
    let line = reply_string(&Reply::Error { message: &error });
    assert_eq!(line.matches('\n').count(), 1);
    let value: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(value["type"], "error");
    assert_eq!(value["message"], "unknown request `a\"b\nc`");
}

#[test]
fn replies_append_to_the_buffer() {
    let mut out = b"previous\n".to_vec();
    write_reply(&mut out, &Reply::Ok);
    assert_eq!(out, b"previous\n{\"type\":\"ok\"}\n");
}

/// The derived `Deserialize` reads a struct from an array too, so
/// `[1,"kill"]` would stop the daemon. Only an object is a request.
#[test]
fn only_an_object_is_a_request() {
    for line in [
        &br#"[1,"kill"]"#[..],
        br#" [1, "query"] "#,
        b"[]",
        b"\"query\"",
        b"1",
        b"null",
        b"true",
    ] {
        assert!(
            matches!(parse(line), Err(RequestError::NotAnObject)),
            "{:?}",
            String::from_utf8_lossy(line)
        );
    }
    // Leading whitespace before the object is still fine.
    assert_eq!(
        parse(b" \t\r\n{\"protocol\":1,\"type\":\"kill\"}").unwrap(),
        Request::Kill
    );
}
