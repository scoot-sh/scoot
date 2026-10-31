//! The launcher config's tests: defaults, bounds, refusals.

use super::*;

#[test]
fn an_empty_file_is_the_defaults() {
    assert_eq!(
        parse(b"").expect("empty parses"),
        Config {
            font: None,
            width_em: 40,
            rows: 10,
            effects: toml::Table::new(),
        }
    );
}

#[test]
fn every_key_parses() {
    let config = parse(b"font = \"/usr/share/fonts/DejaVuSans.ttf\"\nwidth_em = 50\nrows = 5\n")
        .expect("parses");
    assert_eq!(
        config.font.as_deref(),
        Some("/usr/share/fonts/DejaVuSans.ttf")
    );
    assert_eq!(config.width_em, 50);
    assert_eq!(config.rows, 5);
}

#[test]
fn an_unknown_key_is_refused() {
    let error = parse(b"colour = \"red\"\n").expect_err("unknown key refuses");
    assert!(error.contains("colour"), "{error}");
}

#[test]
fn out_of_range_values_are_refused_with_the_key_named() {
    for (body, key) in [
        ("width_em = 9\n", "width_em"),
        ("width_em = 201\n", "width_em"),
        ("rows = 0\n", "rows"),
        ("rows = 65\n", "rows"),
        ("font = \"\"\n", "font"),
    ] {
        let error = parse(body.as_bytes()).expect_err("refuses");
        assert!(error.contains(key), "{body}: {error}");
    }
}

#[test]
fn a_control_character_in_font_is_refused() {
    let error = parse(b"font = \"/tmp/a\tb.ttf\"\n").expect_err("refuses");
    assert!(error.contains("font"), "{error}");
}

#[test]
fn edges_hold() {
    assert_eq!(parse(b"width_em = 10\n").expect("low").width_em, 10);
    assert_eq!(parse(b"width_em = 200\n").expect("high").width_em, 200);
    assert_eq!(parse(b"rows = 1\n").expect("low").rows, 1);
    assert_eq!(parse(b"rows = 64\n").expect("high").rows, 64);
}

#[test]
fn hostile_input_is_refused_not_panicked() {
    assert!(parse(b"\xff\xfe=1\n").is_err());
    assert!(parse(&vec![b'x'; 65 * 1024]).is_err());
    assert!(parse(b"[font]\n").is_err());
}
