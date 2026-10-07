use super::{MAX_PATH, MAX_SECTION, Section, SectionError};
use crate::color::Color;
use crate::image::render::Look;
use crate::image::{Filter, Mode};
use crate::protocol::DEFAULT_FILL;
use crate::state::Profile;
use crate::state::format::{MAX_OUTPUTS, Pick};
use std::path::Path;

/// The cache directory the tests resolve downloads into: `record` is pure,
/// so any directory does.
fn cache() -> &'static Path {
    Path::new("/cache")
}

fn parse(json: &str) -> Result<Section, SectionError> {
    Section::parse(json.as_bytes())
}

fn refused(json: &str) -> String {
    match parse(json) {
        Ok(section) => panic!("{json} was accepted: {section:?}"),
        Err(error) => error.to_string(),
    }
}

fn color(hex: &str) -> Pick {
    Pick::Color(Color::parse(hex).unwrap())
}

#[test]
fn an_empty_section_clears_everything() {
    for json in [
        "{}",
        " { } ",
        r#"{"command":"scootbg"}"#,
        r#"{"output":{}}"#,
    ] {
        let section = parse(json).unwrap();
        assert!(section.is_empty(), "{json}");
        let record = section.record(cache());
        assert_eq!(record.all, Some(Pick::Clear), "{json}");
        assert!(record.named.is_empty(), "{json}");
    }
    // An output's own table is something to apply, even an empty one.
    assert!(!parse(r#"{"output":{"DP-1":{}}}"#).unwrap().is_empty());
    assert!(!parse(r##"{"color":"#101010"}"##).unwrap().is_empty());
}

#[test]
fn every_key_is_read() {
    let section = parse(
        r##"{"image":"/home/me/a b.jpg","mode":"fit","fill":"#102030","filter":"nearest",
            "output":{"DP-2":{"color":"#ABCDEF"},"HDMI-A-1":{},
                      "eDP-1":{"image":"/x.png","mode":"tile"}},
            "command":"/nix/store/abc-scootbg/bin/scootbg"}"##,
    )
    .unwrap();
    let record = section.record(cache());
    assert_eq!(
        record.all,
        Some(Pick::Image {
            path: "/home/me/a b.jpg".into(),
            look: Look {
                mode: Mode::Fit,
                fill: Color::parse("#102030").unwrap(),
                filter: Filter::Nearest,
            },
            fetch: None,
        })
    );
    // Sorted by name, bytewise.
    assert_eq!(
        record.named,
        vec![
            ("DP-2".to_owned(), color("#abcdef")),
            ("HDMI-A-1".to_owned(), Pick::Clear),
            (
                "eDP-1".to_owned(),
                Pick::Image {
                    path: "/x.png".into(),
                    look: Look {
                        mode: Mode::Tile,
                        fill: DEFAULT_FILL,
                        filter: Filter::default(),
                    },
                    fetch: None,
                }
            ),
        ]
    );
}

#[test]
fn a_table_stands_alone() {
    // The top level's mode is not an output's.
    let section =
        parse(r#"{"image":"/a.png","mode":"fit","output":{"DP-1":{"image":"/b.png"}}}"#).unwrap();
    let Pick::Image { look, .. } = &section.record(cache()).named[0].1 else {
        panic!("not an image");
    };
    assert_eq!(look.mode, Mode::default());
}

#[test]
fn the_canonical_encoding_sorts_keys_and_drops_command() {
    let a = parse(
        r##"{"mode":"fill","image":"/p.jpg","command":"one",
            "output":{"b":{"color":"#000001"},"a":{"mode":"fit","image":"/q.png"}}}"##,
    )
    .unwrap();
    let b = parse(
        r##"{ "output" : { "a" : { "image" : "/q.png", "mode" : "fit" },
                           "b" : { "color" : "#000001" } },
              "image" : "/p.jpg", "mode" : "fill", "command" : "two" }"##,
    )
    .unwrap();
    let canonical = r##"{"image":"/p.jpg","mode":"fill","output":{"a":{"image":"/q.png","mode":"fit"},"b":{"color":"#000001"}}}"##;
    assert_eq!(a.canonical(), canonical);
    assert_eq!(b.canonical(), canonical);
    assert_eq!(a.fingerprint(), b.fingerprint());
    // SHA-256 of that exact text, as `printf '%s' "$canonical" | sha256sum`
    // gives it.
    assert_eq!(a.fingerprint(), crate::sha256::hex(canonical.as_bytes()));
    assert_eq!(a.fingerprint().len(), 64);
    // Every key in the canonical order.
    let all =
        parse(r##"{"mode":"fit","image":"/i","filter":"bilinear","fill":"#000000","output":{}}"##)
            .unwrap();
    assert_eq!(
        all.canonical(),
        r##"{"fill":"#000000","filter":"bilinear","image":"/i","mode":"fit","output":{}}"##
    );
    assert_eq!(parse("{}").unwrap().canonical(), "{}");
    assert_eq!(
        parse(r#"{"command":"x"}"#).unwrap().fingerprint(),
        parse("{}").unwrap().fingerprint(),
        "command is not part of it"
    );
}

#[test]
fn any_change_of_a_value_changes_the_fingerprint() {
    let base = parse(r##"{"color":"#1e1e2e"}"##).unwrap().fingerprint();
    for other in [
        r##"{"color":"#1E1E2E"}"##, // as given: case counts
        r##"{"color":"#1e1e2f"}"##,
        r##"{"color":"#1e1e2e","output":{}}"##,
        r##"{"color":"#1e1e2e","output":{"DP-1":{}}}"##,
        r#"{}"#,
        r#"{"image":"/1e1e2e"}"#,
    ] {
        assert_ne!(parse(other).unwrap().fingerprint(), base, "{other}");
    }
    // Explicit defaults are a change too: the section is compared as
    // written.
    assert_ne!(
        parse(r#"{"image":"/a.png"}"#).unwrap().fingerprint(),
        parse(r#"{"image":"/a.png","mode":"fill"}"#)
            .unwrap()
            .fingerprint()
    );
}

#[test]
fn strings_round_trip_through_the_canonical_encoding() {
    // Escapes in the input come out as serde_json writes them, and the
    // canonical text parses back to the same section.
    let json = "{\"image\":\"/a\\u0020\\\"b\\\"\\\\c\\/d\\u00e9\\n.png\",\
                \"output\":{\"\\u0001x\":{\"color\":\"#000000\"}}}";
    let section = parse(json).unwrap();
    let canonical = section.canonical();
    assert!(canonical.len() <= json.len());
    let again = Section::parse(canonical.as_bytes()).unwrap();
    assert_eq!(again, section);
    assert_eq!(again.canonical(), canonical);
    let Some(Pick::Image { path, .. }) = section.record(cache()).all else {
        panic!("not an image");
    };
    assert_eq!(path, "/a \"b\"\\c/d\u{e9}\n.png");
}

#[test]
fn a_typo_is_refused_not_ignored() {
    for (json, says) in [
        (r#"{"imgae":"/a.png"}"#, "unknown field `imgae`"),
        (
            r##"{"output":{"DP-1":{"colour":"#000000"}}}"##,
            "unknown field `colour`",
        ),
        (
            r#"{"output":{"DP-1":{"command":"x"}}}"#,
            "unknown field `command`",
        ),
        (r#"{"outputs":{}}"#, "unknown field `outputs`"),
        (r#"{"image":"/a","image":"/b"}"#, "duplicate field `image`"),
        (
            r#"{"output":{"A":{"mode":"fit","mode":"fit","image":"/a"}}}"#,
            "duplicate field `mode`",
        ),
        (r#"{"output":{},"output":{}}"#, "duplicate field `output`"),
        (
            r##"{"output":{"A":{},"A":{"color":"#000000"}}}"##,
            "output \"A\" is given twice",
        ),
        (r#"{"output":{"":{}}}"#, "output name is empty"),
        (r#"{"image":null}"#, "null"),
        (r#"{"output":null}"#, "null"),
        (r#"{"output":{"A":null}}"#, "null"),
        (r#"{"command":null}"#, "null"),
        (r#"{"image":5}"#, "integer"),
        (r#"{"command":["scootbg"]}"#, "sequence"),
        (r#"{"output":[]}"#, "sequence"),
        (r#"{"output":{"A":["/a.png"]}}"#, "sequence"),
        (r#"["/a.png"]"#, "sequence"),
        (r#""{}""#, "string"),
        ("5", "integer"),
        ("null", "null"),
        ("", "EOF"),
        ("{", "EOF"),
        ("{} {}", "trailing characters"),
        (r#"{"image":"/a"} x"#, "trailing characters"),
    ] {
        let message = refused(json);
        assert!(message.contains(says), "{json}: {message}");
    }
}

#[test]
fn values_are_checked() {
    for (json, says) in [
        (
            r##"{"image":"/a","color":"#000000"}"##,
            "[wallpaper]: `image` or `color`, not both",
        ),
        (
            r##"{"output":{"DP-1":{"image":"/a","color":"#000000"}}}"##,
            "[wallpaper.output.\"DP-1\"]: `image` or `color`, not both",
        ),
        (r#"{"mode":"fill"}"#, "`mode` applies to an `image`"),
        (
            r##"{"color":"#000000","fill":"#000000"}"##,
            "`fill` applies",
        ),
        (
            r#"{"output":{"A":{"filter":"nearest"}}}"#,
            "`filter` applies",
        ),
        (r#"{"image":"a.png"}"#, "not absolute"),
        (r#"{"image":"~/a.png"}"#, "not absolute"),
        (r#"{"image":""}"#, "not absolute"),
        (r#"{"image":"/a\u0000b"}"#, "NUL"),
        (r#"{"color":"red"}"#, "`color` \"red\""),
        (r##"{"color":"#12345"}"##, "`color`"),
        (r##"{"color":"#1234567"}"##, "`color`"),
        (r##"{"image":"/a","fill":"#xyzxyz"}"##, "`fill`"),
        (r#"{"image":"/a","mode":"cover"}"#, "unknown mode \"cover\""),
        (r#"{"image":"/a","mode":"FILL"}"#, "unknown mode"),
        (
            r#"{"image":"/a","filter":"cubic"}"#,
            "unknown filter \"cubic\"",
        ),
    ] {
        let message = refused(json);
        assert!(message.contains(says), "{json}: {message}");
    }
    let longest = format!("/{}", "p".repeat(MAX_PATH - 1));
    assert!(parse(&format!(r#"{{"image":"{longest}"}}"#)).is_ok());
    let message = refused(&format!(r#"{{"image":"{longest}p"}}"#));
    assert!(message.contains("longer than Linux opens"), "{message}");
}

#[test]
fn sizes_are_bounded() {
    let mut json = String::from("{\"output\":{");
    for index in 0..MAX_OUTPUTS {
        if index > 0 {
            json.push(',');
        }
        json.push_str(&format!("\"o{index}\":{{}}"));
    }
    let fits = format!("{json}}}}}");
    assert_eq!(
        parse(&fits).unwrap().record(cache()).named.len(),
        MAX_OUTPUTS
    );
    let over = format!("{json},\"one-more\":{{}}}}}}");
    let message = refused(&over);
    assert!(
        message.contains(&format!("more than {MAX_OUTPUTS}")),
        "{message}"
    );

    // Past the byte limit, refused before parsing.
    let huge = format!(r#"{{"image":"/{}"}}"#, "a".repeat(MAX_SECTION));
    assert!(matches!(
        Section::parse(huge.as_bytes()),
        Err(SectionError::TooLarge(len)) if len == huge.len()
    ));
    // Deep nesting is serde_json's recursion limit, not a stack overflow.
    let deep = format!("{}{}", "[".repeat(10_000), "]".repeat(10_000));
    assert!(parse(&format!(r#"{{"image":{deep}}}"#)).is_err());
    let deep = format!(r#"{{"output":{{"A":{}}}}}"#, "{\"a\":".repeat(5_000));
    assert!(parse(&deep).is_err());
}

#[test]
fn non_utf8_json_is_refused() {
    assert!(Section::parse(b"{\"image\":\"/a\xff.png\"}").is_err());
    assert!(Section::parse(b"\xff").is_err());
}

#[test]
fn the_request_line_carries_the_canonical_section() {
    let section = parse(r##"{"output":{"b":{},"a":{}},"color":"#000000"}"##).unwrap();
    let profile = Profile::parse("scoot-nested").unwrap();
    let line = section.request_line(&profile);
    assert!(line.ends_with('\n'));
    assert_eq!(line.matches('\n').count(), 1);
    assert_eq!(
        line,
        "{\"protocol\":1,\"type\":\"apply-config\",\"profile\":\"scoot-nested\",\
         \"config\":{\"color\":\"#000000\",\"output\":{\"a\":{},\"b\":{}}}}\n"
    );
}

/// Arbitrary bytes never panic the parser, and whatever it accepts
/// re-encodes to text it accepts again, with the same fingerprint.
#[test]
fn arbitrary_input_never_panics() {
    let pieces: [&str; 24] = [
        "{",
        "}",
        "[",
        "]",
        ",",
        ":",
        "\"image\"",
        "\"color\"",
        "\"mode\"",
        "\"output\"",
        "\"command\"",
        "\"fill\"",
        "\"filter\"",
        "\"/a.png\"",
        "\"#010203\"",
        "\"fit\"",
        "\"nearest\"",
        "\"DP-1\"",
        "null",
        "1",
        "\"\\u0000\"",
        " ",
        "\"\\ud800\"",
        "\"x\"",
    ];
    let mut seed: u64 = 0x5eed_cafe;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let mut accepted = 0;
    for _ in 0..20_000 {
        // Braces around a random middle, so some of it is a section.
        let len = (next() % 12) as usize;
        let mut json = String::from("{");
        for _ in 0..len {
            json.push_str(pieces[(next() % pieces.len() as u64) as usize]);
        }
        json.push('}');
        if let Ok(section) = parse(&json) {
            accepted += 1;
            let again = Section::parse(section.canonical().as_bytes()).unwrap();
            assert_eq!(again.fingerprint(), section.fingerprint(), "{json}");
        }
    }
    assert!(accepted > 0, "the generator never produced a valid section");
}

#[test]
fn a_url_is_a_download_not_a_path() {
    let url = "https://example.com/a.png";
    let section = parse(&format!(
        r#"{{"image":"{url}","mode":"fit","output":{{"DP-1":{{"image":"{url}"}}}}}}"#
    ))
    .unwrap();
    let path = crate::fetch::cached_path(cache(), url)
        .to_string_lossy()
        .into_owned();
    let record = section.record(cache());
    assert_eq!(
        record.all,
        Some(Pick::Image {
            path: path.clone(),
            look: Look {
                mode: Mode::Fit,
                fill: DEFAULT_FILL,
                filter: Filter::default(),
            },
            fetch: Some(crate::fetch::Fetch {
                url: url.to_owned(),
                sha256: None,
            }),
        })
    );
    // An output's table stands alone, as a file's does.
    let Pick::Image { look, fetch, .. } = &record.named[0].1 else {
        panic!("not an image");
    };
    assert_eq!(look.mode, Mode::default());
    assert!(fetch.is_some());
    // No `sha256` written, none fingerprinted.
    assert_eq!(
        section.canonical(),
        format!(r#"{{"image":"{url}","mode":"fit","output":{{"DP-1":{{"image":"{url}"}}}}}}"#)
    );
}

#[test]
fn sha256_pins_a_download() {
    let url = "http://127.0.0.1:1/a.png";
    let sha = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";
    let section = parse(&format!(r#"{{"image":"{url}","sha256":"{sha}"}}"#)).unwrap();
    let Some(Pick::Image { fetch, .. }) = section.record(cache()).all else {
        panic!("not an image");
    };
    assert_eq!(
        fetch,
        Some(crate::fetch::Fetch {
            url: url.to_owned(),
            sha256: Some(crate::sha256::digest(b"test")),
        })
    );
    // `sha256` sorts after `mode`: byte order, like every other key.
    assert_eq!(
        section.canonical(),
        format!(r#"{{"image":"{url}","sha256":"{sha}"}}"#)
    );
    assert_ne!(
        section.fingerprint(),
        parse(&format!(r#"{{"image":"{url}"}}"#))
            .unwrap()
            .fingerprint(),
        "pinning the hash is a different section"
    );
    // Uppercase hex pins the same bytes, and still fingerprints by what
    // was written.
    let upper = parse(&format!(
        r#"{{"image":"{url}","sha256":"{}"}}"#,
        sha.to_uppercase()
    ))
    .unwrap();
    assert_ne!(upper.canonical(), section.canonical());
    assert_eq!(
        upper.record(cache()).all,
        section.record(cache()).all,
        "same bytes either case"
    );
}

#[test]
fn url_values_are_checked() {
    let sha = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";
    let bad_sha = format!(r##"{{"image":"https://example.com/a.png","sha256":"{sha}zz"}}"##);
    for (json, says) in [
        (
            r#"{"image":"file:///home/me/a.png"}"#,
            "give the path itself",
        ),
        (
            r#"{"image":"ftp://example.com/a.png"}"#,
            "URL scheme \"ftp\" is not fetched",
        ),
        (
            r#"{"image":"gopher://example.com/a"}"#,
            "URL scheme \"gopher\" is not fetched",
        ),
        (
            r#"{"image":"https://example.com/a.png","sha256":"9f86"}"#,
            "not 64 hex digits",
        ),
        (bad_sha.as_str(), "not 64 hex digits"),
        (
            r##"{"image":"/a.png","sha256":"9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"}"##,
            "not a URL",
        ),
        (
            r##"{"color":"#000000","sha256":"9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"}"##,
            "`sha256` applies to an `image`",
        ),
        (
            r#"{"image":"https://example.com/a.png","sha256":null}"#,
            "null",
        ),
        (
            r#"{"image":"https://example.com/a.png","sha256":5}"#,
            "integer",
        ),
        (
            r##"{"image":"https://example.com/a.png","color":"#000000"}"##,
            "`image` or `color`, not both",
        ),
    ] {
        let message = refused(json);
        assert!(message.contains(says), "{json}: {message}");
    }
    // Past the URL limit, refused before any fetch.
    let long = format!("https://example.com/{}", "p".repeat(crate::fetch::MAX_URL));
    let message = refused(&format!(r#"{{"image":"{long}"}}"#));
    assert!(message.contains("longer than"), "{message}");
    // A NUL byte is refused, not spawned into curl's argv.
    let message = refused("{\"image\":\"https://example.com/a\\u0000b\"}");
    assert!(message.contains("NUL"), "{message}");
}

#[test]
fn transition_keys_parse_stand_alone_and_order_canonically() {
    use crate::transition::{Easing, Kind, Spec};
    // Absent: none, everywhere.
    let plain = parse(r##"{"color":"#1e1e2e"}"##).unwrap();
    assert_eq!(plain.transition(None), Spec::none());
    assert_eq!(plain.transition(Some("DP-1")), Spec::none());
    // A transition parses, with defaults for what is absent.
    let section = parse(
        r##"{"color":"#1e1e2e","transition":"fade","duration-ms":"800",
            "output":{"DP-1":{"image":"/a.png","transition":"wipe","angle":"90"}}}"##,
    )
    .unwrap();
    assert_eq!(
        section.transition(None),
        Spec {
            kind: Kind::Fade,
            duration_ms: 800,
            easing: Easing::EaseOut,
            angle_deg: 0.0,
            pos: (0.5, 0.5),
        }
    );
    assert_eq!(
        section.transition(Some("DP-1")),
        Spec {
            kind: Kind::Wipe,
            duration_ms: 500,
            easing: Easing::EaseOut,
            angle_deg: 90.0,
            pos: (0.5, 0.5),
        }
    );
    // Each table stands alone for its own keys, but an output with no
    // table follows the top level, as for the wallpaper itself.
    assert_eq!(
        section.transition(Some("DP-2")),
        Spec {
            kind: Kind::Fade,
            duration_ms: 800,
            easing: Easing::EaseOut,
            angle_deg: 0.0,
            pos: (0.5, 0.5),
        }
    );
    // Canonical order is byte order, with the new keys among the old.
    assert_eq!(
        section.canonical(),
        r##"{"color":"#1e1e2e","duration-ms":"800","transition":"fade","output":{"DP-1":{"angle":"90","image":"/a.png","transition":"wipe"}}}"##
    );
    // The fingerprint moves with a transition change, as with any value.
    assert_ne!(
        section.fingerprint(),
        parse(r##"{"color":"#1e1e2e","transition":"fade","duration-ms":"801"}"##)
            .unwrap()
            .fingerprint()
    );
    // An explicit `none` ignores the rest.
    let none = parse(r##"{"color":"#000000","transition":"none","angle":"90"}"##).unwrap();
    assert_eq!(none.transition(None), Spec::none());
    assert_eq!(
        none.canonical(),
        r##"{"angle":"90","color":"#000000","transition":"none"}"##
    );
}

#[test]
fn transition_keys_are_refused_strictly() {
    for (json, what) in [
        (
            r##"{"color":"#000000","transition":"dissolve"}"##,
            "unknown transition",
        ),
        (
            r##"{"color":"#000000","transition":"fade","easing":"bounce"}"##,
            "unknown easing",
        ),
        (
            r##"{"color":"#000000","transition":"fade","duration-ms":"-1"}"##,
            "bad duration-ms",
        ),
        (
            r##"{"color":"#000000","transition":"wipe","angle":"NaN"}"##,
            "bad angle",
        ),
        (
            r##"{"color":"#000000","transition":"grow","position":"2,0"}"##,
            "bad position",
        ),
        (
            r##"{"color":"#000000","duration-ms":"800"}"##,
            "applies to a transition",
        ),
        (
            r##"{"output":{"DP-1":{"easing":"linear"}}}"##,
            "applies to a transition",
        ),
    ] {
        let error = parse(json).unwrap_err();
        assert!(error.to_string().contains(what), "{json}: {error}");
    }
}

#[test]
fn a_named_table_without_a_transition_key_animates_nothing() {
    use crate::transition::Spec;
    let section = parse(
        r##"{"color":"#1e1e2e","transition":"fade",
            "output":{"DP-1":{"color":"#000000"}}}"##,
    )
    .unwrap();
    assert_eq!(section.transition(Some("DP-1")), Spec::none());
}
