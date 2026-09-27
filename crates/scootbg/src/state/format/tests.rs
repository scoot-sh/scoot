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
        named.iter().map(|(name, choice)| (*name, choice)),
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
        "scootbg-state 1\n\
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
    assert_eq!(empty, "scootbg-state 1\nprofile default\n");
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
fn headers_other_than_version_1_restore_nothing() {
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
    let parsed = decode(b"scootbg-state 2\nall color #ffffff\nsomething new\n");
    assert!(parsed.newer);
    assert_eq!(parsed.record, Record::default());
    assert_eq!(parsed.warnings.len(), 1);
    assert!(
        parsed.warnings[0].contains("version 2"),
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

#[test]
fn outputs_beyond_the_limit_are_skipped() {
    let body: String = (0..MAX_OUTPUTS + 3)
        .map(|i| format!("output O-{i} clear\n"))
        .collect();
    let parsed = with_body(&body);
    assert_eq!(parsed.record.named.len(), MAX_OUTPUTS);
    assert_eq!(parsed.warnings.len(), 3, "{:?}", parsed.warnings);
    // A name already read is still replaced at the limit.
    let body = format!("{body}output O-0 color #ffffff\n");
    let parsed = with_body(&body);
    assert_eq!(
        parsed.record.named[0].1,
        Pick::Color(Color {
            r: 255,
            g: 255,
            b: 255
        })
    );
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
