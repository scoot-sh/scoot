use serde_json::json;

use super::*;
#[cfg(any(feature = "push", feature = "exec"))]
use crate::modules::View;

fn line(format: Format, text: &str) -> Result<Shown, Invalid> {
    let mut shown = Shown::text("before");
    parse_line(format, text.as_bytes(), &mut shown).map(|()| shown)
}

#[test]
fn a_text_line_is_the_text() {
    let shown = line(Format::Text, "72% sunny").unwrap();
    assert_eq!(shown.text, "72% sunny");
    assert_eq!(shown.class, Class::Normal);
    assert_eq!(shown.tooltip, "");
}

#[test]
fn text_is_trimmed_and_every_control_character_becomes_a_space() {
    assert_eq!(
        line(Format::Text, "  a\tb\x1b[31mc\r").unwrap().text,
        "a b [31mc"
    );
    assert_eq!(line(Format::Text, "\r").unwrap().text, "");
    assert_eq!(line(Format::Text, "").unwrap().text, "");
    assert_eq!(line(Format::Text, "\u{7}\u{0}x").unwrap().text, "x");
}

#[test]
fn text_that_is_not_utf8_shows_its_valid_parts() {
    let mut shown = Shown::default();
    parse_line(Format::Text, b"ok \xff\xfe done", &mut shown).unwrap();
    assert!(shown.text.starts_with("ok "), "{:?}", shown.text);
    assert!(shown.text.ends_with(" done"));
}

#[test]
fn text_is_cut_at_the_bound_on_a_character_boundary() {
    let long = "é".repeat(MAX_TEXT);
    let shown = line(Format::Text, &long).unwrap();
    assert!(shown.text.len() <= MAX_TEXT);
    assert_eq!(shown.text.len() % 2, 0, "cut inside a character");
    assert!(shown.text.chars().all(|c| c == 'é'));
    // At exactly the bound it is kept whole.
    let exact = "x".repeat(MAX_TEXT);
    assert_eq!(line(Format::Text, &exact).unwrap().text, exact);
}

#[test]
fn a_line_past_the_payload_bound_is_refused_and_changes_nothing() {
    let mut shown = Shown::text("kept");
    let long = vec![b'x'; MAX_PAYLOAD + 1];
    assert_eq!(
        parse_line(Format::Text, &long, &mut shown),
        Err(Invalid::TooLong)
    );
    assert_eq!(
        parse_line(Format::Json, &long, &mut shown),
        Err(Invalid::TooLong)
    );
    assert_eq!(shown.text, "kept");
    // At the bound itself, accepted.
    let exact = vec![b'x'; MAX_PAYLOAD];
    assert!(parse_line(Format::Text, &exact, &mut shown).is_ok());
}

#[test]
fn a_json_line_sets_all_three_keys() {
    let shown = line(
        Format::Json,
        r#"{"version":1,"text":"72%","class":"warn","tooltip":"battery low"}"#,
    )
    .unwrap();
    assert_eq!(
        shown,
        Shown {
            text: "72%".into(),
            class: Class::Warn,
            tooltip: "battery low".into(),
        }
    );
}

#[test]
fn every_key_is_optional_and_unknown_ones_are_ignored() {
    assert_eq!(line(Format::Json, "{}").unwrap(), Shown::default());
    let shown = line(Format::Json, r#"{"text":"a","percentage":50,"alt":["x"]}"#).unwrap();
    assert_eq!(shown.text, "a");
    // A line replaces what was shown, it does not merge into it.
    let mut shown = Shown {
        text: "old".into(),
        class: Class::Urgent,
        tooltip: "tip".into(),
    };
    parse_line(Format::Json, br#"{"text":"new"}"#, &mut shown).unwrap();
    assert_eq!(
        shown,
        Shown {
            text: "new".into(),
            ..Shown::default()
        }
    );
}

#[test]
fn every_class_is_read_and_a_bad_one_is_named() {
    for (name, class) in [
        ("normal", Class::Normal),
        ("warn", Class::Warn),
        ("urgent", Class::Urgent),
        ("muted", Class::Muted),
    ] {
        let shown = line(Format::Json, &format!(r#"{{"class":"{name}"}}"#)).unwrap();
        assert_eq!(shown.class, class);
    }
    assert_eq!(
        line(Format::Json, r#"{"class":"loud"}"#),
        Err(Invalid::Class("loud".into()))
    );
    assert_eq!(
        line(Format::Json, r#"{"class":["warn"]}"#),
        Err(Invalid::Field("class", "a string"))
    );
}

#[test]
fn a_bad_json_line_is_refused_by_name_and_changes_nothing() {
    for (bad, want) in [
        ("not json", "not valid JSON"),
        ("{", "not valid JSON"),
        (r#"{"text":}"#, "not valid JSON"),
        ("[1]", "JSON object"),
        ("3", "JSON object"),
        ("null", "JSON object"),
        (r#""just a string""#, "JSON object"),
        (r#"{"text":5}"#, "`text` takes a string"),
        (r#"{"tooltip":false}"#, "`tooltip` takes a string"),
        (r#"{"version":2}"#, "`version` 2"),
        (r#"{"version":"1"}"#, "`version`"),
        (r#"{"version":-1}"#, "`version`"),
        (r#"{"version":1.5}"#, "`version`"),
    ] {
        let mut shown = Shown::text("kept");
        let error = parse_line(Format::Json, bad.as_bytes(), &mut shown).unwrap_err();
        assert!(error.to_string().contains(want), "{bad}: {error}");
        assert_eq!(shown.text, "kept", "{bad}");
    }
}

#[test]
fn json_nested_past_the_bound_is_refused_before_it_is_built() {
    let deep = format!("{}{}", "[".repeat(MAX_DEPTH + 1), "]".repeat(MAX_DEPTH + 1));
    let error = line(Format::Json, &deep).unwrap_err();
    assert!(error.to_string().contains("nested"), "{error}");
    // Brackets inside a string are not nesting.
    let shown = line(
        Format::Json,
        &format!(r#"{{"text":"{}"}}"#, "[".repeat(200)),
    )
    .unwrap();
    assert_eq!(shown.text.len(), 200);
    // Escaped quotes do not end the string early.
    let shown = line(Format::Json, r#"{"text":"a \" [[[[[[[[[[[[ b"}"#).unwrap();
    assert!(shown.text.contains("[[[["));
    // A hostile line of 4096 opening brackets does not recurse.
    let bomb = "{\"a\":".repeat(MAX_PAYLOAD / 5);
    assert!(line(Format::Json, &bomb).is_err());
}

#[test]
fn a_set_value_is_a_string_an_object_or_null() {
    let mut shown = Shown::text("old");
    from_value(&json!("sunny"), &mut shown).unwrap();
    assert_eq!(shown, Shown::text("sunny"));
    from_value(&json!({"text": "t", "class": "muted"}), &mut shown).unwrap();
    assert_eq!((shown.text.as_str(), shown.class), ("t", Class::Muted));
    from_value(&json!(null), &mut shown).unwrap();
    assert_eq!(shown, Shown::default());
    for bad in [json!(1), json!(true), json!([1]), json!(1.5)] {
        let mut shown = Shown::text("kept");
        assert_eq!(from_value(&bad, &mut shown), Err(Invalid::NotAnObject));
        assert_eq!(shown.text, "kept");
    }
    let mut shown = Shown::text("kept");
    let long = json!("x".repeat(MAX_PAYLOAD + 1));
    assert_eq!(from_value(&long, &mut shown), Err(Invalid::TooLong));
    assert_eq!(shown.text, "kept");
}

#[test]
fn a_set_value_gets_the_same_sanitizing() {
    let mut shown = Shown::default();
    from_value(&json!({"text": "a\nb", "tooltip": "\u{1b}x"}), &mut shown).unwrap();
    assert_eq!(shown.text, "a b");
    assert_eq!(shown.tooltip, "x");
}

#[cfg(any(feature = "push", feature = "exec"))]
#[test]
fn shown_writes_its_view_and_nothing_cut() {
    let shown = Shown {
        text: "72%".into(),
        class: Class::Urgent,
        tooltip: "tip".into(),
    };
    let mut view = View::default();
    shown.write(&mut view);
    assert_eq!(
        (view.text(), view.tooltip(), view.class()),
        ("72%", "tip", Class::Urgent)
    );
    assert!(!view.was_cut());
    // The widest text a payload can hold fits the view's bound exactly.
    let mut view = View::default();
    Shown::text(&"x".repeat(MAX_PAYLOAD)).write(&mut view);
    assert_eq!(view.text().len(), MAX_TEXT);
    assert!(!view.was_cut());
    // An empty one shows nothing, so the module takes no space.
    let mut view = View::default();
    Shown::default().write(&mut view);
    assert!(view.is_empty());
}

#[test]
fn the_format_is_named_in_the_config() {
    assert_eq!(Format::parse("text"), Some(Format::Text));
    assert_eq!(Format::parse("json"), Some(Format::Json));
    assert_eq!(Format::parse("JSON"), None);
    assert_eq!(Format::parse(""), None);
    assert_eq!(Format::default(), Format::Text);
}

#[test]
fn a_hostile_set_never_panics() {
    // Every byte string of a few lengths through both parsers: any answer
    // but a panic.
    let mut shown = Shown::default();
    for len in 0..4usize {
        let mut bytes = vec![0u8; len];
        for seed in 0..2000u32 {
            for (i, b) in bytes.iter_mut().enumerate() {
                *b = (seed.wrapping_mul(2654435761).rotate_left(i as u32 * 8) & 0xff) as u8;
            }
            let _ = parse_line(Format::Json, &bytes, &mut shown);
            let _ = parse_line(Format::Text, &bytes, &mut shown);
        }
    }
}

#[test]
fn a_warm_text_line_allocates_nothing() {
    // An exec module's text updates are an event path: once the strings
    // have their capacity, a line costs no allocation.
    let mut shown = Shown::text("warm up so the strings have room");
    let _ = parse_line(Format::Text, b"a line of text", &mut shown);
    let ((), allocations) = scootbg_mem::count_allocations(|| {
        for i in 0..1000u32 {
            let line: &[u8] = if i % 2 == 0 {
                b"72% sunny \t"
            } else {
                b"a shorter one"
            };
            let _ = parse_line(Format::Text, line, &mut shown);
        }
    });
    assert_eq!(allocations, 0);
}
