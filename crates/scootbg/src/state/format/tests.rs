use std::sync::Arc;

use super::{
    FieldError, MAX_BYTES, MAX_FINGERPRINT, MAX_OUTPUTS, Parsed, Pick, Record, decode, encode,
    escape, unescape,
};
use crate::choices::Choice;
use crate::color::Color;
use crate::image::render::Look;
use crate::image::{Filter, Mode};
use crate::wallpaper::{Image, Wallpaper};

const LOOK: Look = Look {
    mode: Mode::Fit,
    fill: Color {
        r: 0x10,
        g: 0x10,
        b: 0x14,
    },
    filter: Filter::CatmullRom,
};

fn image(path: &str) -> Choice {
    Some(Wallpaper::Image(Arc::new(Image {
        path: path.to_owned(),
        look: LOOK,
        serial: 1,
        fetch: None,
    })))
}

fn color(r: u8, g: u8, b: u8) -> Choice {
    Some(Wallpaper::Color(Color { r, g, b }))
}

fn text(fingerprint: Option<&str>, all: Option<&Choice>, named: &[(&str, Choice)]) -> String {
    let mut out = String::new();
    encode(
        &mut out,
        "default",
        fingerprint,
        all,
        &named
            .iter()
            .map(|(name, choice)| (*name, choice))
            .collect::<Vec<_>>(),
    );
    out
}

fn clean(bytes: &[u8]) -> Record {
    let parsed = decode(bytes);
    assert!(
        parsed.warnings.is_empty(),
        "unexpected warnings: {:?}",
        parsed.warnings
    );
    assert!(!parsed.newer);
    parsed.record
}

fn pick_image(path: &str) -> Pick {
    Pick::Image {
        path: path.to_owned(),
        look: LOOK,
        fetch: None,
    }
}

/// The file as written, byte for byte: a reader can rely on this shape.
#[test]
fn the_file_reads_as_documented() {
    let written = text(
        Some("9c1e"),
        Some(&color(0x1e, 0x1e, 0x2e)),
        &[
            ("DP-1", image("/home/me/My Pictures/hills.jpg")),
            ("HDMI-A-1", None),
        ],
    );
    assert_eq!(
        written,
        "scootbg-state 2\n\
         profile default\n\
         fingerprint 9c1e\n\
         all color #1e1e2e\n\
         output DP-1 image /home/me/My%20Pictures/hills.jpg fit #101014 catmull-rom\n\
         output HDMI-A-1 clear\n"
    );
}

#[test]
fn everything_round_trips() {
    let named = [
        ("DP-1", image("/a.jpg")),
        ("HDMI-A-1", None),
        ("eDP-1", color(1, 2, 3)),
    ];
    let record = clean(text(Some("abc"), Some(&image("/all.png")), &named).as_bytes());
    assert_eq!(
        record,
        Record {
            profile: Some("default".to_owned()),
            fingerprint: Some("abc".to_owned()),
            all: Some(pick_image("/all.png")),
            named: vec![
                ("DP-1".to_owned(), pick_image("/a.jpg")),
                ("HDMI-A-1".to_owned(), Pick::Clear),
                ("eDP-1".to_owned(), Pick::Color(Color { r: 1, g: 2, b: 3 })),
            ],
        }
    );
    // Nothing chosen: only the header and profile, and nothing read back.
    let empty = text(None, None, &[]);
    assert_eq!(empty, "scootbg-state 2\nprofile default\n");
    let record = clean(empty.as_bytes());
    assert_eq!((record.all, record.named.len()), (None, 0));
    // A clear of every output is a choice, and kept.
    let record = clean(text(None, Some(&None), &[]).as_bytes());
    assert_eq!(record.all, Some(Pick::Clear));
}

/// Paths (and names) round-trip exactly, whatever they hold.
#[test]
fn hostile_paths_round_trip() {
    for path in [
        "/with space/and  two.jpg",
        "/#hash/#start.png",
        "/100%/%41%zz.png",
        "/new\nline/and\r\ncrlf.jpg",
        "/tab\there.webp",
        "/ctrl\u{1}\u{1f}\u{7f}.png",
        "/üñíçødé/日本語.jpg",
        "/trailing space ",
        "/-",
        "/image",
        "/ ",
    ] {
        let named = [(path, image(path))];
        let record = clean(text(Some(path), Some(&image(path)), &named).as_bytes());
        assert_eq!(record.all, Some(pick_image(path)), "{path:?}");
        assert_eq!(record.fingerprint.as_deref(), Some(path));
        assert_eq!(record.named, vec![(path.to_owned(), pick_image(path))]);
    }
}

#[test]
fn escaping_leaves_no_separator_and_undoes_exactly() {
    let mut out = String::new();
    escape(&mut out, "a b%c\nd\te#f\u{7f}ü");
    assert_eq!(out, "a%20b%25c%0Ad%09e#f%7Fü");
    assert!(!out.contains([' ', '\n', '\t', '\r']));
    assert_eq!(unescape(out.as_bytes()).unwrap(), "a b%c\nd\te#f\u{7f}ü");
    // Lowercase hex is read too.
    assert_eq!(unescape(b"%2f%2F").unwrap(), "//");
}

#[test]
fn bad_escapes_and_non_utf8_are_refused() {
    for bad in [&b"%"[..], b"a%", b"a%4", b"%G0", b"%0g", b"%%"] {
        assert_eq!(unescape(bad), Err(FieldError::Escape), "{bad:?}");
    }
    for bad in [&b"%FF"[..], b"\xff", b"%C3", b"a\xc3(b"] {
        assert_eq!(unescape(bad), Err(FieldError::NotUtf8), "{bad:?}");
    }
}

fn with_body(body: &str) -> Parsed {
    decode(format!("scootbg-state 1\n{body}").as_bytes())
}

/// Each bad line is skipped with its own warning, and the good ones count.
#[test]
fn malformed_lines_are_skipped_and_the_rest_still_counts() {
    let bad = [
        "all",
        "all colour #000000",
        "all color",
        "all color #00000",
        "all color #00000g",
        "all color  #000000",
        "all color #000000 extra",
        "all image /a.jpg fill #000000",
        "all image relative.jpg fill #000000 lanczos3",
        "all image /a.jpg stretchy #000000 lanczos3",
        "all image /a.jpg fill #000000 bicubic",
        "all image /a.jpg fill black lanczos3",
        "all image /a%zz.jpg fill #000000 lanczos3",
        "all image /a%FF.jpg fill #000000 lanczos3",
        "all image %2Fa.jpg%20 fill #000000 lanczos3 extra",
        "all clear extra",
        "output",
        "output DP-1",
        "output  clear",
        "output DP-1 clear extra",
        "output DP%FF clear",
        "output DP%1 clear",
        "profile",
        "profile a b",
        "fingerprint",
        "fingerprint a b",
        "unknown key",
        "ALL clear",
        " all clear",
        "all clear\r",
        "# a comment",
    ];
    let body = format!("{}\noutput DP-2 color #00ff00\n", bad.join("\n"));
    let parsed = with_body(&body);
    assert_eq!(
        parsed.warnings.len(),
        bad.len(),
        "one warning per bad line: {:#?}",
        parsed.warnings
    );
    for (warning, line) in parsed.warnings.iter().zip(2..) {
        assert!(
            warning.starts_with(&format!("line {line}: ")) && warning.ends_with("skipped"),
            "{warning}"
        );
    }
    assert_eq!(parsed.record.all, None);
    assert_eq!(
        parsed.record.named,
        vec![("DP-2".to_owned(), Pick::Color(Color { r: 0, g: 255, b: 0 }))]
    );
    assert!(!parsed.newer);
}

#[test]
fn a_raw_non_utf8_path_is_skipped_with_a_warning() {
    let mut bytes = b"scootbg-state 1\nall image /caf\xe9.jpg fill #000000 lanczos3\n".to_vec();
    bytes.extend_from_slice(b"output A clear\n");
    let parsed = decode(&bytes);
    assert_eq!(parsed.warnings.len(), 1, "{:?}", parsed.warnings);
    assert!(
        parsed.warnings[0].contains("not UTF-8"),
        "{:?}",
        parsed.warnings
    );
    assert_eq!(parsed.record.all, None);
    assert_eq!(parsed.record.named.len(), 1);
}

#[test]
fn headers_other_than_versions_1_and_2_restore_nothing() {
    for header in [
        "",
        "scootbg-state",
        "scootbg-state ",
        "scootbg-state 0",
        "scootbg-state +1",
        "scootbg-state 1 ",
        "scootbg-state  1",
        "scootbg-state x",
        "scootbg-state 99999999999999999999",
        "swww 1",
        "all clear",
    ] {
        let parsed = decode(format!("{header}\nall clear\n").as_bytes());
        assert_eq!(parsed.record, Record::default(), "{header:?}");
        assert_eq!(
            parsed.warnings.len(),
            1,
            "{header:?}: {:?}",
            parsed.warnings
        );
        assert!(
            !parsed.newer,
            "{header:?} is not a newer version: overwritable"
        );
    }
    assert_eq!(decode(b"").record, Record::default());
}

/// A newer scootbg's file is left alone: nothing read, and marked so the
/// daemon never writes over it.
#[test]
fn a_newer_version_is_neither_read_nor_to_be_written() {
    let parsed = decode(b"scootbg-state 3\nall color #ffffff\nsomething new\n");
    assert!(parsed.newer);
    assert_eq!(parsed.record, Record::default());
    assert_eq!(parsed.warnings.len(), 1);
    assert!(
        parsed.warnings[0].contains("version 3"),
        "{:?}",
        parsed.warnings
    );
}

#[test]
fn an_over_long_file_is_not_read() {
    let mut bytes = b"scootbg-state 1\nall color #ffffff\n".to_vec();
    bytes.resize(MAX_BYTES + 1, b'\n');
    let parsed = decode(&bytes);
    assert_eq!(parsed.record, Record::default());
    assert_eq!(parsed.warnings.len(), 1);
    assert!(!parsed.newer);
    // At the limit exactly it is read (blank lines are skipped).
    bytes.truncate(MAX_BYTES);
    let record = clean(&bytes);
    assert_eq!(
        record.all,
        Some(Pick::Color(Color {
            r: 255,
            g: 255,
            b: 255
        }))
    );
}

#[test]
fn duplicates_warn_and_the_later_line_counts() {
    let parsed = with_body(
        "all color #000001\nall color #000002\noutput A color #000003\n\
         output B clear\noutput A clear\nprofile x\nprofile y\n\
         fingerprint f\nfingerprint g\n",
    );
    assert_eq!(parsed.warnings.len(), 4, "{:?}", parsed.warnings);
    for warning in &parsed.warnings {
        assert!(warning.contains("the later one counts"), "{warning}");
    }
    let record = parsed.record;
    assert_eq!(record.all, Some(Pick::Color(Color { r: 0, g: 0, b: 2 })));
    assert_eq!(
        record.named,
        vec![("A".to_owned(), Pick::Clear), ("B".to_owned(), Pick::Clear)]
    );
    assert_eq!(record.profile.as_deref(), Some("y"));
    assert_eq!(record.fingerprint.as_deref(), Some("g"));
}

/// Lines are oldest first: past the limit the reader keeps the last
/// (newest) ones, as the writer would.
#[test]
fn outputs_beyond_the_limit_keep_the_newest() {
    let body: String = (0..MAX_OUTPUTS + 3)
        .map(|i| format!("output O-{i} clear\n"))
        .collect();
    let parsed = with_body(&body);
    assert_eq!(parsed.record.named.len(), MAX_OUTPUTS);
    assert_eq!(parsed.warnings.len(), 3, "{:?}", parsed.warnings);
    assert_eq!(parsed.record.named[0].0, "O-3", "the three oldest went");
    assert_eq!(
        parsed.record.named[MAX_OUTPUTS - 1].0,
        format!("O-{}", MAX_OUTPUTS + 2)
    );
    // A name already read is replaced in place, at the limit or not.
    let body = format!("{body}output O-10 color #ffffff\n");
    let parsed = with_body(&body);
    assert_eq!(parsed.record.named.len(), MAX_OUTPUTS);
    assert_eq!(
        parsed.record.named[7],
        (
            "O-10".to_owned(),
            Pick::Color(Color {
                r: 255,
                g: 255,
                b: 255
            })
        )
    );
}

fn encoded(all: Option<&Choice>, named: &[(String, Choice)]) -> (String, super::Left) {
    let mut out = String::new();
    let list: Vec<(&str, &Choice)> = named.iter().map(|(n, c)| (n.as_str(), c)).collect();
    let left = encode(&mut out, "default", Some("f"), all, &list);
    (out, left)
}

/// The writer stays within what the reader takes: at most `MAX_OUTPUTS`
/// lines, the newest (last given) kept, in the order given.
#[test]
fn the_writer_keeps_the_newest_within_the_line_limit() {
    let named: Vec<(String, Choice)> = (0..MAX_OUTPUTS + 5)
        .map(|i| (format!("O-{i}"), color(1, 2, 3)))
        .collect();
    let (text, left) = encoded(Some(&None), &named);
    assert_eq!(
        left,
        super::Left {
            named: 5,
            all: false
        }
    );
    let record = clean(text.as_bytes());
    assert_eq!(record.named.len(), MAX_OUTPUTS);
    assert_eq!(record.named[0].0, "O-5");
    assert_eq!(
        record.named.last().unwrap().0,
        format!("O-{}", MAX_OUTPUTS + 4)
    );
    assert_eq!(record.all, Some(Pick::Clear));
    // At the limit exactly, nothing is left out.
    let (_, left) = encoded(None, &named[..MAX_OUTPUTS]);
    assert_eq!(left, super::Left::default());
}

/// Long paths: whole lines, newest first, while they fit in `MAX_BYTES`;
/// the file is then read back whole, with no warning.
#[test]
fn the_writer_keeps_the_newest_within_the_byte_limit() {
    // 20 KiB paths of spaces: 60 KiB escaped, so four fit beside `all`.
    let long = |i: usize| format!("/{i}{}", " ".repeat(20 * 1024));
    let named: Vec<(String, Choice)> = (0..10)
        .map(|i| (format!("O-{i}"), image(&long(i))))
        .collect();
    let (text, left) = encoded(Some(&image(&long(99))), &named);
    assert!(text.len() <= MAX_BYTES, "{}", text.len());
    assert!(left.named > 0 && !left.all, "{left:?}");
    let record = clean(text.as_bytes());
    let kept: Vec<&str> = record.named.iter().map(|(n, _)| n.as_str()).collect();
    let want: Vec<String> = (left.named..10).map(|i| format!("O-{i}")).collect();
    assert_eq!(kept, want);
    assert_eq!(record.all, Some(pick_image(&long(99))));
    // One line longer than any file: it alone is left out.
    let huge = format!("/{}", "\n".repeat(MAX_BYTES / 2));
    let (text, left) = encoded(Some(&image(&huge)), &named[9..]);
    assert_eq!(
        left,
        super::Left {
            named: 0,
            all: true
        }
    );
    let record = clean(text.as_bytes());
    assert_eq!((record.all, record.named.len()), (None, 1));
    let (text, left) = encoded(None, &[("BIG".to_owned(), image(&huge)), named[9].clone()]);
    assert_eq!(
        left,
        super::Left {
            named: 1,
            all: false
        },
        "the older, huge one"
    );
    assert_eq!(clean(text.as_bytes()).named[0].0, "O-9");
}

/// The longest path a request can carry (the 64 KiB request line), every
/// byte escaped, still fits as the `all` line.
#[test]
fn the_longest_request_path_fits() {
    let path = format!("/{}", "\u{1}".repeat(crate::protocol::MAX_REQUEST_LINE));
    let (text, left) = encoded(Some(&image(&path)), &[]);
    assert_eq!(left, super::Left::default());
    assert_eq!(clean(text.as_bytes()).all, Some(pick_image(&path)));
}

#[test]
fn an_over_long_fingerprint_is_skipped() {
    let long = "f".repeat(MAX_FINGERPRINT + 1);
    let parsed = with_body(&format!("fingerprint {long}\n"));
    assert_eq!(parsed.record.fingerprint, None);
    assert_eq!(parsed.warnings.len(), 1);
    let fits = "f".repeat(MAX_FINGERPRINT);
    assert_eq!(
        clean(format!("scootbg-state 1\nfingerprint {fits}\n").as_bytes()).fingerprint,
        Some(fits)
    );
}

/// No trailing newline, blank lines between entries: both fine.
#[test]
fn blank_lines_and_a_missing_final_newline_are_fine() {
    let record = clean(b"scootbg-state 1\n\n\nall clear\n\noutput A color #aBcDeF");
    assert_eq!(record.all, Some(Pick::Clear));
    assert_eq!(
        record.named,
        vec![(
            "A".to_owned(),
            Pick::Color(Color {
                r: 0xab,
                g: 0xcd,
                b: 0xef
            })
        )]
    );
}

/// An empty output name cannot be written, so it is left out rather than
/// written as a line no reader takes.
#[test]
fn an_empty_output_name_is_not_written() {
    let written = text(None, None, &[("", color(1, 1, 1)), ("A", None)]);
    assert!(!written.contains("output  "), "{written}");
    let record = clean(written.as_bytes());
    assert_eq!(record.named, vec![("A".to_owned(), Pick::Clear)]);
}

/// Arbitrary bytes never panic, whatever the header.
#[test]
fn arbitrary_bytes_never_panic() {
    let mut seed: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let alphabet = b"scootbg-state 1\n%allouputimagecolorclear#0123456789abcdefFG/ \t\r\xff\xc3";
    for _ in 0..2000 {
        let len = (next() % 200) as usize;
        let mut bytes = b"scootbg-state 1\n".to_vec();
        for _ in 0..len {
            let index = (next() % alphabet.len() as u64) as usize;
            bytes.push(alphabet[index]);
        }
        let parsed = decode(&bytes);
        assert!(parsed.record.named.len() <= MAX_OUTPUTS);
        let raw: Vec<u8> = bytes[16..].to_vec();
        let _ = decode(&raw);
    }
}

/// A downloaded image round-trips with its URL and its pin; a version-1
/// file still reads, as a file choice.
#[test]
fn downloads_round_trip_and_version_1_still_reads() {
    use crate::fetch::Fetch;
    let sha = crate::sha256::digest(b"test");
    let sha_hex = crate::sha256::hex(b"test");
    let downloaded = || {
        Some(Wallpaper::Image(Arc::new(Image {
            path: "/cache/9f86d081".to_owned(),
            look: LOOK,
            serial: 1,
            fetch: Some(Fetch {
                url: "https://example.com/a b.png".to_owned(),
                sha256: Some(sha),
            }),
        })))
    };
    let written = text(None, Some(&downloaded()), &[]);
    assert_eq!(
        written,
        format!(
            "scootbg-state 2\nprofile default\nall image /cache/9f86d081 fit #101014 \
             catmull-rom url https://example.com/a%20b.png sha256 {sha_hex}\n"
        )
    );
    let record = clean(written.as_bytes());
    assert_eq!(
        record.all,
        Some(Pick::Image {
            path: "/cache/9f86d081".to_owned(),
            look: LOOK,
            fetch: Some(Fetch {
                url: "https://example.com/a b.png".to_owned(),
                sha256: Some(sha),
            }),
        })
    );
    // Without a pin, no `sha256` is written.
    let unpinned = Some(Wallpaper::Image(Arc::new(Image {
        path: "/cache/aa".to_owned(),
        look: LOOK,
        serial: 1,
        fetch: Some(Fetch {
            url: "http://127.0.0.1:1/a.png".to_owned(),
            sha256: None,
        }),
    })));
    let written = text(None, Some(&unpinned), &[]);
    assert!(
        written.ends_with("url http://127.0.0.1:1/a.png\n"),
        "{written}"
    );
    // Version 1 has no trailer: its lines read as file choices.
    let record = clean(b"scootbg-state 1\nall image /a.png fit #101014 catmull-rom\n");
    assert_eq!(record.all, Some(pick_image("/a.png")));
}

/// A bad trailer is one skipped line, not a lost file.
#[test]
fn bad_trailers_are_skipped() {
    for line in [
        // Not a URL.
        "all image /a.png fit #101014 catmull-rom url /a.png",
        // Not hex.
        "all image /a.png fit #101014 catmull-rom url https://example.com/a.png sha256 zz",
        // A pin without a URL.
        "all image /a.png fit #101014 catmull-rom sha256 9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08",
        // Anything else back there.
        "all image /a.png fit #101014 catmull-rom url https://example.com/a.png sha256 9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08 extra",
        "all image /a.png fit #101014 catmull-rom url",
    ] {
        let parsed = decode(format!("scootbg-state 2\n{line}\n").as_bytes());
        assert_eq!(parsed.record.all, None, "{line}");
        assert_eq!(parsed.warnings.len(), 1, "{line}: {:?}", parsed.warnings);
        assert!(!parsed.newer);
    }
}
