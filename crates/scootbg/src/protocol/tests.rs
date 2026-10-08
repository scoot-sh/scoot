use std::sync::Arc;

use super::{
    DEFAULT_FILL, ImageRequest, OutputEntry, PROTOCOL_VERSION, Reply, Request, RequestError,
    RotationInfo, Show, SlideshowRequest, Source, WorkspaceEntry, parse, write_reply,
};
use crate::color::Color;
use crate::image::render::Look;
use crate::image::{Filter, Mode};
use crate::wallpaper::{Image, Wallpaper};

fn reply_string(reply: &Reply<'_>) -> String {
    let mut out = Vec::new();
    write_reply(&mut out, reply);
    String::from_utf8(out).unwrap()
}

fn red() -> Color {
    Color::parse("#c03020").unwrap()
}

#[test]
fn each_request_parses() {
    for request in [
        Request::Query,
        Request::Kill,
        Request::Version,
        Request::Set {
            show: Show::Color(red()),
            output: None,
            transition: crate::transition::Spec::none(),
        },
        Request::Set {
            show: Show::Color(red()),
            output: Some("DP-1".into()),
            transition: crate::transition::Spec::none(),
        },
        Request::Set {
            show: Show::Image(ImageRequest {
                source: Source::Path("/home/me/Pictures/a b \"c\".jpg".into()),
                mode: Mode::Tile,
                fill: red(),
                filter: Filter::Nearest,
            }),
            output: Some("DP-1".into()),
            transition: crate::transition::Spec::none(),
        },
        Request::Clear { output: None },
        Request::Clear {
            output: Some("HEADLESS-2".into()),
        },
        Request::SetWorkspace {
            show: Show::Color(red()),
            output: None,
            workspace: "2".into(),
            transition: crate::transition::Spec::none(),
        },
        Request::SetWorkspace {
            show: Show::Image(ImageRequest {
                source: Source::Path("/home/me/Pictures/a b \"c\".jpg".into()),
                mode: Mode::Tile,
                fill: red(),
                filter: Filter::Nearest,
            }),
            output: Some("DP-1".into()),
            workspace: "2 DP-1".into(),
            transition: crate::transition::Spec::none(),
        },
        Request::ClearWorkspace {
            output: None,
            workspace: "2".into(),
        },
        Request::ClearWorkspace {
            output: Some("HEADLESS-2".into()),
            workspace: "web".into(),
        },
    ] {
        let line = request.line();
        assert!(line.ends_with('\n'));
        assert_eq!(line.matches('\n').count(), 1);
        assert_eq!(parse(line.trim_end().as_bytes()).unwrap(), request);
    }
}

#[test]
fn set_and_clear_lines_are_what_the_docs_say() {
    let set = Request::Set {
        show: Show::Color(Color::parse("#1E1E2E").unwrap()),
        output: Some("DP-1".into()),
        transition: crate::transition::Spec::none(),
    };
    assert_eq!(
        set.line(),
        "{\"protocol\":1,\"type\":\"set\",\"color\":\"#1e1e2e\",\"output\":\"DP-1\"}\n"
    );
    assert_eq!(
        Request::Clear { output: None }.line(),
        "{\"protocol\":1,\"type\":\"clear\"}\n"
    );
    assert_eq!(
        Request::Query.line(),
        "{\"protocol\":1,\"type\":\"query\"}\n"
    );
}

/// Output names come from the compositor and can hold anything: quotes,
/// backslashes, control characters, non-ASCII. They round-trip, one line.
#[test]
fn any_output_name_round_trips() {
    for name in [
        "a\"b",
        "back\\slash",
        "new\nline",
        "\u{1b}[31m",
        "Écran",
        "",
    ] {
        let request = Request::Clear {
            output: Some(name.into()),
        };
        let line = request.line();
        assert_eq!(line.matches('\n').count(), 1, "{name:?}");
        assert_eq!(parse(line.trim_end().as_bytes()).unwrap(), request);
    }
}

#[test]
fn a_set_needs_a_valid_color() {
    assert!(matches!(
        parse(br##"{"protocol":1,"type":"set"}"##),
        Err(RequestError::NoTarget)
    ));
    assert!(matches!(
        parse(br##"{"protocol":1,"type":"set","color":"#000000","image":"/x.png"}"##),
        Err(RequestError::Both)
    ));
    for bad in ["#fff", "c03020", "#c03020 ", "#c03020ff", ""] {
        let line = format!("{{\"protocol\":1,\"type\":\"set\",\"color\":{bad:?}}}");
        match parse(line.as_bytes()) {
            Err(RequestError::BadColor { text, error }) => {
                assert_eq!(text, bad);
                let message = RequestError::BadColor { text, error }.to_string();
                assert!(message.contains("#rrggbb"), "{message}");
            }
            other => panic!("{bad:?}: {other:?}"),
        }
    }
    // Uppercase is fine; the wire form is lowercase.
    assert_eq!(
        parse(br##"{"protocol":1,"type":"set","color":"#C03020"}"##).unwrap(),
        Request::Set {
            show: Show::Color(red()),
            output: None,
            transition: crate::transition::Spec::none(),
        }
    );
    // Image options do not go with a color.
    for field in ["mode", "fill", "filter"] {
        let line = format!(
            "{{\"protocol\":1,\"type\":\"set\",\"color\":\"#000000\",\"{field}\":\"fit\"}}"
        );
        match parse(line.as_bytes()) {
            Err(RequestError::ImageOnly(named)) => assert_eq!(named, field),
            other => panic!("{field}: {other:?}"),
        }
    }
}

#[test]
fn an_image_set_has_defaults_and_checks_every_field() {
    assert_eq!(
        parse(br##"{"protocol":1,"type":"set","image":"/p/a.png"}"##).unwrap(),
        Request::Set {
            show: Show::Image(ImageRequest {
                source: Source::Path("/p/a.png".into()),
                mode: Mode::Fill,
                fill: DEFAULT_FILL,
                filter: Filter::Lanczos3,
            }),
            output: None,
            transition: crate::transition::Spec::none(),
        }
    );
    let full = br##"{"protocol":1,"type":"set","image":"/p/a.png","mode":"center","fill":"#ABCDEF","filter":"bilinear","output":"X"}"##;
    assert_eq!(
        parse(full).unwrap(),
        Request::Set {
            show: Show::Image(ImageRequest {
                source: Source::Path("/p/a.png".into()),
                mode: Mode::Center,
                fill: Color::parse("#abcdef").unwrap(),
                filter: Filter::Bilinear,
            }),
            output: Some("X".into()),
            transition: crate::transition::Spec::none(),
        }
    );
    // The documented line.
    let line = Request::Set {
        show: Show::Image(ImageRequest {
            source: Source::Path("/abs/a.jpg".into()),
            mode: Mode::Fit,
            fill: Color::parse("#101014").unwrap(),
            filter: Filter::Lanczos3,
        }),
        output: None,
        transition: crate::transition::Spec::none(),
    }
    .line();
    assert_eq!(
        line,
        "{\"protocol\":1,\"type\":\"set\",\"image\":\"/abs/a.jpg\",\"mode\":\"fit\",\
         \"fill\":\"#101014\",\"filter\":\"lanczos3\"}\n"
    );
    for (line, check) in [
        (
            &br##"{"protocol":1,"type":"set","image":"a.png"}"##[..],
            "not absolute",
        ),
        (
            br##"{"protocol":1,"type":"set","image":""}"##,
            "not absolute",
        ),
        (
            br##"{"protocol":1,"type":"set","image":"~/a.png"}"##,
            "not absolute",
        ),
        (
            br##"{"protocol":1,"type":"set","image":"/a","mode":"cover"}"##,
            "unknown mode",
        ),
        (
            br##"{"protocol":1,"type":"set","image":"/a","filter":"lanczos"}"##,
            "unknown filter",
        ),
        (
            br##"{"protocol":1,"type":"set","image":"/a","fill":"black"}"##,
            "bad fill",
        ),
        (
            br##"{"protocol":1,"type":"set","image":"/a","fill":"#fff"}"##,
            "bad fill",
        ),
    ] {
        let error = parse(line).unwrap_err().to_string();
        assert!(error.contains(check), "{error}");
    }
}

#[test]
fn field_order_whitespace_and_extra_fields_do_not_matter() {
    let line = br##" { "future": [1, 2], "type" : "query", "protocol" : 1 } "##;
    assert_eq!(parse(line).unwrap(), Request::Query);
    // A trailing carriage return is whitespace to JSON.
    assert_eq!(
        parse(b"{\"protocol\":1,\"type\":\"kill\"}\r").unwrap(),
        Request::Kill
    );
}

#[test]
fn an_escaped_type_still_parses() {
    let line = br##"{"protocol":1,"type":"query"}"##;
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
        parse(br##"{"type":"query"}"##),
        Err(RequestError::NoProtocol)
    ));
    assert!(matches!(
        parse(br##"{"protocol":2,"type":"query"}"##),
        Err(RequestError::WrongProtocol(2))
    ));
    assert!(matches!(
        parse(br##"{"protocol":0,"type":"query"}"##),
        Err(RequestError::WrongProtocol(0))
    ));
    let message = RequestError::WrongProtocol(9).to_string();
    assert!(message.contains('9') && message.contains(&PROTOCOL_VERSION.to_string()));
}

#[test]
fn a_missing_or_unknown_type_is_refused() {
    assert!(matches!(
        parse(br##"{"protocol":1}"##),
        Err(RequestError::NoType)
    ));
    match parse(br##"{"protocol":1,"type":"apply-configs"}"##) {
        Err(RequestError::Unknown(name)) => assert_eq!(name, "apply-configs"),
        other => panic!("expected Unknown, got {other:?}"),
    }
}

#[test]
fn replies_are_one_tagged_line() {
    assert_eq!(reply_string(&Reply::Ok), "{\"type\":\"ok\"}\n");
    assert_eq!(
        reply_string(&Reply::Outputs {
            outputs: &[] as &[OutputEntry<'_>; 0],
            workspaces: &[] as &[WorkspaceEntry<'_>; 0],
            saving: true,
            profile: "default",
            rotation: None,
        }),
        "{\"type\":\"outputs\",\"outputs\":[],\"workspaces\":[],\"saving\":true,\"profile\":\"default\"}\n"
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
    let color = Wallpaper::Color(Color::parse("#C03020").unwrap());
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
            scale: Some(crate::density::Scale::Fractional(180)),
            pixels: Some(Size {
                width: 1620,
                height: 2880,
            }),
        },
        draw_failed: false,
        draw_error: None,
        shows: Some(super::Shows(&color)),
        workspace: Some("2"),
        transition: Some("fade"),
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
            scale: None,
            pixels: None,
        },
        draw_failed: true,
        draw_error: Some("a \"reason\"\nover two lines"),
        shows: None,
        workspace: None,
        transition: None,
    };
    let mapping = WorkspaceEntry {
        output: Some("DP-1"),
        workspace: "2",
        shows: super::Shows(&color),
    };
    let line = reply_string(&Reply::Outputs {
        outputs: &[known, unknown],
        workspaces: &[mapping],
        saving: false,
        profile: "scoot",
        rotation: None,
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
                "surface": {
                    "state": "configured",
                    "size": {"width": 1080, "height": 1920},
                    "scale": 1.5,
                    "pixels": {"width": 1620, "height": 2880},
                },
                "draw_failed": false,
                "draw_error": null,
                "shows": {"color": "#c03020"},
                "workspace": "2",
                "transition": "fade",
            },
            {
                "name": null,
                "description": null,
                "mode": null,
                "scale": 1,
                "transform": "normal",
                "logical": null,
                "surface": {"state": "waiting", "size": null, "scale": null, "pixels": null},
                "draw_failed": true,
                "draw_error": "a \"reason\"\nover two lines",
                "shows": null,
                "workspace": null,
                "transition": null,
            },
        ],
        "workspaces": [
            {"output": "DP-1", "workspace": "2", "shows": {"color": "#c03020"}},
        ], "saving": false, "profile": "scoot"})
    );
}

#[test]
fn a_workspace_slideshow_is_refused() {
    assert!(matches!(
        parse(
            br##"{"protocol":1,"type":"set-workspace","workspace":"2","directory":"/pics","every":"1800s"}"##
        ),
        Err(RequestError::SlideshowWithWorkspace)
    ));
}

#[test]
fn workspace_requests_need_a_good_name() {
    // Missing.
    assert!(matches!(
        parse(br##"{"protocol":1,"type":"set-workspace","color":"#c03020"}"##),
        Err(RequestError::NoWorkspace)
    ));
    assert!(matches!(
        parse(br##"{"protocol":1,"type":"clear-workspace"}"##),
        Err(RequestError::NoWorkspace)
    ));
    // Empty.
    assert!(matches!(
        parse(br##"{"protocol":1,"type":"clear-workspace","workspace":""}"##),
        Err(RequestError::EmptyWorkspace)
    ));
    // Past the bound.
    let long = "w".repeat(crate::choices::MAX_WORKSPACE_NAME + 1);
    assert!(matches!(
        parse(
            format!(
                r##"{{"protocol":1,"type":"set-workspace","workspace":"{long}","color":"#c03020"}}"##
            )
            .as_bytes()
        ),
        Err(RequestError::WorkspaceTooLong)
    ));
    // A NUL byte, as a JSON escape (one cannot appear raw).
    assert!(matches!(
        parse(
            br##"{"protocol":1,"type":"set-workspace","workspace":"a\u0000b","color":"#c03020"}"##
                .as_ref()
        ),
        Err(RequestError::WorkspaceNul)
    ));
    // A set-workspace still needs something to show, and parses its
    // transition like a set does.
    assert!(matches!(
        parse(br##"{"protocol":1,"type":"set-workspace","workspace":"2"}"##),
        Err(RequestError::NoTarget)
    ));
    assert!(matches!(
        parse(br##"{"protocol":1,"type":"set-workspace","workspace":"2","color":"#c03020","duration-ms":"500"}"##),
        Err(RequestError::BadTransition(_))
    ));
    // A clear-workspace with transition fields is refused, like a clear.
    assert!(matches!(
        parse(br##"{"protocol":1,"type":"clear-workspace","workspace":"2","transition":"fade"}"##),
        Err(RequestError::TransitionWithClear)
    ));
}

#[test]
fn a_workspace_line_carries_what_a_set_does_plus_the_name() {
    let request = Request::SetWorkspace {
        show: Show::Color(red()),
        output: Some("DP-1".into()),
        workspace: "2".into(),
        transition: crate::transition::Spec::none(),
    };
    let line = request.line();
    let value: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(value["type"], "set-workspace");
    assert_eq!(value["workspace"], "2");
    assert_eq!(value["output"], "DP-1");
    assert_eq!(value["color"], "#c03020");
    assert!(
        value.get("transition").is_none(),
        "no transition: no fields"
    );
    assert_eq!(parse(line.trim_end().as_bytes()).unwrap(), request);
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
        &br##"[1,"kill"]"##[..],
        br##" [1, "query"] "##,
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

#[test]
fn an_image_shows_its_path_mode_fill_and_filter() {
    let image = Wallpaper::Image(Arc::new(Image {
        path: "/home/me/a \"b\".png".into(),
        look: Look {
            mode: Mode::Fit,
            fill: Color::parse("#101014").unwrap(),
            filter: Filter::CatmullRom,
        },
        serial: 7,
        fetch: None,
    }));
    let value = serde_json::to_value(super::Shows(&image)).unwrap();
    assert_eq!(
        value,
        serde_json::json!({
            "image": "/home/me/a \"b\".png",
            "mode": "fit",
            "fill": "#101014",
            "filter": "catmull-rom",
        })
    );
}

fn section(json: &str) -> crate::section::Section {
    crate::section::Section::parse(json.as_bytes()).unwrap()
}

#[test]
fn apply_config_round_trips() {
    let request = Request::ApplyConfig {
        profile: crate::state::Profile::parse("scoot").unwrap(),
        section: Box::new(section(
            r##"{"image":"/a b.png","output":{"DP-2":{"color":"#101014"}}}"##,
        )),
    };
    let line = request.line();
    assert!(line.ends_with('\n'));
    assert_eq!(parse(line.trim_end().as_bytes()).unwrap(), request);
    assert_eq!(request.name(), "apply-config");
    // Keys in any order, other fields ignored, as for every request.
    let reordered = br##"{"config":{"output":{"DP-2":{"color":"#101014"}},"image":"/a b.png"},
        "extra":[1,2],"profile":"scoot","type":"apply-config","protocol":1}"##;
    assert_eq!(parse(reordered).unwrap(), request);
}

#[test]
fn apply_config_is_checked() {
    for (line, says) in [
        (
            r#"{"protocol":1,"type":"apply-config","config":{}}"#,
            "needs a `profile`",
        ),
        (
            r#"{"protocol":1,"type":"apply-config","profile":"scoot"}"#,
            "needs a `config`",
        ),
        (
            r#"{"protocol":1,"type":"apply-config","profile":"../x","config":{}}"#,
            "profile name",
        ),
        (
            r#"{"protocol":1,"type":"apply-config","profile":7,"config":{}}"#,
            "bad apply-config request",
        ),
        (
            r#"{"protocol":1,"type":"apply-config","profile":"scoot","config":{"imgae":"/a"}}"#,
            "unknown field `imgae`",
        ),
        (
            r#"{"protocol":1,"type":"apply-config","profile":"scoot","config":{"image":"a"}}"#,
            "not absolute",
        ),
        (
            r#"{"protocol":1,"type":"apply-config","profile":"scoot","config":{"image":"/a","image":"/b"}}"#,
            "duplicate field",
        ),
        (
            r#"{"protocol":1,"type":"apply-config","profile":"scoot","config":[]}"#,
            "bad apply-config request",
        ),
        (
            r#"{"protocol":2,"type":"apply-config","profile":"scoot","config":{}}"#,
            "protocol mismatch",
        ),
    ] {
        match parse(line.as_bytes()) {
            Ok(request) => panic!("{line} parsed as {request:?}"),
            Err(error) => {
                let message = error.to_string();
                assert!(message.contains(says), "{line}: {message}");
            }
        }
    }
}

/// `config` and `profile` on any other request stay ignored unknown
/// fields: only `apply-config` reads them.
#[test]
fn config_on_another_request_is_ignored() {
    assert_eq!(
        parse(br##"{"protocol":1,"type":"query","config":{"imgae":1},"profile":"../"}"##).unwrap(),
        Request::Query
    );
}

#[test]
fn a_url_set_parses_and_round_trips() {
    let sha = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";
    let line = format!(
        "{{\"protocol\":1,\"type\":\"set\",\"image\":\"https://example.com/a.png\",\
          \"sha256\":\"{sha}\",\"mode\":\"fit\"}}"
    );
    let request = parse(line.as_bytes()).unwrap();
    let Request::Set { show, output, .. } = &request else {
        panic!("not a set");
    };
    assert_eq!(output, &None);
    let Show::Image(image) = show else {
        panic!("not an image");
    };
    assert_eq!(
        image.source,
        Source::Url {
            url: "https://example.com/a.png".into(),
            sha256: Some(crate::sha256::digest(b"test")),
        }
    );
    assert_eq!(image.mode, Mode::Fit);
    // The wire carries the URL and the hash in hex.
    assert_eq!(
        request.line(),
        format!(
            "{{\"protocol\":1,\"type\":\"set\",\"image\":\"https://example.com/a.png\",\
              \"sha256\":\"{sha}\",\"mode\":\"fit\",\"fill\":\"#000000\",\"filter\":\"lanczos3\"}}\n"
        )
    );
    // Without a pin, no `sha256` goes out.
    let plain =
        parse(b"{\"protocol\":1,\"type\":\"set\",\"image\":\"http://127.0.0.1:1/a.png\"}").unwrap();
    assert!(
        !plain.line().contains("sha256"),
        "unpinned: {}",
        plain.line()
    );
}

#[test]
fn url_and_hash_misuse_is_refused() {
    // A hash that is not one.
    let bad = parse(
        b"{\"protocol\":1,\"type\":\"set\",\"image\":\"https://example.com/a.png\",\"sha256\":\"zz\"}",
    );
    assert!(matches!(bad, Err(RequestError::BadSha(_))), "{bad:?}");
    // A hash with a file, or with a color.
    assert!(matches!(
        parse(
            b"{\"protocol\":1,\"type\":\"set\",\"image\":\"/a.png\",\"sha256\":\"9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08\"}"
        ),
        Err(RequestError::ShaWithFile)
    ));
    assert!(matches!(
        parse(
            b"{\"protocol\":1,\"type\":\"set\",\"color\":\"#000000\",\"sha256\":\"9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08\"}"
        ),
        Err(RequestError::ImageOnly("sha256"))
    ));
    // A NUL byte never reaches curl's argv (as the JSON escape: a raw
    // NUL is not JSON at all).
    let nul = b"{\"protocol\":1,\"type\":\"set\",\"image\":\"https://example.com/a\\u0000b\"}";
    assert!(
        matches!(parse(nul), Err(RequestError::UrlNul)),
        "{:?}",
        parse(nul)
    );
}

#[test]
fn a_set_carries_a_transition() {
    use crate::transition::{Easing, Kind, Spec};
    // Absent: none.
    assert_eq!(
        parse(br##"{"protocol":1,"type":"set","color":"#000000"}"##).unwrap(),
        Request::Set {
            show: Show::Color(Color::parse("#000000").unwrap()),
            output: None,
            transition: Spec::none(),
        }
    );
    // An explicit `none` ignores the rest too.
    assert_eq!(
        parse(
            br##"{"protocol":1,"type":"set","color":"#000000","transition":"none","angle":"90"}"##
        )
        .unwrap(),
        Request::Set {
            show: Show::Color(Color::parse("#000000").unwrap()),
            output: None,
            transition: Spec::none(),
        }
    );
    // Full: everything parses, with defaults for what is absent.
    assert_eq!(
        parse(
            br##"{"protocol":1,"type":"set","color":"#000000","transition":"wipe","angle":"90"}"##
        )
        .unwrap(),
        Request::Set {
            show: Show::Color(Color::parse("#000000").unwrap()),
            output: None,
            transition: Spec {
                kind: Kind::Wipe,
                duration_ms: 500,
                easing: Easing::EaseOut,
                angle_deg: 90.0,
                pos: (0.5, 0.5),
            },
        }
    );
    // The line carries a transition, and it round-trips.
    let request = Request::Set {
        show: Show::Color(Color::parse("#101014").unwrap()),
        output: Some("DP-1".into()),
        transition: Spec {
            kind: Kind::Grow,
            duration_ms: 800,
            easing: Easing::Linear,
            angle_deg: 0.0,
            pos: (0.0, 0.0),
        },
    };
    let line = request.line();
    assert_eq!(
        line,
        "{\"protocol\":1,\"type\":\"set\",\"color\":\"#101014\",\"output\":\"DP-1\",\
         \"transition\":\"grow\",\"duration-ms\":\"800\",\"easing\":\"linear\",\
         \"angle\":\"0\",\"position\":\"0,0\"}\n"
    );
    assert_eq!(parse(line.trim_end().as_bytes()).unwrap(), request);
    // A `set` with no transition carries no transition fields.
    let plain = Request::Set {
        show: Show::Color(Color::parse("#101014").unwrap()),
        output: None,
        transition: Spec::none(),
    };
    assert_eq!(
        plain.line(),
        "{\"protocol\":1,\"type\":\"set\",\"color\":\"#101014\"}\n"
    );
    // Refusals say what each takes.
    for (line, what) in [
        (
            &br##"{"protocol":1,"type":"set","color":"#000000","transition":"dissolve"}"##[..],
            "unknown transition",
        ),
        (
            br##"{"protocol":1,"type":"set","color":"#000000","transition":"fade","duration-ms":"-1"}"##,
            "bad duration",
        ),
        (
            br##"{"protocol":1,"type":"set","color":"#000000","transition":"fade","easing":"bounce"}"##,
            "unknown easing",
        ),
        (
            br##"{"protocol":1,"type":"set","color":"#000000","transition":"wipe","angle":"NaN"}"##,
            "bad angle",
        ),
        (
            br##"{"protocol":1,"type":"set","color":"#000000","transition":"grow","position":"2,0"}"##,
            "bad position",
        ),
        (
            br##"{"protocol":1,"type":"set","color":"#000000","duration-ms":"5"}"##,
            "applies to a transition",
        ),
    ] {
        match parse(line) {
            Err(RequestError::BadTransition(error)) => {
                assert!(error.to_string().contains(what), "{error}")
            }
            other => panic!("{line:?}: {other:?}"),
        }
    }
    // A `clear` with transition fields is refused, saying why.
    match parse(br##"{"protocol":1,"type":"clear","transition":"fade"}"##) {
        Err(RequestError::TransitionWithClear) => {}
        other => panic!("{other:?}"),
    }
    let message = RequestError::TransitionWithClear.to_string();
    assert!(message.contains("nothing to blend"), "{message}");
}

#[test]
fn a_slideshow_round_trips() {
    for request in [
        Request::Set {
            show: Show::Slideshow(SlideshowRequest {
                dir: "/home/me/Pictures".into(),
                every_secs: 1800,
                shuffle: false,
                mode: Mode::Fill,
                fill: DEFAULT_FILL,
                filter: Filter::Lanczos3,
            }),
            output: None,
            transition: crate::transition::Spec::none(),
        },
        Request::Set {
            show: Show::Slideshow(SlideshowRequest {
                dir: "/home/me/Pictures".into(),
                every_secs: 3600,
                shuffle: true,
                mode: Mode::Tile,
                fill: red(),
                filter: Filter::Nearest,
            }),
            output: Some("DP-1".into()),
            transition: crate::transition::Spec::none(),
        },
    ] {
        let line = request.line();
        assert!(line.ends_with('\n'));
        assert_eq!(line.matches('\n').count(), 1);
        assert_eq!(parse(line.trim_end().as_bytes()).unwrap(), request);
    }
}

#[test]
fn a_slideshow_line_is_what_the_docs_say() {
    let set = Request::Set {
        show: Show::Slideshow(SlideshowRequest {
            dir: "/pics".into(),
            every_secs: 1800,
            shuffle: false,
            mode: Mode::Fill,
            fill: DEFAULT_FILL,
            filter: Filter::Lanczos3,
        }),
        output: None,
        transition: crate::transition::Spec::none(),
    };
    assert_eq!(
        set.line(),
        "{\"protocol\":1,\"type\":\"set\",\"directory\":\"/pics\",\"every\":\"1800s\",\"mode\":\"fill\",\"fill\":\"#000000\",\"filter\":\"lanczos3\"}\n"
    );
}

#[test]
fn a_slideshow_needs_a_directory_and_its_pace() {
    // No target at all.
    assert!(matches!(
        parse(br#"{"protocol":1,"type":"set"}"#),
        Err(RequestError::NoTarget)
    ));
    // A directory without its pace, and a pace without its directory.
    assert!(matches!(
        parse(br#"{"protocol":1,"type":"set","directory":"/pics"}"#),
        Err(RequestError::EveryMissing)
    ));
    assert!(matches!(
        parse(br#"{"protocol":1,"type":"set","every":"1800s"}"#),
        Err(RequestError::EveryWithoutDirectory)
    ));
    assert!(matches!(
        parse(br#"{"protocol":1,"type":"set","shuffle":true}"#),
        Err(RequestError::ShuffleWithoutDirectory)
    ));
    // Two targets, and a hash pinning what is not a download.
    assert!(matches!(
        parse(br##"{"protocol":1,"type":"set","color":"#000000","directory":"/pics","every":"1800s"}"##),
        Err(RequestError::Both)
    ));
    assert!(matches!(
        parse(
            br#"{"protocol":1,"type":"set","directory":"/pics","every":"1800s","sha256":"9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"}"#
        ),
        Err(RequestError::ShaWithDirectory)
    ));
    // A relative directory, and a pace that is not one.
    assert!(matches!(
        parse(br#"{"protocol":1,"type":"set","directory":"pics","every":"1800s"}"#),
        Err(RequestError::RelativePath(_))
    ));
    assert!(matches!(
        parse(br#"{"protocol":1,"type":"set","directory":"/pics","every":"30s"}"#),
        Err(RequestError::BadEvery(_))
    ));
    // Unknown fields are ignored: an old daemon reads this as a `set`
    // with no target, and refuses loudly instead of misreading.
    let parsed =
        parse(br#"{"protocol":1,"type":"set","directory":"/pics","every":"1800s"}"#).unwrap();
    let Request::Set { show, .. } = parsed else {
        panic!("a slideshow is a set");
    };
    assert!(matches!(show, Show::Slideshow(_)));
}

#[test]
fn a_query_without_a_slideshow_is_byte_for_byte_what_it_was() {
    assert_eq!(
        reply_string(&Reply::Outputs {
            outputs: &[] as &[OutputEntry<'_>; 0],
            workspaces: &[] as &[WorkspaceEntry<'_>; 0],
            saving: true,
            profile: "default",
            rotation: None,
        }),
        "{\"type\":\"outputs\",\"outputs\":[],\"workspaces\":[],\"saving\":true,\"profile\":\"default\"}\n"
    );
    let line = reply_string(&Reply::Outputs {
        outputs: &[] as &[OutputEntry<'_>; 0],
        workspaces: &[] as &[WorkspaceEntry<'_>; 0],
        saving: true,
        profile: "default",
        rotation: Some(RotationInfo {
            directory: "/pics",
            every_secs: 1800,
            shuffle: true,
            files: 12,
        }),
    });
    let value: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(
        value["rotation"],
        serde_json::json!({"directory": "/pics", "every_secs": 1800, "shuffle": true, "files": 12})
    );
}
